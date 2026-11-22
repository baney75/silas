//! Transcribes an audio or video file: `silas transcribe <file>` on the command line, and the app's
//! "Transcribe a file" button and drag and drop. The result goes to stdout or History; it is never typed.
//!
//! The file is decoded with Symphonia (pure Rust) plus a pure-Rust Opus decoder, mixed to mono and
//! brought to 16 kHz a second at a time with `audio::resample`, so a long file never sits in memory at
//! its own rate. It goes through the model in pieces cut at the first sentence pause after 6 s, as a
//! long dictation is (`pieces::find_cut`), and joined the same way. Short pieces matter: given 25 to
//! 30 s at once, Parakeet v3 dropped whole sentences (the bench clips joined into one file came back
//! at 36% word errors with 30 s pieces, 2026-11-22).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde_json::json;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::common::Tier;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

use crate::audio::{resample, RATE};
use crate::core::Shared;
use crate::pieces::{find_cut, is_silence, join};
use crate::store::{self, HistoryItem};

/// The longest piece the model gets at once, when the speech has no pause to cut at sooner.
/// Parakeet v3 kept every word at 19 s and dropped sentences at 26 to 30 s.
pub const MAX_PIECE_SECONDS: f32 = 20.0;

/// The file types the picker offers and drag and drop takes. Symphonia probes the content, so a file
/// with another name still works from the command line.
pub const EXTENSIONS: &[&str] = &[
    "wav", "mp3", "m4a", "flac", "ogg", "oga", "opus", "mp4", "m4v", "mov", "webm", "mkv", "aac", "aif", "aiff", "caf",
];

/// What a file that can't be read gets told.
pub fn unreadable_message() -> String {
    "Silas can't read that file. It reads WAV, MP3, M4A, FLAC, OGG and Opus audio, and the sound in MP4, MOV and WebM videos.".into()
}

// ------------------------------------------------------------ decoding

/// A file being decoded, a second of 16 kHz mono audio at a time.
pub struct Decoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track: u32,
    /// The sample rate of `pending`.
    rate: u32,
    /// Mono samples at `rate`, not yet brought to 16 kHz.
    pending: Vec<f32>,
    planes: Vec<Vec<f32>>,
    /// Packets decoded, and how many of those failed.
    packets: usize,
    bad_packets: usize,
    done: bool,
    /// How long the file is, when it says.
    pub seconds: Option<f32>,
}

fn codecs() -> &'static CodecRegistry {
    static REGISTRY: std::sync::OnceLock<CodecRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut r = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut r);
        // Fallback, so a first-party Opus decoder in a later Symphonia takes over by itself.
        r.register_audio_decoder_at_tier::<symphonia_opus::OpusDecoder>(Tier::Fallback);
        r
    })
}

