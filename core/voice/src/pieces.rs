//! Transcribes a long dictation while it's being spoken, a piece at a time, cutting only at pauses. On release only the
//! last few seconds are left, so the wait stays short however long someone talked: a 47-second dictation used to take
//! 3.5 s after the key came up (2026-11-07). A dictation with no piece ready works as it always did.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::audio::{resample, RATE};

/// How much unprocessed speech it takes before looking for a pause.
const MIN_PIECE_SECONDS: f32 = 6.0;
/// A pause this long usually ends a sentence. People hesitate for less than this inside one.
const SENTENCE_PAUSE_SECONDS: f32 = 0.45;
/// After this much unprocessed speech a shorter pause will do, or a fast talker would never get a cut.
const IMPATIENT_AFTER_SECONDS: f32 = 15.0;
const IMPATIENT_PAUSE_SECONDS: f32 = 0.3;
/// Quieter than this is the absence of speech on any microphone. Room tone can read lower; it can't count as louder.
const SILENCE_RMS: f32 = 0.003;
const FRAME_SECONDS: f32 = 0.02;
/// How often the worker looks at the microphone's buffer.
pub const POLL: Duration = Duration::from_millis(100);

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
    let floor = sorted[sorted.len() / 200];
    let speech = sorted[sorted.len() * 9 / 10];
    // Room tone only, or nothing that stands out from it: no way to tell a pause from a word.
    if speech < SILENCE_RMS || speech < floor * 3.0 {
        return None;
    }
    // Near the room's own level, so a soft word or a trailing-off voice never reads as a pause. In a room too loud to
    // tell, this finds no pause and the dictation goes the ordinary way.
    let quiet = (floor.max(0.0005) * 3.0).min(speech * 0.1);
    let seconds = audio.len() as f32 / rate as f32;
    let pause = if seconds >= IMPATIENT_AFTER_SECONDS {
        IMPATIENT_PAUSE_SECONDS
    } else {
        SENTENCE_PAUSE_SECONDS
    };
    let need = (pause / FRAME_SECONDS).ceil() as usize;
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
    /// When the first piece's text was ready.
    pub first: Option<Instant>,
}

/// Whether audio holds nothing to transcribe.
pub fn is_silence(audio: &[f32]) -> bool {
    audio.is_empty()
        || audio
            .chunks((RATE as f32 * FRAME_SECONDS) as usize)
            .all(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt() < SILENCE_RMS)
}

/// Words that carry a sentence on after a pause far more often than they name anything: the
/// small words that hold a sentence together. "I" and words that are also names ("Will",
/// "May", "Grace") are left out.
const JOINING_WORDS: &[&str] = &[
    "a", "about", "above", "after", "again", "against", "all", "almost", "along", "also",
    "although", "always", "am", "an", "and", "another", "any", "anyway", "are", "around", "as",
    "at", "be", "because", "been", "before", "behind", "being", "below", "between", "both",
    "but", "by", "can", "could", "did", "do", "does", "down", "each", "either", "else", "even",
    "ever", "every", "few", "for", "from", "had", "has", "have", "he", "her", "here", "hers",
    "him", "his", "how", "however", "if", "in", "into", "is", "it", "it's", "its", "just",
    "less", "like", "maybe", "me", "might", "more", "most", "much", "must", "my", "neither",
    "never", "no", "nor", "not", "now", "of", "off", "often", "on", "once", "only", "or",
    "other", "our", "ours", "out", "over", "perhaps", "quite", "rather", "really", "she",
    "should", "since", "so", "some", "still", "such", "than", "that", "that's", "the", "their",
    "them", "then", "there", "there's", "these", "they", "this", "those", "though", "through",
    "to", "too", "toward", "under", "unless", "until", "up", "upon", "us", "very", "was", "we",
    "were", "what", "whatever", "when", "where", "whether", "which", "while", "who", "whom",
    "whose", "why", "with", "within", "without", "would", "yet", "you", "your", "yours",
];

/// Whether a capital on this word, at the start of a piece, is only the model starting a new
/// piece. Names and other words that keep their capital mid-sentence say no.
fn only_starts_a_piece(word: &str) -> bool {
    let word = word.to_lowercase();
    JOINING_WORDS.contains(&word.as_str())
        || (word.len() > 5 && word.ends_with("ing") && crate::dictionary::is_everyday(&word))
}

