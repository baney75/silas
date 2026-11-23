//! Replays recorded speech through the path a dictation takes: the microphone's buffer, pieces cut at pauses while
//! you are still talking, guesses made in pauses, the rest at release, then the dictionary. A benchmark built on this
//! measures the wait users feel, not only the model. The key goes down when the clip starts and comes up when it ends.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use crate::audio::{Recording, RATE};
use crate::core::{tail_for, Ending};
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
    /// Guesses transcribed in pauses before release.
    pub guesses: usize,
    /// Whether release took the last guess as the text, with nothing left to transcribe.
    pub guess_used: bool,
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
/// polls it, and release waits the same tail the app would. Takes as long as the clip.
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
    let pieces = Pieces::start(raw.clone(), rate, timed(model, &compute));
    std::thread::sleep((pressed + seconds).saturating_duration_since(Instant::now()));
    let released = Instant::now();
    pieces.key_up();
    let quiet = pieces::ends_quiet(&raw.lock(), rate);
    std::thread::sleep(tail_for(Ending::Key, quiet));
    rec.stop();
    let done = pieces.finish();
    let (first, made, guesses) = (done.first, done.pieces, done.guesses);
    let mut rest = rec.finish_from(done.raw_samples);
    if quiet {
        pieces::pad_quiet(&mut rest);
    }
    let guess_used = pieces::guess_ends_it(&done, &rest);
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
        guesses,
        guess_used,
    })
}

/// The model's speed on the simulated clock: seconds of work per second of audio it hears. The median `rtf` of the
/// 40 `--fast` rows in `bench/rivals/opencode-chapel1/silas-fast.jsonl.gz` (0.123): Parakeet v3 on transcribe.cpp,
/// chapel1's Intel i5-10210U, load average about 1, 2026-11-18.
pub const SIM_RTF: f64 = 0.123;