impl Decoder {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let file = std::fs::File::open(path).map_err(|e| anyhow::anyhow!("Can't open {}: {e}", path.display()))?;
        let source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, source, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| {
                log::info!("file: {}: {e}", path.display());
                anyhow::anyhow!(unreadable_message())
            })?;
        Self::from_format(format)
    }

    fn from_format(format: Box<dyn FormatReader>) -> anyhow::Result<Self> {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| anyhow::anyhow!("That file has no sound to transcribe."))?;
        let Some(CodecParameters::Audio(params)) = track.codec_params.clone() else {
            anyhow::bail!("That file has no sound to transcribe.");
        };
        let seconds = match (track.num_frames, params.sample_rate) {
            (Some(frames), Some(rate)) if rate > 0 => Some(frames as f32 / rate as f32),
            _ => None,
        };
        let id = track.id;
        let decoder = codecs().make_audio_decoder(&params, &AudioDecoderOptions::default()).map_err(|e| {
            log::info!("file: no decoder for {:?}: {e}", params.codec);
            anyhow::anyhow!(unreadable_message())
        })?;
        Ok(Self {
            format,
            decoder,
            track: id,
            rate: params.sample_rate.unwrap_or(0),
            pending: Vec::new(),
            planes: Vec::new(),
            packets: 0,
            bad_packets: 0,
            done: false,
            seconds,
        })
    }

    /// The next 16 kHz mono audio, about a second of it; None at the end of the file.
    pub fn next_block(&mut self) -> anyhow::Result<Option<Vec<f32>>> {
        while !self.done && (self.rate == 0 || self.pending.len() < self.rate as usize) {
            self.read_packet()?;
        }
        if self.pending.is_empty() {
            return Ok(None);
        }
        // A whole second at a time comes out at exactly 16,000 samples, so no rounding builds up.
        let take = if self.done { self.pending.len() } else { self.rate as usize };
        let block = resample(&self.pending[..take], self.rate, RATE);
        self.pending.drain(..take);
        Ok(Some(block))
    }

    fn read_packet(&mut self) -> anyhow::Result<()> {
        let packet = match self.format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return self.end(),
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return self.end(),
            // The decoder can't go on from here; keep what came before.
            Err(SymError::ResetRequired) => return self.end(),
            Err(e) => {
                log::info!("file: reading stopped: {e}");
                return if self.packets > 0 { self.end() } else { Err(anyhow::anyhow!(unreadable_message())) };
            }
        };
        if packet.track_id != self.track {
            return Ok(());
        }
        self.packets += 1;
        let buffer = match self.decoder.decode(&packet) {
            Ok(b) => b,
            // One damaged packet costs a few milliseconds of sound, not the file.
            Err(SymError::DecodeError(e)) => {
                self.bad_packets += 1;
                log::debug!("file: skipped a damaged packet: {e}");
                return Ok(());
            }
            Err(e) => {
                log::info!("file: decoding stopped: {e}");
                return self.end();
            }
        };
        let rate = buffer.spec().rate();
        if rate == 0 {
            return Ok(());
        }
        if rate != self.rate {
            // A stream that changes rate: what's pending goes out at the old one first.
            if !self.pending.is_empty() && self.rate > 0 {
                let old = resample(&self.pending, self.rate, RATE);
                self.pending = resample(&old, RATE, rate);
            }
            self.rate = rate;
        }
        buffer.copy_to_vecs_planar::<f32>(&mut self.planes);
        let frames = buffer.frames();
        let channels = self.planes.len().max(1) as f32;
        self.pending.extend((0..frames).map(|i| self.planes.iter().map(|p| p[i]).sum::<f32>() / channels));
        Ok(())
    }

    fn end(&mut self) -> anyhow::Result<()> {
        self.done = true;
        if self.packets > 0 && self.bad_packets == self.packets {
            anyhow::bail!(unreadable_message());
        }
        Ok(())
    }
}

/// The whole file as 16 kHz mono.
pub fn decode(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut d = Decoder::open(path)?;
    let mut out = Vec::new();
    while let Some(block) = d.next_block()? {
        out.extend(block);
    }
    Ok(out)
}

// ------------------------------------------------------------ transcribing