/// Adds a piece's text to what's already there. The model capitalizes the start of every piece,
/// which is wrong when the one before stopped without ending its sentence; then a capital on a
/// joining word or an everyday "-ing" word comes off. Any other capital stays, since it may be
/// a name ("Donovan", "Paris") and a wrong capital reads better than a lowercased name.
pub fn join(text: &mut String, piece: &str) {
    let piece = piece.trim();
    if piece.is_empty() {
        return;
    }
    let inside_a_sentence = !text.is_empty() && !text.ends_with(['.', '?', '!', ':', ';', '"']);
    if !text.is_empty() {
        text.push(' ');
    }
    let first_word: String = piece.chars().take_while(|c| c.is_alphabetic() || *c == '\'').collect();
    let mut letters = piece.chars();
    match (letters.next(), letters.next()) {
        (Some(first), Some(second))
            if inside_a_sentence
                && first.is_uppercase()
                && second.is_lowercase()
                && only_starts_a_piece(&first_word) =>
        {
            text.extend(first.to_lowercase());
            text.push_str(&piece[first.len_utf8()..]);
        }
        _ => text.push_str(piece),
    }
}

/// The worker's decisions without its thread or clock, so a replay on simulated time cuts exactly where the
/// worker would.
pub struct Cutter {
    rate: u32,
    /// With no pause found, half a second of new audio before copying the buffer again.
    next_look: usize,
    pub done: Done,
}

impl Cutter {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            next_look: 0,
            done: Done::default(),
        }
    }

    /// Whether a buffer this long is worth a look now, and if so where the unprocessed audio starts.
    pub fn look_from(&mut self, len: usize) -> Option<usize> {
        let need = (MIN_PIECE_SECONDS * self.rate as f32) as usize;
        if len < self.done.raw_samples + need || len < self.next_look {
            return None;
        }
        self.next_look = len + self.rate as usize / 2;
        Some(self.done.raw_samples)
    }

    /// The next piece of the unprocessed audio: 16 kHz samples up to the cut, and how many buffer samples that covers.
    pub fn piece(&self, pending: &[f32]) -> Option<(Vec<f32>, usize)> {
        let mut audio = resample(pending, self.rate, RATE);
        let cut = find_cut(&audio, RATE)?;
        audio.truncate(cut);
        Some((
            audio,
            (cut as u64 * self.rate as u64 / RATE as u64) as usize,
        ))
    }

    pub fn add(&mut self, text: &str, raw_samples: usize) {
        join(&mut self.done.text, text);
        self.done.raw_samples += raw_samples;
        if !text.trim().is_empty() {
            self.done.first.get_or_insert_with(Instant::now);
        }
    }
}

/// The whole dictation's text: what the pieces got through, then `rest`, the audio after them. After a piece, a rest
/// that is only the end of a pause isn't transcribed; if the rest fails, the pieces' text is still worth typing.
pub fn transcript(
    done: Done,
    rest: &[f32],
    transcribe: impl FnOnce(&[f32]) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let tail = if !done.text.is_empty() && is_silence(rest) {
        Ok(String::new())
    } else {
        transcribe(rest)
    };
    match tail {
        Ok(t) => {
            let mut all = done.text;
            join(&mut all, &t);
            Ok(all)
        }
        Err(e) if !done.text.is_empty() => {
            log::warn!("transcribing the end of a dictation failed: {e}");
            Ok(done.text)
        }
        Err(e) => Err(e),
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
            let mut cutter = Cutter::new(rate);
            while !stopping.load(Ordering::Relaxed) {
                // A release wakes this at once, so it never adds to the wait.
                std::thread::park_timeout(POLL);
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let pending: Vec<f32> = {
                    let buffer = raw.lock();
                    let Some(from) = cutter.look_from(buffer.len()) else {
                        continue;
                    };
                    buffer[from..].to_vec()
                };
                let Some((audio, covers)) = cutter.piece(&pending) else {
                    continue;
                };
                match transcribe(&audio) {
                    Ok(piece) => cutter.add(&piece, covers),
                    // Whatever's left goes through at release, the ordinary way.
                    Err(e) => {
                        log::warn!("transcribing a piece failed: {e}");
                        break;
                    }
                }
            }
            cutter.done
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
            .and_then(|t| {
                t.thread().unpark();
                t.join().ok()
            })
            .unwrap_or_default()
    }
}

