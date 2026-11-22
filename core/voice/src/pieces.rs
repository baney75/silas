//! Transcribes a long dictation while it's being spoken, a piece at a time, cutting only at pauses. On release only the
//! last few seconds are left, so the wait stays short however long someone talked: a 47-second dictation used to take
//! 3.5 s after the key came up (2026-11-07). A dictation with no piece ready works as it always did.
//!
//! It also guesses: when the speaker stops for a moment, the audio not yet in a piece is transcribed then, while the
//! key is still down. If nothing but silence follows before release, that guess is the text and release decodes
//! nothing; if speech follows, the guess is thrown away and release works as it always did. Cutting a short dictation
//! at such a pause instead would lose the words around the cut (the model ends each piece as a sentence), so a guess
//! becomes a piece only where a piece would have been cut anyway.

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
/// A pause this long after speech, still going, is worth a guess: the speaker may be done. Shorter starts the guess
/// sooner after the last word, so less of it is left at key up, at the cost of more guesses thrown away. On chapel1's
/// ratchet clips 120 ms cut the median wait by 120 ms more than 200 ms did, for 9% more CPU (2026-11-22).
const GUESS_PAUSE_SECONDS: f32 = 0.12;
/// The most unprocessed speech worth a guess while no piece has been cut: a dictation that never paused long enough
/// for a piece has only its guess to shorten the wait. Once pieces are being cut, `MIN_PIECE_SECONDS` is the most.
const GUESS_MAX_SECONDS: f32 = 10.0;
/// Silence this long just before key up means nobody is mid-word, so release records no tail.
const QUIET_BEFORE_KEY_UP_SECONDS: f32 = 0.25;
/// How often the worker looks at the microphone's buffer.
pub const POLL: Duration = Duration::from_millis(50);

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
    /// Pieces cut before release.
    pub pieces: usize,
    /// The latest guess at the audio after `raw_samples`, made in a pause.
    pub guess: Option<Guess>,
    /// Guesses transcribed before release, used or not.
    pub guesses: usize,
}

/// The text of the audio after the pieces, transcribed in a pause before the key came up.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    /// Where the guessed audio starts in the microphone's buffer: what the pieces covered when it was made.
    pub from: usize,
    /// Where its speech ended and the pause began.
    pub spoke: usize,
    /// Where it ends in the buffer, inside the pause.
    pub upto: usize,
    /// How long the guessed audio is at 16 kHz, so release can find what came after it.
    pub samples: usize,
    pub text: String,
}

/// What the worker transcribes next.
pub enum Job {
    /// A piece, cut at a pause: 16 kHz audio and how many buffer samples it covers.
    Piece { audio: Vec<f32>, covers: usize },
    /// A guess at everything after the pieces, which ends in a pause.
    Guess { audio: Vec<f32>, from: usize, spoke: usize, upto: usize, samples: usize },
}

impl Job {
    pub fn audio(&self) -> &[f32] {
        match self {
            Job::Piece { audio, .. } | Job::Guess { audio, .. } => audio,
        }
    }

    pub fn is_piece(&self) -> bool {
        matches!(self, Job::Piece { .. })
    }
}

/// Copies of the buffer the worker wants, taken while it holds the lock; the work happens after.
#[derive(Default)]
pub struct Want {
    piece: Option<Vec<f32>>,
    guess: Option<(usize, usize, usize, Vec<f32>)>,
}

fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

/// Whether the last quarter second before key up is silence: nobody is mid-word, so release needs no tail.
/// `buffer` is the microphone's buffer at `rate` when the key came up.
pub fn ends_quiet(buffer: &[f32], rate: u32) -> bool {
    let n = (QUIET_BEFORE_KEY_UP_SECONDS * rate as f32) as usize;
    n > 0 && buffer.len() >= n && is_silence(&resample(&buffer[buffer.len() - n..], rate, RATE))
}

/// The quiet a tail would have recorded, added to 16 kHz audio that stopped without one. The model's text shifts a
/// little with how much silence trails the speech, so it sees the same as it would have after a tail.
pub fn pad_quiet(audio: &mut Vec<f32>) {
    let n = (crate::core::TAIL.as_secs_f64() * RATE as f64).round() as usize;
    audio.resize(audio.len() + n, 0.0);
}

/// Whether audio holds nothing to transcribe.
pub fn is_silence(audio: &[f32]) -> bool {
    audio.is_empty()
        || audio
            .chunks((RATE as f32 * FRAME_SECONDS) as usize)
            .all(|f| rms(f) < SILENCE_RMS)
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
    /// How much of the buffer has been listened to, in whole frames.
    heard: usize,
    /// Where the last frame louder than silence ended.
    spoke_until: usize,
    /// False after a guess fails: release does the work the ordinary way.
    guessing: bool,
    pub done: Done,
}