/// Where to end the next piece of `window` (16 kHz, one full piece long): at its longest pause, or
/// with no clear pause (music under the speech, a noisy room) at its quietest moment in the second half.
pub fn next_cut(window: &[f32]) -> usize {
    if let Some(cut) = find_cut(window, RATE) {
        return cut;
    }
    let frame = (RATE / 50) as usize;
    let half = window.len() / 2 / frame * frame;
    window[half..]
        .chunks(frame)
        .enumerate()
        .map(|(i, f)| (i, f.iter().map(|s| s * s).sum::<f32>()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or(window.len(), |(i, _)| half + i * frame + frame / 2)
        .clamp(1, window.len())
}

/// A file's transcript and what it took.
#[derive(Debug, Default, Clone)]
pub struct Outcome {
    /// The model's text, pieces joined, before the dictionary.
    pub raw: String,
    pub audio_seconds: f32,
    pub pieces: usize,
    pub decode: Duration,
    pub transcribe: Duration,
}

/// Transcribes the audio `next` hands over (16 kHz blocks, None at the end), a piece at a time.
/// `progress` gets the seconds of audio done after each piece; `cancelled` is asked between pieces.
pub fn transcribe(
    next: &mut dyn FnMut() -> anyhow::Result<Option<Vec<f32>>>,
    model: &mut dyn FnMut(&[f32]) -> anyhow::Result<String>,
    progress: &mut dyn FnMut(f32),
    cancelled: &dyn Fn() -> bool,
) -> anyhow::Result<Outcome> {
    let max = (MAX_PIECE_SECONDS * RATE as f32) as usize;
    let mut out = Outcome::default();
    let mut buffer: Vec<f32> = Vec::new();
    let mut done_samples = 0usize;
    let mut piece = |audio: &[f32], out: &mut Outcome| -> anyhow::Result<()> {
        anyhow::ensure!(!cancelled(), crate::core::CANCELLED);
        // Silence gets nothing from the model but a stray word.
        if !is_silence(audio) {
            let t = Instant::now();
            let text = model(audio)?;
            out.transcribe += t.elapsed();
            join(&mut out.raw, &text);
            out.pieces += 1;
        }
        done_samples += audio.len();
        progress(done_samples as f32 / RATE as f32);
        Ok(())
    };
    loop {
        let t = Instant::now();
        let block = next()?;
        out.decode += t.elapsed();
        let Some(block) = block else { break };
        out.audio_seconds += block.len() as f32 / RATE as f32;
        buffer.extend(block);
        // A cut as soon as there's a pause, as in a dictation; at the cap, the best place there is.
        loop {
            let window = &buffer[..buffer.len().min(max)];
            let cut = match find_cut(window, RATE) {
                Some(cut) => cut,
                None if buffer.len() >= max => next_cut(window),
                None => break,
            };
            piece(&buffer[..cut], &mut out)?;
            buffer.drain(..cut);
        }
    }
    if !buffer.is_empty() {
        piece(&buffer, &mut out)?;
    }
    Ok(out)
}

// ------------------------------------------------------------ in the app

/// One file at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Said when a file is dropped while a dictation is under way.
pub fn busy_message() -> String {
    "Finish your dictation first, then transcribe the file.".into()
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl crate::core::Core {
    /// Transcribe a file with the loaded model and keep the text in History. Emits `file-progress`.
    /// Refused while a dictation is under way; a dictation that starts meanwhile goes first, and the
    /// file waits between pieces.
    pub fn transcribe_file(self: &Shared, path: &Path) -> anyhow::Result<HistoryItem> {
        let name = file_name(path);
        let emit = |stage: &str, done: f32, total: Option<f32>, extra: serde_json::Value| {
            let mut p = json!({ "name": name, "stage": stage, "doneS": done, "totalS": total });
            if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
                p.extend(extra.clone());
            }
            self.host.emit("file-progress", p);
        };
        // A refusal is an event too: a dropped file has nobody waiting on the command.
        let refuse = |message: String| {
            emit("refused", 0.0, None, json!({ "message": message }));
            anyhow::anyhow!(message)
        };
        if self.dictating() {
            return Err(refuse(busy_message()));
        }
        if self.active_model().is_none() {
            return Err(refuse("No speech model is loaded yet. Try again in a moment.".into()));
        }
        if RUNNING.swap(true, Ordering::AcqRel) {
            return Err(refuse("Silas is already transcribing a file. Wait for it to finish.".into()));
        }
        let _running = Running;
        CANCEL.store(false, Ordering::Release);
        let started = Instant::now();
        emit("decoding", 0.0, None, json!({}));
        let result = (|| {
            let mut decoder = Decoder::open(path)?;
            let total = decoder.seconds;
            emit("transcribing", 0.0, total, json!({}));
            let language = self.settings.lock().language.clone();
            let done_so_far = std::cell::Cell::new(0.0f32);
            let mut model = |audio: &[f32]| -> anyhow::Result<String> {
                // A dictation has the model first; the file waits for it between pieces.
                if self.dictating() {
                    emit("waiting", done_so_far.get(), total, json!({}));
                    while self.dictating() {
                        anyhow::ensure!(!CANCEL.load(Ordering::Acquire), crate::core::CANCELLED);
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
                self.transcribe_audio(audio, &language)
            };
            transcribe(
                &mut || decoder.next_block(),
                &mut model,
                &mut |done| {
                    done_so_far.set(done);
                    emit("transcribing", done, total, json!({}))
                },
                &|| CANCEL.load(Ordering::Acquire),
            )
        })();
        let outcome = match result {
            Ok(o) => o,
            Err(e) => {
                let stage = if e.to_string() == crate::core::CANCELLED { "cancelled" } else { "error" };
                emit(stage, 0.0, None, json!({ "message": e.to_string() }));
                return Err(e);
            }
        };
        let remove_fillers = self.settings.lock().remove_fillers;
        let text = self.dict.lock().apply(&outcome.raw, remove_fillers);
        if text.trim().is_empty() {
            let message = "Silas heard no speech in that file.";
            emit("error", outcome.audio_seconds, Some(outcome.audio_seconds), json!({ "message": message }));
            anyhow::bail!(message);
        }
        let now = Utc::now();
        let item = HistoryItem {
            id: now.timestamp_millis().to_string(),
            ts: now.to_rfc3339(),
            words: text.split_whitespace().count(),
            text: text.trim().to_string(),
            raw: outcome.raw,
            duration_s: outcome.audio_seconds,
            latency_ms: started.elapsed().as_millis() as u64,
            corrected: None,
            file: Some(name.clone()),
        };
        {
            let mut h = self.history.lock();
            h.insert(0, item.clone());
            h.truncate(store::HISTORY_KEEP);
            store::save("history.json", &*h);
        }
        store::save("dictionary.json", &*self.dict.lock());
        emit("done", outcome.audio_seconds, Some(outcome.audio_seconds), json!({ "item": item }));
        Ok(item)
    }

    /// Stop the file being transcribed. False when none is.
    pub fn cancel_file(&self) -> bool {
        let running = RUNNING.load(Ordering::Acquire);
        if running {
            CANCEL.store(true, Ordering::Release);
        }
        running
    }
}

// ------------------------------------------------------------ the command line

const USAGE: &str = "Usage: silas transcribe <file> [--raw] [--json] [--model <id>] [--data-dir <dir>]

Prints what was said in an audio or video file: WAV, MP3, M4A, FLAC, OGG, Opus, or the sound in
an MP4, MOV or WebM video. Uses the speech model and dictionary Silas uses.

  --raw            the model's own words, without your dictionary or filler removal
  --json           text, raw text and timings as JSON
  --model <id>     another installed model (default: the one Silas uses)
  --data-dir <dir> Silas's data folder (default: the app's)";

#[derive(Debug, Default, PartialEq)]
struct Args {
    file: PathBuf,
    raw: bool,
    json: bool,
    model: Option<String>,
    data_dir: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut file = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--raw" => a.raw = true,
            "--json" => a.json = true,
            "--model" => a.model = Some(it.next().ok_or("--model needs a model id")?.clone()),
            "--data-dir" => a.data_dir = Some(it.next().ok_or("--data-dir needs a folder")?.into()),
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("Unknown option {s}")),
            s if file.is_none() => file = Some(PathBuf::from(s)),
            s => return Err(format!("One file at a time ({s} is a second one)")),
        }
    }
    a.file = file.ok_or("Name a file to transcribe")?;
    Ok(a)
}