impl Drop for Pieces {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
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
        let audio = [speech(7.0), pause(0.6), speech(2.0)].concat();
        let cut = find_cut(&audio, RATE).expect("a pause after 6 s");
        assert!(is_quiet(&audio, cut), "cut at {cut} is in speech");
    }

    #[test]
    fn of_equal_pauses_it_takes_the_last_so_less_is_left_for_release() {
        let audio = [
            speech(7.0),
            pause(0.6),
            speech(3.0),
            pause(0.6),
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
    fn joining_fixes_a_capital_only_inside_a_sentence() {
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

    /// Review sol-sol-F3 on #23: a name after a pause lost its capital ("donovan").
    #[test]
    fn joining_keeps_the_capital_on_a_name() {
        let mut text = String::from("I talked with");
        join(&mut text, "Donovan about it and");
        join(&mut text, "Paris was lovely");
        join(&mut text, "John and Grace came,");
        join(&mut text, "Will said.");
        assert_eq!(text, "I talked with Donovan about it and Paris was lovely John and Grace came, Will said.");
        let mut small = String::from("We waited");
        join(&mut small, "And then it's");
        join(&mut small, "The best part,");
        join(&mut small, "It's fine");
        join(&mut small, "Thinking about it.");
        assert_eq!(small, "We waited and then it's the best part, it's fine thinking about it.");
    }

    #[test]
    fn a_pause_before_the_minimum_or_too_short_a_gap_doesnt_count() {
        let early = [speech(3.0), pause(0.5), speech(5.0)].concat();
        assert_eq!(find_cut(&early, RATE), None);
        let hesitation = [speech(7.0), pause(0.3), speech(2.0)].concat();
        assert_eq!(find_cut(&hesitation, RATE), None);
    }

    #[test]
    fn after_a_long_stretch_a_shorter_pause_will_do() {
        let audio = [speech(16.0), pause(0.35), speech(1.0)].concat();
        assert!(find_cut(&audio, RATE).is_some());
    }

    #[test]
    fn a_soft_word_between_loud_ones_is_not_a_pause() {
        let soft: Vec<f32> = speech(0.6).iter().map(|s| s * 0.08).collect();
        let audio = [speech(7.0), pause(0.2), soft, pause(0.2), speech(2.0)].concat();
        assert_eq!(find_cut(&audio, RATE), None);
    }

    #[test]
    fn a_word_after_the_last_cut_is_not_silence() {
        assert!(is_silence(&pause(0.3)));
        assert!(is_silence(&[]));
        assert!(!is_silence(&[pause(0.1), speech(0.2)].concat()));
    }

    #[test]
    fn finishing_an_idle_worker_wakes_it_at_once() {
        // Without the wake-up every finish waits out the rest of POLL (about 90 ms). A busy CI
        // runner can stall any one try (50 ms on macOS, 2026-11-22), so judge the fastest of five.
        let fastest = (0..5)
            .map(|_| {
                let pieces = Pieces::start(Arc::new(Mutex::new(vec![])), RATE, |_| Ok(String::new()));
                std::thread::sleep(Duration::from_millis(10));
                let t = std::time::Instant::now();
                pieces.finish();
                t.elapsed()
            })
            .min()
            .unwrap();
        assert!(fastest < Duration::from_millis(30), "{fastest:?}");
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
        let raw = Arc::new(Mutex::new([speech(7.0), pause(0.6), speech(2.0)].concat()));
        let pieces = Pieces::start(raw, RATE, |_| anyhow::bail!("no model"));
        std::thread::sleep(Duration::from_millis(400));
        let done = pieces.finish();
        assert_eq!((done.text.as_str(), done.raw_samples), ("", 0));
    }

    #[test]
    fn pieces_join_in_order_and_report_what_they_covered() {
        let raw = Arc::new(Mutex::new([speech(7.0), pause(0.6), speech(2.0)].concat()));
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
