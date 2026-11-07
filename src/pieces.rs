//! Transcribes a long dictation while it's being spoken, a piece at a time, cutting only at pauses. On release only the
//! last few seconds are left, so the wait stays short however long someone talked: a 47-second dictation used to take
//! 3.5 s after the key came up (2026-11-07). A dictation with no piece ready works as it always did.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::audio::resample;

/// How much unprocessed speech it takes before looking for a pause.
const MIN_PIECE_SECONDS: f32 = 6.0;
/// A pause this long is between words people meant to separate. Shorter gaps fall inside sentences.
const MIN_PAUSE_SECONDS: f32 = 0.25;
const FRAME_SECONDS: f32 = 0.02;
/// How often the worker looks at the microphone's buffer.
const POLL: Duration = Duration::from_millis(100);

/// Where to cut this audio: the middle of its longest pause at least `MIN_PIECE_SECONDS` in, the latest of equals, or
/// None to wait for more. Longest because people pause longer between sentences than inside them.
pub fn find_cut(audio: &[f32], rate: u32) -> Option<usize> {
    let frame = ((rate as f32 * FRAME_SECONDS) as usize).max(1);
    if audio.len() < (MIN_PIECE_SECONDS * rate as f32) as usize {
        return None;
    }
    let levels: Vec<f32> = audio
        .chunks(frame)
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    let mut sorted = levels.clone();
    sorted.sort_by(f32::total_cmp);
    // Pauses can be a tiny share of the frames, so the floor is taken near the very bottom.
    let floor = sorted[sorted.len() / 50];
    let speech = sorted[sorted.len() * 9 / 10];
    // Room tone only, or nothing that stands out from it: no way to tell a pause from a word.
    if speech < 0.003 || speech < floor * 3.0 {
        return None;
    }
    let quiet = (floor * 2.0).max(speech * 0.1);
    let need = (MIN_PAUSE_SECONDS / FRAME_SECONDS).ceil() as usize;
    let earliest = (MIN_PIECE_SECONDS / FRAME_SECONDS) as usize;
    let mut best: Option<(usize, usize)> = None;
    let mut run_start = None;
    for (i, level) in levels.iter().enumerate() {
        if *level < quiet {
            let start = *run_start.get_or_insert(i);
            let length = i + 1 - start;
            // The middle of what's been heard of the pause so far is silence, even if it's still going.
            if length >= need
                && (start + i) / 2 >= earliest
                && best.is_none_or(|(l, _)| length >= l)
            {
                best = Some((length, (start + i) / 2 * frame));
            }
        } else {
            run_start = None;
        }
    }
    best.map(|(_, cut)| cut)
}

/// What the worker got through.
#[derive(Default)]
pub struct Done {
    pub text: String,
    /// How many samples of the microphone's buffer that text covers.
    pub raw_samples: usize,
}

/// Adds a piece's text to what's already there. The model capitalizes the start of every piece, which is wrong after a
/// comma: the cut fell inside a sentence.
pub fn join(text: &mut String, piece: &str) {
    let piece = piece.trim();
    if piece.is_empty() {
        return;
    }
    let inside_a_sentence = text.ends_with(',');
    if !text.is_empty() {
        text.push(' ');
    }
    let mut letters = piece.chars();
    match (letters.next(), letters.next()) {
        (Some(first), Some(second))
            if inside_a_sentence && first.is_uppercase() && second.is_lowercase() =>
        {
            text.extend(first.to_lowercase());
            text.push_str(&piece[first.len_utf8()..]);
        }
        _ => text.push_str(piece),
    }
}

pub struct Pieces {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Done>>,
}