/// `silas transcribe ...`, given the arguments after `transcribe`. Returns the exit code.
pub fn cli(args: &[String]) -> i32 {
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) if e.is_empty() => {
            println!("{USAGE}");
            return 0;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return 2;
        }
    };
    match cli_run(&a) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("silas: {e}");
            1
        }
    }
}

fn cli_run(a: &Args) -> anyhow::Result<()> {
    use crate::models;
    use std::io::{IsTerminal, Write};
    if let Some(dir) = &a.data_dir {
        store::set_data_dir(Some(dir.clone()));
    }
    let started = Instant::now();
    // The file first: a file that can't be read shouldn't wait for a model to load.
    let mut decoder = Decoder::open(&a.file)?;
    let settings: store::Settings = store::load_fields("settings.json");
    let model = match &a.model {
        Some(m) => m.clone(),
        None if models::installed(&settings.model) => settings.model.clone(),
        None => models::MODELS
            .iter()
            .find(|m| models::installed(m.id) && models::runs_here(m.id))
            .map(|m| m.id.to_string())
            .ok_or_else(|| anyhow::anyhow!("No speech model is installed. Open Silas once to download one."))?,
    };
    let t = Instant::now();
    let mut engine = models::Engine::load(&model)?;
    let load = t.elapsed();
    let total = decoder.seconds;
    let tty = std::io::stderr().is_terminal() && !a.json;
    let clock = |s: f32| format!("{}:{:02}", s as u32 / 60, s as u32 % 60);
    let outcome = transcribe(
        &mut || decoder.next_block(),
        &mut |audio| engine.transcribe(audio, &settings.language),
        &mut |done| {
            if tty {
                let of = total.map(|t| format!(" of {}", clock(t))).unwrap_or_default();
                eprint!("\rTranscribing: {}{of}   ", clock(done));
                let _ = std::io::stderr().flush();
            }
        },
        &|| false,
    )?;
    if tty {
        eprint!("\r{}\r", " ".repeat(40));
    }
    let text = if a.raw {
        outcome.raw.clone()
    } else {
        let mut dict: crate::dictionary::Dictionary = store::load("dictionary.json");
        dict.apply(&outcome.raw, settings.remove_fillers)
    };
    if a.json {
        let report = json!({
            "file": a.file.display().to_string(),
            "model": model,
            "text": text.trim(),
            "raw": outcome.raw,
            "audioSeconds": outcome.audio_seconds,
            "pieces": outcome.pieces,
            "timings": {
                "loadMs": load.as_millis() as u64,
                "decodeMs": outcome.decode.as_millis() as u64,
                "transcribeMs": outcome.transcribe.as_millis() as u64,
                "totalMs": started.elapsed().as_millis() as u64,
            },
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", text.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn testdata(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/media").join(name)
    }

    /// The tone's pitch, from how often it crosses zero going up.
    fn pitch(audio: &[f32]) -> f32 {
        let ups = audio.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        ups as f32 / (audio.len() as f32 / RATE as f32)
    }

    /// Every file holds one second of a 440 Hz tone, made by src/testdata/media/make.sh.
    #[test]
    fn every_supported_format_decodes_to_16khz_mono() {
        for name in [
            "tone.wav", "tone.mp3", "tone.m4a", "alac.m4a", "tone.flac", "tone.ogg", "tone.opus", "tone.mp4", "tone.mov",
            "tone.webm", "vorbis.webm",
        ] {
            let audio = decode(&testdata(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            let seconds = audio.len() as f32 / RATE as f32;
            // Encoders pad the start and end by a few tens of milliseconds.
            assert!((0.95..1.1).contains(&seconds), "{name}: {seconds} s");
            let middle = &audio[RATE as usize / 4..RATE as usize * 3 / 4];
            let hz = pitch(middle);
            assert!((430.0..450.0).contains(&hz), "{name}: {hz} Hz");
            let rms = (middle.iter().map(|s| s * s).sum::<f32>() / middle.len() as f32).sqrt();
            // Half scale is 0.35 RMS; far off means the channels didn't mix right.
            assert!((0.25..0.45).contains(&rms), "{name}: {rms} RMS");
        }
    }

    #[test]
    fn a_file_that_isnt_media_says_what_silas_reads() {
        let e = decode(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap_err().to_string();
        assert_eq!(e, unreadable_message());
        let e = decode(&testdata("missing.wav")).unwrap_err().to_string();
        assert!(e.contains("Can't open"), "{e}");
    }

    #[test]
    fn a_video_with_no_sound_says_so() {
        let e = decode(&testdata("silent-video.mp4")).unwrap_err().to_string();
        assert_eq!(e, "That file has no sound to transcribe.");
    }

    fn speech(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize).map(|i| 0.2 * (i as f32 * 0.05).sin()).collect()
    }

    fn pause(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize).map(|i| 0.0005 * (i as f32 * 0.7).sin()).collect()
    }

    /// Feeds `audio` a second at a time, as the decoder does.
    fn blocks(audio: Vec<f32>) -> impl FnMut() -> anyhow::Result<Option<Vec<f32>>> {
        let mut at = 0;
        move || {
            if at >= audio.len() {
                return Ok(None);
            }
            let end = (at + RATE as usize).min(audio.len());
            let b = audio[at..end].to_vec();
            at = end;
            Ok(Some(b))
        }
    }

    /// Runs `audio` through `transcribe` with a model that names each piece by its length in seconds.
    fn pieces_of(audio: Vec<f32>) -> (Outcome, Vec<f32>, Vec<f32>) {
        let mut lengths = vec![];
        let mut progress = vec![];
        let outcome = transcribe(
            &mut blocks(audio),
            &mut |a| {
                lengths.push(a.len() as f32 / RATE as f32);
                Ok(format!("piece{}.", lengths.len()))
            },
            &mut |done| progress.push(done),
            &|| false,
        )
        .unwrap();
        (outcome, lengths, progress)
    }

    #[test]
    fn a_long_file_goes_through_in_pieces_cut_at_pauses() {
        // Eight 8-second stretches of speech with a second's pause between them: 75 seconds.
        let mut audio = vec![];
        for _ in 0..8 {
            audio.extend(speech(8.0));
            audio.extend(pause(1.0));
        }
        audio.extend(speech(3.0));
        let total = audio.len() as f32 / RATE as f32;
        let (outcome, lengths, progress) = pieces_of(audio);
        assert!(lengths.iter().all(|l| *l <= MAX_PIECE_SECONDS), "{lengths:?}");
        // Each cut lands mid-pause: 8.5 s, then 9 s each.
        assert!((lengths[0] - 8.5).abs() < 0.1, "{lengths:?}");
        assert!((lengths[1] - 9.0).abs() < 0.1, "{lengths:?}");
        assert_eq!(outcome.pieces, lengths.len());
        assert_eq!(outcome.raw, (1..=lengths.len()).map(|i| format!("piece{i}.")).collect::<Vec<_>>().join(" "));
        assert!((outcome.audio_seconds - total).abs() < 0.01);
        assert!(progress.windows(2).all(|w| w[0] < w[1]), "progress only goes up: {progress:?}");
        assert!((progress.last().unwrap() - total).abs() < 0.01, "and ends at the whole file");
    }

    #[test]
    fn pieces_end_at_the_first_sentence_pause_not_the_longest_one_later() {
        // Sentences of 5 s with 0.6 s between, and one long pause late on. Cutting at the longest
        // pause in a full window made 26 to 30 s pieces, and Parakeet dropped sentences from them.
        let mut audio = vec![];
        for i in 0..12 {
            audio.extend(speech(5.0));
            audio.extend(pause(if i == 4 { 2.0 } else { 0.6 }));
        }
        let (_, lengths, _) = pieces_of(audio);
        assert!(lengths[..lengths.len() - 1].iter().all(|l| (6.0..12.0).contains(l)), "{lengths:?}");
    }

    #[test]
    fn speech_with_no_pause_is_cut_at_its_quietest_moment() {
        // Speech whose loudness swells and fades every 7 s and never stops: no pause for find_cut.
        let audio: Vec<f32> = (0..RATE as usize * 70)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                (0.3 + 0.2 * (t * std::f32::consts::TAU / 7.0).sin()) * (i as f32 * 0.05).sin()
            })
            .collect();
        let (outcome, lengths, _) = pieces_of(audio);
        assert!(lengths.iter().all(|l| *l <= MAX_PIECE_SECONDS), "{lengths:?}");
        assert_eq!(outcome.pieces, lengths.len());
        assert!(lengths[..lengths.len() - 1].iter().all(|l| *l >= MAX_PIECE_SECONDS / 2.0), "{lengths:?}");
        let sum: f32 = lengths.iter().sum();
        assert!((sum - 70.0).abs() < 0.01, "no audio is lost or repeated: {sum}");
    }

    #[test]
    fn silence_is_never_sent_to_the_model() {
        let mut audio = pause(40.0);
        audio.extend(speech(2.0));
        let (outcome, lengths, progress) = pieces_of(audio);
        assert_eq!(lengths.len(), 1, "only the piece with speech: {lengths:?}");
        assert_eq!(outcome.raw, "piece1.");
        assert!((progress.last().unwrap() - 42.0).abs() < 0.01);
        let (outcome, lengths, _) = pieces_of(pause(5.0));
        assert!(lengths.is_empty() && outcome.raw.is_empty());
    }

    #[test]
    fn a_cancel_stops_between_pieces() {
        let mut calls = 0;
        let cancel = AtomicBool::new(false);
        let e = transcribe(
            &mut blocks(speech(100.0)),
            &mut |_| {
                calls += 1;
                cancel.store(true, Ordering::Relaxed);
                Ok("words".into())
            },
            &mut |_| {},
            &|| cancel.load(Ordering::Relaxed),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), crate::core::CANCELLED);
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_model_error_ends_the_file_with_that_error() {
        let e = transcribe(&mut blocks(speech(5.0)), &mut |_| anyhow::bail!("model fell over"), &mut |_| {}, &|| false).unwrap_err();
        assert_eq!(e.to_string(), "model fell over");
    }

    #[test]
    fn the_command_line_reads_its_options() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            parse(&s(&["a.mp3", "--raw", "--json", "--model", "cohere-int4"])),
            Ok(Args { file: "a.mp3".into(), raw: true, json: true, model: Some("cohere-int4".into()), data_dir: None })
        );
        assert_eq!(parse(&s(&["--data-dir", "/d", "b.wav"])).map(|a| a.data_dir), Ok(Some("/d".into())));
        assert_eq!(parse(&s(&["--help"])), Err(String::new()));
        assert!(parse(&s(&[])).unwrap_err().contains("Name a file"));
        assert!(parse(&s(&["a.wav", "b.wav"])).unwrap_err().contains("One file"));
        assert!(parse(&s(&["--model"])).unwrap_err().contains("model id"));
        assert!(parse(&s(&["--fast", "a.wav"])).unwrap_err().contains("--fast"));
        assert_eq!(cli(&s(&["--nope"])), 2);
        assert_eq!(cli(&s(&["/no/such/file.wav"])), 1);
    }

    #[test]
    fn messages_use_no_dashes() {
        for m in [unreadable_message(), busy_message(), USAGE.to_string()] {
            assert!(!m.contains('\u{2014}') && !m.contains('\u{2013}'), "{m}");
        }
    }
}
