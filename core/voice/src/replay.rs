//! Replays recorded speech through the path a dictation takes: the microphone's buffer, pieces cut at pauses while
//! you are still talking, the rest at release, then the dictionary. A benchmark built on this measures the wait users
//! feel, not only the model. The key goes down when the clip starts and comes up when it ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::audio::{Recording, RATE};
use crate::core::TAIL;
use crate::pieces::{self, Cutter, Pieces, POLL};

/// A speech model as the replay sees it: 16 kHz audio in, text out.
pub type Model = Box<dyn FnMut(&[f32]) -> anyhow::Result<String> + Send>;

#[derive(Debug, Clone, Default)]
pub struct Dictation {
    /// The model's text, pieces joined as the app joins them.
    pub raw: String,
    /// What the app would type: `raw` after `finish` (the dictionary).
    pub text: String,
    /// Key down to the first text available: the first piece, or the final text if there was none.
    pub first: Duration,
    /// Key up to the final text, the wait users feel. Includes the tail recorded after release.
    pub after_release: Duration,
    /// Time spent inside the model.
    pub compute: Duration,
    /// Pieces cut before release.
    pub pieces: usize,
}

fn timed(
    model: &Arc<Mutex<Model>>,
    compute: &Arc<Mutex<(Duration, usize)>>,
) -> impl FnMut(&[f32]) -> anyhow::Result<String> + Send + 'static {
    let (model, compute) = (model.clone(), compute.clone());
    move |audio| {
        let mut m = model.lock();
        let t = Instant::now();
        let text = m(audio);
        let mut c = compute.lock();
        c.0 += t.elapsed();
        c.1 += 1;
        text
    }
}

/// Replay in real time on the app's own threads: the clip is fed as a microphone would feed it, the pieces worker
/// polls it, and release waits the same tail. Takes as long as the clip.
pub fn live(
    audio: Vec<f32>,
    model: &Arc<Mutex<Model>>,
    finish: impl FnOnce(&str) -> String,
) -> anyhow::Result<Dictation> {
    let seconds = Duration::from_secs_f64(audio.len() as f64 / RATE as f64);
    let compute = Arc::new(Mutex::new((Duration::ZERO, 0)));
    let mut rec = Recording::replay(audio, RATE);
    let pressed = loop {
        if let Some(t) = rec.first_sample() {
            break t;
        }
        std::thread::yield_now();
    };
    let (raw, rate) = rec.tap();
    let pieces = Pieces::start(raw, rate, timed(model, &compute));
    std::thread::sleep((pressed + seconds).saturating_duration_since(Instant::now()));
    let released = Instant::now();
    std::thread::sleep(TAIL);
    rec.stop();
    let done = pieces.finish();
    let first = done.first;
    let made = compute.lock().1;
    let rest = rec.finish_from(done.raw_samples);
    let raw = pieces::transcript(done, &rest, timed(model, &compute))?;
    let text = finish(&raw);
    let end = Instant::now();
    let compute = compute.lock().0;
    Ok(Dictation {
        text,
        raw,
        first: first.unwrap_or(end).saturating_duration_since(pressed),
        after_release: end.saturating_duration_since(released),
        compute,
        pieces: made,
    })
}

/// The same decisions on a simulated clock, for quick runs: the model's time is real, the waiting for speech is not.
/// The worker looks every `POLL` of clip time and a piece holds it for as long as the model took, so pieces are cut
/// where the live worker would cut them on an idle machine.
pub fn simulated(
    audio: &[f32],
    model: &mut Model,
    finish: impl FnOnce(&str) -> String,
) -> anyhow::Result<Dictation> {
    let rate = RATE as f64;
    let key_up = audio.len() as f64 / rate;
    let stop_at = key_up + TAIL.as_secs_f64();
    // What the microphone holds when it stops: the clip, then the quiet room during the tail.
    let mut buffer = audio.to_vec();
    buffer.resize(
        audio.len() + (TAIL.as_secs_f64() * rate).round() as usize,
        0.0,
    );
    let mut cutter = Cutter::new(RATE);
    let (mut now, mut busy_until) = (0.0f64, 0.0f64);
    let (mut compute, mut made, mut first) = (Duration::ZERO, 0, None);
    loop {
        now += POLL.as_secs_f64();
        if now >= stop_at {
            break;
        }
        let len = ((now * rate) as usize).min(buffer.len());
        let Some(from) = cutter.look_from(len) else {
            continue;
        };
        let Some((piece, covers)) = cutter.piece(&buffer[from..len]) else {
            continue;
        };
        let t = Instant::now();
        let text = model(&piece);
        let took = t.elapsed();
        compute += took;
        made += 1;
        match text {
            Ok(text) => {
                now += took.as_secs_f64();
                busy_until = now;
                if !text.trim().is_empty() {
                    first.get_or_insert(now);
                }
                cutter.add(&text, covers);
            }
            Err(e) => {
                log::warn!("transcribing a piece failed: {e}");
                now += took.as_secs_f64();
                busy_until = now;
                break;
            }
        }
    }
    // Release stops the worker at once, unless it is partway through a piece.
    let worker_done = busy_until.max(stop_at);
    let done = std::mem::take(&mut cutter.done);
    let t = Instant::now();
    let rest = &buffer[done.raw_samples.min(buffer.len())..];
    let mut rest_compute = Duration::ZERO;
    let raw = pieces::transcript(done, rest, |a| {
        let t = Instant::now();
        let text = model(a);
        rest_compute = t.elapsed();
        text
    })?;
    let text = finish(&raw);
    let after_release = Duration::from_secs_f64(worker_done - key_up) + t.elapsed();
    let final_at = Duration::from_secs_f64(key_up) + after_release;
    Ok(Dictation {
        text,
        raw,
        first: first.map_or(final_at, Duration::from_secs_f64),
        after_release,
        compute: compute + rest_compute,
        pieces: made,
    })
}