impl Pieces {
    /// `raw` is the microphone's buffer at `rate`; `transcribe` takes 16 kHz audio.
    pub fn start(
        raw: Arc<Mutex<Vec<f32>>>,
        rate: u32,
        mut transcribe: impl FnMut(&[f32]) -> anyhow::Result<String> + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut done = Done::default();
            let need = (MIN_PIECE_SECONDS * rate as f32) as usize;
            while !stopping.load(Ordering::Relaxed) {
                std::thread::sleep(POLL);
                let pending: Vec<f32> = {
                    let buffer = raw.lock();
                    if buffer.len() < done.raw_samples + need {
                        continue;
                    }
                    buffer[done.raw_samples..].to_vec()
                };
                let audio = resample(&pending, rate, crate::audio::RATE);
                let Some(cut) = find_cut(&audio, crate::audio::RATE) else {
                    continue;
                };
                match transcribe(&audio[..cut]) {
                    Ok(piece) => {
                        join(&mut done.text, &piece);
                        done.raw_samples +=
                            (cut as u64 * rate as u64 / crate::audio::RATE as u64) as usize;
                    }
                    // Whatever's left goes through at release, the ordinary way.
                    Err(e) => {
                        log::warn!("transcribing a piece failed: {e}");
                        break;
                    }
                }
            }
            done
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops looking and returns what's done, waiting for a piece in progress.
    pub fn finish(mut self) -> Done {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .take()
            .and_then(|t| t.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for Pieces {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

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

    fn is_quiet(audio: &[f32], at: usize) -> bool {
        let f = &audio[at.saturating_sub(40)..(at + 40).min(audio.len())];
        f.iter().all(|s| s.abs() < 0.01)
    }

    #[test]
    fn short_audio_is_left_alone() {
        let audio = [speech(2.0), pause(0.5), speech(2.0)].concat();
        assert_eq!(find_cut(&audio, RATE), None);
    }

    #[test]
    fn it_cuts_inside_a_pause_never_inside_speech() {
        let audio = [speech(7.0), pause(0.4), speech(2.0)].concat();
        let cut = find_cut(&audio, RATE).expect("a pause after 6 s");
        assert!(is_quiet(&audio, cut), "cut at {cut} is in speech");
    }

    #[test]
    fn of_equal_pauses_it_takes_the_last_so_less_is_left_for_release() {
        let audio = [
            speech(7.0),
            pause(0.4),
            speech(3.0),
            pause(0.4),
            speech(1.0),
        ]
        .concat();
        let cut = find_cut(&audio, RATE).unwrap();
        assert!(
            cut as f32 / RATE as f32 > 10.0,
            "cut at {} s",
            cut as f32 / RATE as f32
        );
        assert!(is_quiet(&audio, cut));
    }

    #[test]
    fn the_longer_pause_wins_because_its_more_likely_a_sentence_end() {
        let audio = [
            speech(7.0),
            pause(0.8),
            speech(3.0),
            pause(0.3),
            speech(1.0),
        ]
        .concat();
        let cut = find_cut(&audio, RATE).unwrap();
        assert!(
            (cut as f32 / RATE as f32) < 8.0,
            "cut at {} s",
            cut as f32 / RATE as f32
        );
    }

    #[test]
    fn joining_fixes_a_capital_only_after_a_comma() {
        let mut text = String::new();
        join(&mut text, "Her knees drawn up to her chin,");
        join(&mut text, "Laughing like a girl.");
        join(&mut text, "Do we know?");
        join(&mut text, "  ");
        assert_eq!(
            text,
            "Her knees drawn up to her chin, laughing like a girl. Do we know?"
        );
        let mut names = String::from("We went to Paris,");
        join(&mut names, "I said.");
        assert_eq!(names, "We went to Paris, I said.");
    }

    #[test]
    fn a_pause_before_the_minimum_or_too_short_a_gap_doesnt_count() {
        let early = [speech(3.0), pause(0.5), speech(5.0)].concat();
        assert_eq!(find_cut(&early, RATE), None);
        let brief = [speech(7.0), pause(0.1), speech(2.0)].concat();
        assert_eq!(find_cut(&brief, RATE), None);
    }

    #[test]
    fn nothing_but_silence_has_no_cut() {
        assert_eq!(find_cut(&pause(10.0), RATE), None);
        assert_eq!(find_cut(&vec![0.0; 160_000], RATE), None);
    }

    #[test]
    fn one_continuous_stretch_has_no_cut() {
        assert_eq!(find_cut(&speech(30.0), RATE), None);
    }

    #[test]
    fn a_failed_piece_leaves_the_rest_for_release() {
        let raw = Arc::new(Mutex::new([speech(7.0), pause(0.4), speech(2.0)].concat()));
        let pieces = Pieces::start(raw, RATE, |_| anyhow::bail!("no model"));
        std::thread::sleep(Duration::from_millis(400));
        let done = pieces.finish();
        assert_eq!((done.text.as_str(), done.raw_samples), ("", 0));
    }

    #[test]
    fn pieces_join_in_order_and_report_what_they_covered() {
        let raw = Arc::new(Mutex::new([speech(7.0), pause(0.4), speech(2.0)].concat()));
        let total = raw.lock().len();
        let pieces = Pieces::start(raw.clone(), RATE, |audio| {
            Ok(format!("{} samples", audio.len()))
        });
        std::thread::sleep(Duration::from_millis(400));
        let done = pieces.finish();
        assert!(
            done.raw_samples > 7 * RATE as usize && done.raw_samples < total,
            "{}",
            done.raw_samples
        );
        assert_eq!(done.text, format!("{} samples", done.raw_samples));
    }
}