impl Cutter {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            next_look: 0,
            heard: 0,
            spoke_until: 0,
            guessing: true,
            done: Done::default(),
        }
    }

    /// Listens to the audio not heard yet, a frame at a time, noting where speech last ended. Cheap enough to run
    /// on every look while holding the buffer's lock.
    fn hear(&mut self, buffer: &[f32]) {
        let frame = ((self.rate as f32 * FRAME_SECONDS) as usize).max(1);
        while self.heard + frame <= buffer.len() {
            if rms(&buffer[self.heard..self.heard + frame]) >= SILENCE_RMS {
                self.spoke_until = self.heard + frame;
            }
            self.heard += frame;
        }
    }

    /// Where a guess would start and end now: speech after the pieces, a pause of `GUESS_PAUSE_SECONDS` since, and
    /// no guess yet that covers that speech. In a long dictation, past `MIN_PIECE_SECONDS` of unprocessed speech it
    /// doesn't guess: the cutter makes a piece at the next sentence's pause, and what follows the piece is short and
    /// cheap to guess. Guessing there instead would transcribe the same long stretch again at every hesitation.
    fn guess_due(&self) -> Option<(usize, usize, usize)> {
        let from = self.done.raw_samples;
        let pause = (GUESS_PAUSE_SECONDS * self.rate as f32) as usize;
        let most = if self.done.pieces == 0 {
            GUESS_MAX_SECONDS
        } else {
            MIN_PIECE_SECONDS
        };
        let cap = (most * self.rate as f32) as usize;
        if !self.guessing
            || self.spoke_until <= from
            || self.spoke_until - from >= cap
            || self.heard < self.spoke_until + pause
        {
            return None;
        }
        if self
            .done
            .guess
            .as_ref()
            .is_some_and(|g| g.from == from && g.upto >= self.spoke_until)
        {
            return None;
        }
        Some((from, self.spoke_until, self.heard))
    }

    /// What to copy out of the buffer now. Call it holding the buffer's lock; `job` does the work after.
    pub fn want(&mut self, buffer: &[f32]) -> Want {
        self.hear(buffer);
        Want {
            piece: self.look_from(buffer.len()).map(|from| buffer[from..].to_vec()),
            guess: self
                .guess_due()
                .map(|(from, spoke, upto)| (from, spoke, upto, buffer[from..upto].to_vec())),
        }
    }

    /// The next thing to transcribe: a piece if the audio has a place to cut, else a guess if one is due.
    pub fn job(&mut self, want: Want) -> Option<Job> {
        if let Some((audio, covers)) = want.piece.and_then(|pending| self.piece(&pending)) {
            // A guess made in the pause the cutter chose, with no speech since, is that piece already.
            let cut = self.done.raw_samples + covers;
            if let Some(g) = self.done.guess.take_if(|g| {
                g.from == self.done.raw_samples && g.spoke <= cut && self.spoke_until <= g.upto
            }) {
                self.add(&g.text, g.upto - g.from);
                return None;
            }
            return Some(Job::Piece { audio, covers });
        }
        let (from, spoke, upto, pending) = want.guess?;
        let mut audio = resample(&pending, self.rate, RATE);
        let samples = audio.len();
        pad_quiet(&mut audio);
        Some(Job::Guess { audio, from, spoke, upto, samples })
    }

    /// Takes in what the model made of a job. False when the worker should stop: a piece failed, and whatever's
    /// left goes through at release, the ordinary way.
    pub fn finished(&mut self, job: Job, text: anyhow::Result<String>) -> bool {
        match (job, text) {
            (Job::Piece { covers, .. }, Ok(text)) => {
                self.add(&text, covers);
                true
            }
            (Job::Piece { .. }, Err(e)) => {
                log::warn!("transcribing a piece failed: {e}");
                false
            }
            (Job::Guess { from, spoke, upto, samples, .. }, text) => {
                self.done.guesses += 1;
                match text {
                    Ok(text) if from == self.done.raw_samples => {
                        self.done.guess = Some(Guess { from, spoke, upto, samples, text });
                    }
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("guessing the end of a dictation failed: {e}");
                        self.done.guess = None;
                        self.guessing = false;
                    }
                }
                true
            }
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
        self.done.pieces += 1;
        // A guess started where the pieces used to end.
        self.done.guess = None;
        if !text.trim().is_empty() {
            self.done.first.get_or_insert_with(Instant::now);
        }
    }
}

/// Whether release can take the guess as the rest's text: it starts where the pieces end and only silence follows it.
pub fn guess_ends_it(done: &Done, rest: &[f32]) -> bool {
    done.guess
        .as_ref()
        .is_some_and(|g| g.from == done.raw_samples && is_silence(&rest[g.samples.min(rest.len())..]))
}