/// The same decisions on a simulated clock, for quick runs: neither the waiting for speech nor the model's time is
/// real. The worker looks every `POLL` of clip time, and a piece or a guess holds it for `SIM_RTF` times the audio
/// the model hears, so they are made where the live worker would make them on an idle chapel1, and the same text
/// comes out on every machine at any load. `compute` is the model's real time. `first` is on the simulated clock.
/// `after_release` is the simulated wait for a piece or guess still running at key up (or the tail), plus the real
/// time the rest and the dictionary took.
pub fn simulated(
    audio: &[f32],
    model: &mut Model,
    finish: impl FnOnce(&str) -> String,
) -> anyhow::Result<Dictation> {
    let rate = RATE as f64;
    let key_up = audio.len() as f64 / rate;
    let quiet = pieces::ends_quiet(audio, RATE);
    let tail = tail_for(Ending::Key, quiet);
    let stop_at = key_up + tail.as_secs_f64();
    // What the microphone holds when it stops: the clip, then the quiet room during the tail.
    let mut buffer = audio.to_vec();
    buffer.resize(
        audio.len() + (tail.as_secs_f64() * rate).round() as usize,
        0.0,
    );
    let mut cutter = Cutter::new(RATE);
    let (mut now, mut busy_until) = (0.0f64, 0.0f64);
    let (mut compute, mut first) = (Duration::ZERO, None);
    loop {
        now += POLL.as_secs_f64();
        if now >= stop_at {
            break;
        }
        if now >= key_up {
            cutter.key_up();
        }
        let len = ((now * rate) as usize).min(buffer.len());
        let want = cutter.want(&buffer[..len]);
        let job = cutter.job(want);
        let going = match job {
            Some(job) => {
                let t = Instant::now();
                let text = model(job.audio());
                compute += t.elapsed();
                now += job.audio().len() as f64 / rate * SIM_RTF;
                busy_until = now;
                cutter.finished(job, text)
            }
            None => true,
        };
        // The pieces' first words, whether a piece was transcribed or a guess became one.
        if !cutter.done.text.trim().is_empty() {
            first.get_or_insert(now);
        }
        if !going {
            break;
        }
    }
    // Release stops the worker at once, unless it is partway through a piece or a guess.
    let worker_done = busy_until.max(stop_at);
    let done = std::mem::take(&mut cutter.done);
    let (made, guesses) = (done.pieces, done.guesses);
    let t = Instant::now();
    let mut rest = buffer[done.raw_samples.min(buffer.len())..].to_vec();
    if quiet {
        pieces::pad_quiet(&mut rest);
    }
    let guess_used = pieces::guess_ends_it(&done, &rest);
    let mut rest_compute = Duration::ZERO;
    let raw = pieces::transcript(done, &rest, |a| {
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
        guesses,
        guess_used,
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

/// The machine's 1-minute load average, where the system reports it. Latency means little without it.
pub fn load1() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg").ok()?.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::TAIL;

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
        // The clip ends mid-word: release waits a tail, and the quiet in the tail gets no guess.
        assert_eq!((d.pieces, d.guesses), (0, 0));
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
        // The pause outlasts the guess made in it (7.3 s of audio at SIM_RTF), so that guess becomes the piece.
        let audio = [speech(7.0), pause(1.2), speech(3.0)].concat();
        let d = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        assert_eq!(d.pieces, 1, "{}", d.raw);
        // The first piece was ready while the speaker was still talking.
        let key_up = Duration::from_secs_f64(audio.len() as f64 / RATE as f64);
        assert!(d.first < key_up, "{:?}", d.first);
        let words: Vec<usize> = d
            .raw
            .split(" samples.")
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        let tail = (TAIL.as_secs_f64() * RATE as f64) as usize;
        // The piece is the guess made in that pause, which the model heard with a tail's quiet after it; the rest
        // has the real tail. Together they cover every sample once.
        assert_eq!(words.iter().sum::<usize>(), audio.len() + 2 * tail);
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

    fn tail() -> usize {
        (TAIL.as_secs_f64() * RATE as f64) as usize
    }

    #[test]
    fn a_dictation_that_ends_in_a_pause_takes_its_guess_and_waits_for_nothing() {
        let audio = [speech(3.0), pause(1.0)].concat();
        let d = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        assert_eq!(d.guesses, 1, "one pause, one guess");
        assert!(d.guess_used);
        // The guess stopped inside the pause, and the model heard a tail's worth of quiet after it.
        let n: usize = d.raw.trim_end_matches(" samples.").parse().unwrap();
        assert!(n > 3 * RATE as usize + tail() && n < audio.len() + tail(), "{n}");
        // No tail and nothing to transcribe after release.
        assert!(d.after_release < Duration::from_millis(50), "{:?}", d.after_release);
    }

    #[test]
    fn speech_after_the_guess_throws_it_away() {
        let audio = [speech(2.0), pause(0.6), speech(1.5)].concat();
        let d = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        assert_eq!(d.guesses, 1);
        assert!(!d.guess_used);
        // One transcription of everything, the last word's tail included: the guess is nowhere in it.
        assert_eq!(d.raw, format!("{} samples.", audio.len() + tail()));
        assert!(d.after_release >= TAIL);
    }

    #[test]
    fn a_failed_guess_loses_no_text() {
        let audio = [speech(3.0), pause(1.0)].concat();
        let mut calls = 0;
        let mut model: Model = Box::new(move |a: &[f32]| {
            calls += 1;
            if calls == 1 {
                anyhow::bail!("busy")
            }
            Ok(format!("{} samples.", a.len()))
        });
        let d = simulated(&audio, &mut model, |t| t.to_string()).unwrap();
        assert_eq!(d.guesses, 1);
        assert!(!d.guess_used);
        // Release transcribed it all: the clip, then the quiet a tail would have recorded.
        assert_eq!(d.raw, format!("{} samples.", audio.len() + tail()));
    }

    #[test]
    fn live_guesses_like_simulated() {
        let audio = [speech(2.0), pause(0.8)].concat();
        let sim = simulated(&audio, &mut counting(), |t| t.to_string()).unwrap();
        let model = Arc::new(Mutex::new(counting()));
        let live = live(audio, &model, |t| t.to_string()).unwrap();
        assert!(sim.guess_used && live.guess_used, "{sim:?} {live:?}");
        assert!(live.after_release < TAIL, "{:?}", live.after_release);
    }

    #[test]
    fn simulated_gives_the_same_text_however_fast_the_model_runs() {
        // A piece, then short pauses a 50 ms poll can only just catch: whether each gets a guess used to depend on
        // where the model's real time had moved the poll to.
        let mut audio = [speech(7.0), pause(0.6), speech(1.0)].concat();
        for _ in 0..6 {
            audio.extend(pause(0.15));
            audio.extend(speech(0.3));
        }
        audio.extend(pause(0.6));
        let run = |ms: u64| {
            let mut model: Model = Box::new(move |a: &[f32]| {
                std::thread::sleep(Duration::from_millis(ms));
                Ok(format!("{} samples.", a.len()))
            });
            let d = simulated(&audio, &mut model, |t| t.to_string()).unwrap();
            (d.raw, d.pieces, d.guesses, d.guess_used, d.first)
        };
        let (fast, slow) = (run(0), run(30));
        assert_eq!(fast.1, 1, "{fast:?}");
        assert!(fast.2 > 1, "{fast:?}");
        assert_eq!(fast, slow);
    }

    #[test]
    fn load_is_read_where_linux_reports_it() {
        if cfg!(target_os = "linux") {
            assert!(load1().is_some_and(|l| l >= 0.0));
        }
    }

    #[test]
    fn usage_reads_this_process() {
        if let Some(u) = usage() {
            assert!(u.peak_rss_mb > 1.0);
        }
    }
}