/// CPU time this process has used and its peak resident memory.
pub struct Usage {
    pub cpu: Duration,
    pub peak_rss_mb: f64,
}

#[cfg(target_os = "linux")]
pub fn usage() -> Option<Usage> {
    // SAFETY: getrusage only writes the struct it is given.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return None;
    }
    let tv = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec as u64) + Duration::from_micros(t.tv_usec as u64)
    };
    Some(Usage {
        cpu: tv(ru.ru_utime) + tv(ru.ru_stime),
        peak_rss_mb: ru.ru_maxrss as f64 / 1024.0,
    })
}

#[cfg(not(target_os = "linux"))]
pub fn usage() -> Option<Usage> {
    None
}

/// The package energy counter in joules (Intel RAPL), where it can be read without root. It wraps, so only short
/// differences mean anything.
pub fn energy_joules() -> Option<f64> {
    let uj = std::fs::read_to_string("/sys/class/powercap/intel-rapl:0/energy_uj").ok()?;
    Some(uj.trim().parse::<f64>().ok()? / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| 0.2 * (i as f32 * 0.05).sin())
            .collect()
    }

    fn pause(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| 0.0005 * (i as f32 * 0.7).sin())
            .collect()
    }

    fn counting() -> Model {
        Box::new(|audio: &[f32]| Ok(format!("{} samples.", audio.len())))
    }

    #[test]
    fn a_short_clip_is_one_transcription_after_release() {
        let audio = speech(2.0);
        let d = simulated(&audio, &mut counting(), |t| t.to_uppercase()).unwrap();
        assert_eq!(d.pieces, 0);
        // The rest includes the tail of quiet room.
        let tail = (TAIL.as_secs_f64() * RATE as f64) as usize;
        assert_eq!(d.raw, format!("{} samples.", audio.len() + tail));
        assert_eq!(d.text, d.raw.to_uppercase());
        assert!(
            d.after_release >= TAIL && d.after_release < TAIL + Duration::from_millis(100),
            "{:?}",
            d.after_release
        );
        assert!(d.first >= Duration::from_secs(2));
    }

    #[test]
    fn a_long_clip_is_cut_at_its_pause_before_release() {
        let audio = [speech(7.0), pause(0.6), speech(3.0)].concat();
        let d = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        assert_eq!(d.pieces, 1, "{}", d.raw);
        // The first piece was ready while the speaker was still talking.
        assert!(d.first < Duration::from_secs(8), "{:?}", d.first);
        let words: Vec<usize> = d
            .raw
            .split(" samples.")
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        let tail = (TAIL.as_secs_f64() * RATE as f64) as usize;
        assert_eq!(words.iter().sum::<usize>(), audio.len() + tail);
    }

    #[test]
    fn live_and_simulated_cut_the_same_way() {
        let audio = [speech(7.0), pause(0.6), speech(1.0)].concat();
        let sim = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        let model = Arc::new(Mutex::new(counting()));
        let live = live(audio, &model, |t| t.to_string()).unwrap();
        assert_eq!(live.pieces, sim.pieces);
        assert_eq!(live.pieces, 1);
        // The live feed lands in 10 ms blocks, so its cut can differ by a block.
        let len = |d: &Dictation| {
            d.raw
                .split(" samples.")
                .next()
                .unwrap()
                .trim()
                .parse::<i64>()
                .unwrap()
        };
        assert!(
            (len(&live) - len(&sim)).abs() <= 160 * 60,
            "{} vs {}",
            live.raw,
            sim.raw
        );
        assert!(live.after_release >= TAIL);
    }

    #[test]
    fn usage_reads_this_process() {
        if let Some(u) = usage() {
            assert!(u.peak_rss_mb > 1.0);
        }
    }
}