/// The whole dictation's text: what the pieces got through, then `rest`, the audio after them. If a guess covers the
/// start of the rest and only silence follows it, the guess is the text and nothing is transcribed; a guess that speech
/// came after is never typed, and the whole rest is transcribed instead. After a piece, a
/// rest that is only the end of a pause isn't transcribed; if the rest fails, the pieces' text is still worth typing.
pub fn transcript(
    done: Done,
    rest: &[f32],
    transcribe: impl FnOnce(&[f32]) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let ends_it = guess_ends_it(&done, rest);
    let Done { mut text, guess, .. } = done;
    if let Some(g) = guess.filter(|_| ends_it) {
        join(&mut text, &g.text);
        return Ok(text);
    }
    let tail = if !text.is_empty() && is_silence(rest) {
        Ok(String::new())
    } else {
        transcribe(rest)
    };
    match tail {
        Ok(t) => {
            join(&mut text, &t);
            Ok(text)
        }
        Err(e) if !text.is_empty() => {
            log::warn!("transcribing the end of a dictation failed: {e}");
            Ok(text)
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
                let want = cutter.want(&raw.lock());
                let Some(job) = cutter.job(want) else {
                    continue;
                };
                let text = transcribe(job.audio());
                if !cutter.finished(job, text) {
                    break;
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
        for _ in 0..5 {
            let pieces = Pieces::start(Arc::new(Mutex::new(vec![])), RATE, |_| Ok(String::new()));
            std::thread::sleep(Duration::from_millis(10));
            let t = std::time::Instant::now();
            pieces.finish();
            assert!(t.elapsed() < Duration::from_millis(30), "{:?}", t.elapsed());
        }
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
    fn the_tail_is_skipped_only_after_a_quarter_second_of_silence() {
        assert!(ends_quiet(&[speech(1.0), pause(0.3)].concat(), RATE));
        assert!(!ends_quiet(&[speech(1.0), pause(0.2)].concat(), RATE));
        assert!(!ends_quiet(&speech(1.0), RATE));
        // Too little audio to tell: keep the tail.
        assert!(!ends_quiet(&pause(0.2), RATE));
        // A microphone at another rate.
        let at_48k: Vec<f32> = [speech(1.0), pause(0.3)].concat().iter().flat_map(|&s| [s; 3]).collect();
        assert!(ends_quiet(&at_48k, 48_000));
        let at_48k: Vec<f32> = speech(1.0).iter().flat_map(|&s| [s; 3]).collect();
        assert!(!ends_quiet(&at_48k, 48_000));
    }

    #[test]
    fn padding_adds_a_tails_worth_of_quiet() {
        let mut audio = speech(1.0);
        pad_quiet(&mut audio);
        let n = (crate::core::TAIL.as_secs_f64() * RATE as f64).round() as usize;
        assert_eq!(audio.len(), RATE as usize + n);
        assert!(is_silence(&audio[RATE as usize..]));
    }

    fn guessed(text: &str, from: usize, samples: usize) -> Done {
        Done {
            text: "Before.".into(),
            raw_samples: from,
            guess: Some(Guess { from, spoke: from + samples / 2, upto: from + samples, samples, text: text.into() }),
            ..Done::default()
        }
    }

    fn redo(audio: &[f32]) -> anyhow::Result<String> {
        Ok(format!("redo {}", audio.len()))
    }

    /// A guess at two seconds of speech and a little of the pause after it.
    const GUESSED: usize = 2 * RATE as usize + 4_800;

    #[test]
    fn a_guess_with_only_silence_after_it_is_the_text() {
        let rest = [speech(2.0), pause(0.5)].concat();
        let text = transcript(guessed("Guess.", 0, GUESSED), &rest, |_| anyhow::bail!("must not decode")).unwrap();
        assert_eq!(text, "Before. Guess.");
    }

    #[test]
    fn a_guess_that_speech_came_after_is_never_typed() {
        let rest = [speech(2.0), pause(0.5), speech(0.5)].concat();
        let text = transcript(guessed("Guess.", 0, GUESSED), &rest, redo).unwrap();
        assert_eq!(text, format!("Before. redo {}", rest.len()));
    }

    #[test]
    fn a_guess_from_before_the_last_piece_is_ignored() {
        let rest = pause(0.5);
        let mut done = guessed("Stale.", 100, 10);
        done.raw_samples = 500;
        let text = transcript(done, &rest, |_| anyhow::bail!("must not decode")).unwrap();
        assert_eq!(text, "Before.");
    }

    #[test]
    fn a_failed_rest_after_a_dropped_guess_keeps_the_pieces_text() {
        let rest = [speech(2.0), pause(0.5), speech(0.5)].concat();
        let text = transcript(guessed("Guess.", 0, GUESSED), &rest, |_| anyhow::bail!("no model")).unwrap();
        assert_eq!(text, "Before.");
    }

    /// Feeds `audio` to a cutter the way the worker sees the buffer grow, `POLL` at a time, running every job.
    fn run(audio: &[f32]) -> (Cutter, Vec<(bool, usize)>) {
        let mut cutter = Cutter::new(RATE);
        let mut jobs = vec![];
        let step = (POLL.as_secs_f64() * RATE as f64) as usize;
        let mut len = 0;
        while len < audio.len() {
            len = (len + step).min(audio.len());
            let want = cutter.want(&audio[..len]);
            if let Some(job) = cutter.job(want) {
                jobs.push((job.is_piece(), job.audio().len()));
                let text = format!("{}.", job.audio().len());
                cutter.finished(job, Ok(text));
            }
        }
        (cutter, jobs)
    }

    #[test]
    fn short_pauses_get_guesses_and_never_become_pieces() {
        let audio = [speech(2.5), pause(0.5), speech(2.5), pause(0.5)].concat();
        let (cutter, jobs) = run(&audio);
        assert_eq!(jobs.len(), 2, "{jobs:?}");
        assert!(jobs.iter().all(|(piece, _)| !piece), "{jobs:?}");
        assert!(jobs[1].1 > 5 * RATE as usize, "the second guess covers it all: {jobs:?}");
        assert_eq!(cutter.done.pieces, 0);
        let rest = audio[cutter.done.raw_samples..].to_vec();
        assert!(guess_ends_it(&cutter.done, &rest));
    }

    #[test]
    fn after_a_piece_long_unprocessed_speech_is_left_to_the_cutter() {
        let audio = [speech(7.0), pause(0.6), speech(6.5), pause(0.3), speech(0.5), pause(0.6), speech(2.0), pause(0.5)].concat();
        let (cutter, jobs) = run(&audio);
        // The first 7 s is guessed (no piece yet) and kept as the piece. The next 6.5 s gets no guess at its
        // hesitation, only a piece at the sentence's pause, and then the short rest is guessed.
        let kinds: Vec<bool> = jobs.iter().map(|(piece, _)| *piece).collect();
        assert_eq!(kinds, [false, true, false], "{jobs:?}");
        assert!(jobs[2].1 < 3 * RATE as usize, "the last guess starts at the piece: {jobs:?}");
        assert_eq!(cutter.done.pieces, 2);
        let rest = audio[cutter.done.raw_samples..].to_vec();
        assert!(guess_ends_it(&cutter.done, &rest));
    }

    #[test]
    fn a_short_dictation_is_guessed_up_to_its_limit() {
        let under = [speech(9.5), pause(0.4)].concat();
        let (cutter, jobs) = run(&under);
        assert_eq!((jobs.len(), cutter.done.guesses), (1, 1), "{jobs:?}");
        let over = [speech(10.5), pause(0.4)].concat();
        let (cutter, jobs) = run(&over);
        assert_eq!((jobs.len(), cutter.done.guesses), (0, 0), "{jobs:?}");
    }

    #[test]
    fn a_guess_in_the_pause_the_cutter_chooses_becomes_the_piece() {
        let audio = [speech(5.9), pause(0.8)].concat();
        let (cutter, jobs) = run(&audio);
        // Transcribed once, as a guess, then kept as the piece the cutter would have cut there.
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        assert!(!jobs[0].0, "{jobs:?}");
        assert_eq!((cutter.done.pieces, cutter.done.guess.is_none()), (1, true));
        assert!(is_silence(&audio[cutter.done.raw_samples..]));
    }

    #[test]
    fn the_worker_guesses_in_a_pause_once() {
        let raw = Arc::new(Mutex::new([speech(2.0), pause(0.8)].concat()));
        let calls = Arc::new(Mutex::new(0));
        let counted = calls.clone();
        let pieces = Pieces::start(raw, RATE, move |audio| {
            *counted.lock() += 1;
            Ok(format!("{} samples", audio.len()))
        });
        std::thread::sleep(Duration::from_millis(400));
        let done = pieces.finish();
        assert_eq!(*calls.lock(), 1, "the same pause is guessed once");
        assert_eq!((done.pieces, done.guesses, done.raw_samples), (0, 1, 0));
        let g = done.guess.expect("a guess");
        assert!(g.upto >= 2 * RATE as usize + 4_800 && g.samples == g.upto, "{g:?}");
    }

    #[test]
    fn noise_that_never_falls_silent_gets_no_guess() {
        let raw = Arc::new(Mutex::new(speech(3.0)));
        let pieces = Pieces::start(raw, RATE, |_| Ok("x".into()));
        std::thread::sleep(Duration::from_millis(300));
        let done = pieces.finish();
        assert_eq!((done.guesses, done.guess), (0, None));
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
