//! The running app: owns settings, the dictionary, the loaded model and the
//! record → transcribe → type → learn loop.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Utc};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;

use crate::audio::Recording;
use crate::benchmark::{self, ModelResult, Report, Timed};
use crate::dictionary::{Dictionary, LearnResult};
use crate::keys::{EditWatch, Machine, Out, Trigger, Watch};
use crate::models::{self, Engine};
use crate::store::{self, HistoryItem, Settings, Stats};
use crate::typer;

mod again;
mod shaping;
pub use again::Via;

/// What the interface around the engine provides. The engine never names a UI toolkit:
/// the Tauri app (Windows, Linux, Android) and the native Mac app both implement this.
///
/// Calls may come from any thread, including while the engine holds its own short locks,
/// so an implementation must return quickly and must not call back into [`Core`] methods
/// that take the status lock (`set_status`).
pub trait Host: Send + Sync + 'static {
    /// A named event with a JSON payload: `status`, `transcript`, `learned`, `notice`,
    /// `model-progress`, `benchmark-progress`, `benchmark-done`, `benchmark-error`. A host
    /// with an overlay keeps it up for a `notice` (see [`overlay_shown`]).
    fn emit(&self, event: &str, payload: serde_json::Value);
    /// Microphone level in 0..=1, many times a second while recording.
    fn level(&self, level: f32);
    /// The status changed. `show_overlay` is the user's setting; whether the overlay is up
    /// for this status is [`overlay_status`].
    fn status_changed(&self, status: Status, show_overlay: bool);
    /// Bring the app's main window forward (a model is needed, or setup is incomplete).
    fn show_main(&self);
    /// The key listener came back after the user fixed a permission: refocus the app's
    /// main window if it is showing.
    fn listener_returned(&self);
    /// A fresh install of a nightly build stays on the nightly channel.
    fn is_nightly(&self) -> bool {
        false
    }
    /// The channel this build came from (`store::build_channel`). A fresh install starts on it,
    /// so a nightly or dev build doesn't jump to stable.
    fn build_channel(&self) -> &'static str {
        if self.is_nightly() {
            "nightly"
        } else {
            "stable"
        }
    }
}

/// A host that ignores everything. For tests and headless use.
pub struct NullHost;

impl Host for NullHost {
    fn emit(&self, _: &str, _: serde_json::Value) {}
    fn level(&self, _: f32) {}
    fn status_changed(&self, _: Status, _: bool) {}
    fn show_main(&self) {}
    fn listener_returned(&self) {}
}

/// What `silas-ctl` asks of the running engine (see `control.rs`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    /// The hotkey went down (a short press-and-release starts hands-free).
    Press,
    /// The hotkey came up.
    Release,
    /// The record button.
    Toggle,
    /// Escape while listening.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Loading,
    NeedsModel,
    Ready,
    Recording,
    Transcribing,
    Typing,
    Error,
}

pub struct Core {
    pub host: Arc<dyn Host>,
    /// No settings.json before this start. The model migration can save one first, so the app asks
    /// this, not the file, whether it is the first run.
    pub first_run: bool,
    pub settings: Mutex<Settings>,
    pub dict: Mutex<Dictionary>,
    pub history: Mutex<Vec<HistoryItem>>,
    status: Mutex<(Status, Option<String>)>,
    engine: Mutex<Option<(String, Engine)>>,
    engine_id: Mutex<Option<String>>,
    recording: Mutex<Option<Live>>,
    /// Transcribes a long dictation while it's being spoken.
    pieces: Mutex<Option<crate::pieces::Pieces>>,
    /// Where the keyboard was when the hotkey went down; the text goes back there.
    target: Mutex<Option<crate::focus::Target>>,
    pub machine: Mutex<Machine>,
    /// What the key listener swallows, shared with its event tap. See `sync_blocked`.
    pub blocked: handy_keys::BlockingHotkeys,
    watch: Mutex<Option<EditWatch>>,
    /// True while Silas is typing, so its own keystrokes are not read as the user's.
    pub typing: AtomicBool,
    /// The global key listener is running. Without it the hotkey can't work, so Silas
    /// must never claim to be ready.
    pub listener_ok: AtomicBool,
    /// A modifier key is down, as the key listener last saw it. Linux waits for it before typing.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    mods_held: AtomicBool,
    /// The user pressed keys while Silas was typing, so its copy of the text is off.
    typed_over: AtomicBool,
    /// The user pressed Escape while Silas was typing: stop, and leave the rest in History.
    stop_typing: AtomicBool,
    /// How the key gesture ended while the mic was still opening: `END_*`. Set under the
    /// recording lock, so `start_recording` sees it or the end sees the recording.
    pending_end: std::sync::atomic::AtomicU8,
    /// When the last busy cue played (`busy_cue_due`).
    last_busy: Mutex<Option<Instant>>,
    /// Bumped per recording so the length limit only stops the one it was set for.
    recording_gen: std::sync::atomic::AtomicU64,
    /// Bumped when a press starts from an error, so that error's recovery timer stands down.
    recover_gen: std::sync::atomic::AtomicU64,
    /// A pending hotkey pick: gets the key's name, or why there is none (`CANCELLED`, a refusal).
    pub capture: Mutex<Option<std::sync::mpsc::Sender<Result<String, String>>>>,
    pub last_activity: Mutex<Instant>,
    downloading: Mutex<Vec<String>>,
    /// Downloads the user cancelled; `models::download` checks this between reads.
    cancelled_downloads: Mutex<Vec<String>>,
    /// A benchmark is running; only one at a time.
    bench_running: AtomicBool,
    /// Recordings started so far; a benchmark clip that saw this change overlapped a dictation.
    dictations: AtomicU64,
    /// The model a background load is working on, so one model is never loaded twice at once.
    loading: Mutex<Option<String>>,
    /// False while the interface has paused dictation: keys are ignored, nothing records.
    /// The model stays loaded.
    enabled: AtomicBool,
    /// Dictionary sync (docs/sync.md), once `start_sync` has run.
    pub sync: std::sync::OnceLock<Arc<crate::sync_engine::SyncEngine>>,
    /// Pauses what's playing while a dictation records (nowplaying.rs).
    media: crate::nowplaying::MediaPause,
    /// The id of the app the last dictation went to, for Settings (`shaping.rs`).
    app: Mutex<Option<String>>,
}

pub type Shared = Arc<Core>;

/// The live dictionary as sync sees it: merges happen under the core's lock and are saved and
/// announced like any other change, except that they don't count as the person's edits.
struct CoreDict(std::sync::Weak<Core>);

impl crate::sync_engine::DictStore for CoreDict {
    fn update(&self, f: &mut dyn FnMut(&mut Dictionary) -> bool) {
        let Some(core) = self.0.upgrade() else { return };
        let snapshot = {
            let mut d = core.dict.lock();
            if !f(&mut d) {
                return;
            }
            store::save("dictionary.json", &*d);
            d.clone()
        };
        core.host.emit("dictionary", json!(snapshot));
    }
}

/// Which models a benchmark run measures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scope {
    /// Just the active model.
    Active,
    /// The active model and every other installed one.
    All,
    /// The active model, then every other installed one (nothing is downloaded), so a better
    /// model already on disk is never passed over. Switches only to a pick that passes both bars.
    Auto,
}

/// Whether a registry or community pick for this device moves Silas to `model`. Only while the
/// model is still the default one, and only to a model that is here: never strand someone who
/// has a working model on one they haven't downloaded. With no model installed yet, the
/// maintainer-approved registry may choose what first-run setup downloads; community results
/// never may, since anyone can post them.
fn remote_pick_applies(model_source: &str, source: &str, model_installed: bool, any_installed: bool) -> bool {
    model_source == "default" && (model_installed || (source == "registry" && !any_installed))
}

/// What startup does about this computer's model.
#[derive(Debug, PartialEq)]
enum DevicePlan {
    /// Nothing: this computer's own benchmark settles it, or a saved report says one already ran.
    Keep,
    /// Move to this model, if `remote_pick_applies`, from "registry" or "community".
    Remote(String, &'static str),
    /// Measure the active model once one loads.
    Measure,
}

/// Decide from this computer's current report and the lookups, each called at most once. This
/// computer's own passing pick comes first, as in `best_for_device_from`, and then no lookup runs:
/// neither the registry nor the community overrides a measurement made here. Then the registry,
/// then the community; with no answer, measure unless a report is already saved.
fn plan_device_pick(
    report: Option<&Report>,
    registry: impl FnOnce() -> Option<String>,
    community: impl FnOnce() -> Option<String>,
) -> DevicePlan {
    if report.is_some_and(|r| benchmark::best_for_device_from(Some(r)).is_some()) {
        return DevicePlan::Keep;
    }
    if let Some(model) = registry() {
        return DevicePlan::Remote(model, "registry");
    }
    if let Some(model) = community() {
        return DevicePlan::Remote(model, "community");
    }
    if report.is_some() {
        DevicePlan::Keep
    } else {
        DevicePlan::Measure
    }
}

/// May a model change go ahead? A user's choice is never overridden by an automatic one,
/// unless the model they chose is no longer installed and so there is no choice left to keep.
pub fn allows_model_change(source: &str, auto: bool, chosen_installed: bool) -> bool {
    !(auto && source == "user" && chosen_installed)
}

/// A finished load is stale when another model was chosen while it ran.
fn load_is_stale(current: &str, loaded: &str) -> bool {
    current != loaded
}

/// A load for `id` should start unless one for `id` is already running.
fn should_start_load(loading: Option<&str>, id: &str) -> bool {
    loading != Some(id)
}

/// Frees the benchmark slot when dropped.
pub struct BenchGuard(Shared);

impl Drop for BenchGuard {
    fn drop(&mut self) {
        self.0.bench_running.store(false, Ordering::Release);
    }
}

const MAX_RECORDING: Duration = Duration::from_secs(300);

/// How long an error from a dictation stays up before the status goes back to Ready.
const RECOVER_AFTER: Duration = if cfg!(test) {
    Duration::from_millis(50)
} else {
    Duration::from_secs(4)
};

/// Recording goes on this long after the key comes up, so the last word isn't cut off: people
/// let go of the key as they finish speaking, not after.
pub const TAIL: Duration = Duration::from_millis(200);

/// Busy cues and lines come at most this often, so a held key's repeats don't chime on and on.
const BUSY_EVERY: Duration = Duration::from_secs(1);

/// The text `pc_last_error` and events carry when the user cancelled (a hotkey pick, a download).
pub const CANCELLED: &str = "cancelled";

pub fn key_access_message() -> String {
    key_access_message_for(std::env::consts::OS)
}

/// The macOS text also covers a stale grant: after a build with a different signature, the
/// Accessibility switch can still look on for the old one while this build is not trusted.
/// Turning that switch off and on keeps the old signature, so the text never suggests it:
/// Fix Permission clears the entry, and removing and re-adding Silas by hand does the same.
fn key_access_message_for(os: &str) -> String {
    match os {
        "macos" => "Silas can't hear your key yet. Click Fix Permission on the Silas home screen. If that doesn't work, open System Settings, Privacy & Security, Accessibility, select Silas, click the minus button to remove it, click plus and add Silas from Applications, then turn on its switch.".into(),
        "linux" => "Silas can't read the keyboard. Reinstall the package, or add the udev rule from the README, then log out and back in.".into(),
        _ => "Silas couldn't start its keyboard listener. Restart Silas.".into(),
    }
}

/// Why a press didn't record when the microphone isn't allowed. Onboarding and the home screen
/// have the Allow button.
pub fn mic_message() -> String {
    "Microphone: not allowed yet. Open Silas and click Allow for the microphone.".into()
}

/// Said when a password field has the keyboard as a dictation starts. macOS hides every key
/// from other apps then (secure input), and text typed there would land in a password.
pub fn secure_start_message() -> String {
    "A password field has the keyboard, so Silas isn't listening. Click into another field and try again.".into()
}

/// Said when a password field took the keyboard while the dictation was being written. The text
/// is dropped: it is never typed, kept in History, or learned from.
pub fn secure_finish_message() -> String {
    "A password field has the keyboard, so Silas didn't type what you said and didn't keep it.".into()
}

/// Said after a panic in the speech runtime or the key handling ended a dictation.
pub fn crash_message() -> String {
    "Something went wrong inside Silas, so that dictation stopped. Press your key to try again.".into()
}

/// A short line in the overlay: `kind` is error, busy or stopped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Notice {
    /// Something failed; the status is Error for a few seconds.
    Error,
    /// The press came while Silas was still busy, so it didn't start anything.
    Busy,
    /// The user stopped the typing with Escape.
    Stopped,
}

impl Notice {
    pub fn kind(self) -> &'static str {
        match self {
            Notice::Error => "error",
            Notice::Busy => "busy",
            Notice::Stopped => "stopped",
        }
    }
}

/// What a press of the dictation key does, given where the engine is.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    Start,
    /// Start, taking over an error that was about to clear on its own.
    StartFromError,
    /// No model yet: bring the app forward to choose one.
    ShowMain,
    /// Nothing starts; say why in the overlay, with the error cue.
    Refuse(Notice, String),
}

/// Errors clear on their own after a few seconds; a retry right away isn't ignored while a model
/// is loaded and the key listener works. Every other state says why the press did nothing.
pub fn press(status: Status, detail: Option<&str>, model_loaded: bool, listener_ok: bool) -> Press {
    let busy = |s: &str| Press::Refuse(Notice::Busy, s.to_string());
    match status {
        Status::Ready => Press::Start,
        Status::Error if model_loaded && listener_ok => Press::StartFromError,
        Status::NeedsModel => Press::ShowMain,
        Status::Loading => busy("The speech model is still loading. Try again in a moment."),
        Status::Recording => busy("Still finishing your last dictation."),
        Status::Transcribing => busy("Still writing your last dictation."),
        Status::Typing => busy("Still typing your last dictation. Press Escape to stop it."),
        Status::Error => Press::Refuse(
            Notice::Error,
            detail
                .filter(|d| !d.is_empty())
                .unwrap_or("Dictation can't start right now.")
                .to_string(),
        ),
    }
}

/// What a key the user presses while Silas is typing means.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhileTyping {
    /// Escape: stop typing; the rest of the text is in History.
    Stop,
    /// A key Silas never sends (delete, arrows): the user changed the text, so Silas's copy
    /// of it is off and edits aren't learned from.
    Edit,
    /// Silas's own keystrokes, a key release, or anything else.
    Other,
}

pub fn key_while_typing(ev: &handy_keys::KeyEvent) -> WhileTyping {
    use handy_keys::Key::*;
    if !ev.is_key_down {
        return WhileTyping::Other;
    }
    match ev.key {
        Some(Escape) => WhileTyping::Stop,
        Some(
            Delete | ForwardDelete | LeftArrow | RightArrow | UpArrow | DownArrow | Home | End | PageUp
            | PageDown,
        ) => WhileTyping::Edit,
        _ => WhileTyping::Other,
    }
}

/// Where a finished dictation's text goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Delivery {
    /// Type it, then keep it in History.
    Type,
    /// Focus went somewhere Silas couldn't bring back; typing would hit the wrong window.
    HistoryOnly,
    /// A password field has the keyboard: drop the text.
    Drop,
}

pub fn delivery(secure_input: bool, restored: crate::focus::Restore) -> Delivery {
    if secure_input {
        Delivery::Drop
    } else if restored == crate::focus::Restore::Failed {
        Delivery::HistoryOnly
    } else {
        Delivery::Type
    }
}

/// An edit is learned only if the same window still has the keyboard and it isn't a password
/// field. `then` is None where the system can't say which window that was.
pub fn may_learn(then: Option<u64>, now: Option<u64>, secure_input: bool) -> bool {
    !secure_input && (then.is_none() || then == now)
}

/// How a recording ended, which decides how much more it records.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ending {
    /// The key came up (or the second tap, or a button).
    Key,
    /// The five-minute limit: nobody is mid-word.
    Limit,
}

/// The tail a release records. None when the last moment before key up was already silence
/// (`pieces::ends_quiet`): nobody is mid-word, and the quiet a tail would add is padded on instead.
pub fn tail_for(ending: Ending, quiet: bool) -> Duration {
    match ending {
        Ending::Key if !quiet => TAIL,
        Ending::Key | Ending::Limit => Duration::ZERO,
    }
}

/// A busy cue may play now if none played in the last `BUSY_EVERY`.
fn busy_cue_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|t| now.duration_since(t) >= BUSY_EVERY)
}

/// A dictation that was recording: the microphone, when it opened, and when the key went down.
struct Live {
    rec: Recording,
    started: Instant,
    pressed: Instant,
}

impl Core {
    pub fn new(host: Arc<dyn Host>) -> Shared {
        let mut settings: Settings = store::load_fields("settings.json");
        let fresh = !store::exists("settings.json");
        // An older Silas kept Parakeet v3's ONNX files under the id the GGUF build now has. They
        // move aside as the ONNX fallback, before anything counts or loads models.
        if let Err(e) = models::move_legacy_parakeet() {
            log::error!("models: could not move the old Parakeet v3 files aside: {e}");
        }
        // Settings from before per-device picks have no modelSource. A model other than the
        // default was chosen by the user then, so no benchmark or registry may move them off it.
        if !fresh {
            let installed = models::MODELS
                .iter()
                .filter(|m| models::installed(m.id))
                .count();
            store::migrate_model_source(
                &mut settings,
                &store::read_raw("settings.json"),
                installed,
            );
            store::migrate_hotkey(&mut settings, &store::read_raw("settings.json"));
            store::migrate_language(&mut settings, &store::read_raw("settings.json"));
        }
        // A fresh nightly or dev install stays on its channel instead of jumping to stable.
        if fresh {
            settings.update_channel = host.build_channel().into();
        }
        // Whoever dictated with those files keeps dictating with them; the model list offers the
        // GGUF build as the recommended download.
        if models::retarget_legacy_parakeet(&mut settings) {
            log::info!("models: Parakeet v3's ONNX files are now {}", models::PARAKEET_V3_ONNX);
        }
        // A saved key this build can't use is replaced, not kept: every later settings save sends it
        // back and would be refused for it.
        if Trigger::parse(&settings.hotkey).is_none() {
            log::warn!("settings: {:?} is not a key; using {}", settings.hotkey, store::default_hotkey());
            settings.hotkey = store::default_hotkey().into();
            settings.hotkey_fallback = None;
        }
        let trigger = Trigger::parse(&settings.hotkey).expect("default hotkey parses");
        let mut machine = Machine::new(trigger, settings.tap_to_toggle);
        machine.fallback = settings.hotkey_fallback.as_deref().and_then(Trigger::parse);
        // A saved retype key this build can't use, or one that is now the dictation key, is dropped.
        let again = again::trigger(&settings).unwrap_or_else(|_| {
            settings.retype_hotkey = None;
            None
        });
        machine.again.set(again);
        let mut history: Vec<HistoryItem> = store::load("history.json");
        history.truncate(store::HISTORY_KEEP);
        Arc::new(Core {
            host,
            first_run: fresh,
            dict: Mutex::new(store::load("dictionary.json")),
            history: Mutex::new(history),
            status: Mutex::new((Status::Loading, None)),
            engine: Mutex::new(None),
            engine_id: Mutex::new(None),
            recording: Mutex::new(None),
            pieces: Mutex::new(None),
            target: Mutex::new(None),
            machine: Mutex::new(machine),
            blocked: Default::default(),
            watch: Mutex::new(None),
            typing: AtomicBool::new(false),
            listener_ok: AtomicBool::new(false),
            mods_held: AtomicBool::new(false),
            typed_over: AtomicBool::new(false),
            stop_typing: AtomicBool::new(false),
            pending_end: std::sync::atomic::AtomicU8::new(END_NONE),
            last_busy: Mutex::new(None),
            recording_gen: std::sync::atomic::AtomicU64::new(0),
            recover_gen: std::sync::atomic::AtomicU64::new(0),
            capture: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            downloading: Mutex::new(vec![]),
            cancelled_downloads: Mutex::new(vec![]),
            bench_running: AtomicBool::new(false),
            dictations: AtomicU64::new(0),
            loading: Mutex::new(None),
            enabled: AtomicBool::new(true),
            sync: std::sync::OnceLock::new(),
            media: crate::nowplaying::MediaPause::system(),
            app: Mutex::new(None),
            settings: Mutex::new(settings),
        })
    }

    // ------------------------------------------------------------ status

    pub fn status(&self) -> (Status, Option<String>) {
        self.status.lock().clone()
    }

    /// The key listener started or stopped; refresh the status the user sees.
    pub fn set_listener(&self, ok: bool) {
        // Flip the flag and read the status under the status lock, so a concurrent
        // set_status can't write a stale permission error after the listener came back.
        let refresh = {
            let cur = self.status.lock();
            if self.listener_ok.swap(ok, Ordering::Relaxed) == ok {
                return;
            }
            let missing_key =
                cur.0 == Status::Error && cur.1.as_deref() == Some(self.key_message().as_str());
            (ok && missing_key) || (!ok && cur.0 == Status::Ready)
        };
        if ok {
            // The user just flipped the switch in System Settings; bring them back to Silas.
            self.host.listener_returned();
        }
        if refresh {
            if ok && self.active_model().is_none() {
                self.load_status_after_listener();
            } else {
                // Ready becomes the permission message again when the listener is down.
                self.set_status(Status::Ready, None);
            }
        }
    }

    /// The listener came back before any model was loaded: show what is really pending.
    fn load_status_after_listener(&self) {
        let installed = crate::models::installed(&self.settings.lock().model);
        self.set_status(
            if installed {
                Status::Loading
            } else {
                Status::NeedsModel
            },
            None,
        );
    }

    /// The message for a key listener that can't start.
    fn key_message(&self) -> String {
        key_access_message()
    }

    /// A short line for the overlay. Sent before the status it goes with, so the overlay shows the
    /// line instead of hiding for that status.
    fn notice(&self, n: Notice, message: &str) {
        self.host.emit("notice", json!({ "kind": n.kind(), "message": message }));
    }

    /// A dictation failed: say why in the overlay, with the error cue, set the Error status, and
    /// go back to Ready after a few seconds.
    fn fail(self: &Shared, message: String) {
        self.cue("error");
        self.notice(Notice::Error, &message);
        self.set_status(Status::Error, Some(message));
        self.recover_soon();
    }

    /// Runs `f`; a panic in it becomes an Error with a plain message instead of a dictation
    /// stuck at "Writing…". `key`: the panic came from key handling, so a half-started
    /// recording is dropped too. (The thread that types never touches the recording slot: a
    /// new dictation may own it by then.)
    fn survive(self: &Shared, key: bool, f: impl FnOnce()) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_ok() {
            return;
        }
        log::error!("a panic ended a dictation ({})", if key { "key handling" } else { "writing" });
        self.typing.store(false, Ordering::Relaxed);
        if key {
            self.machine.lock().reset();
            self.media.resume(Duration::ZERO);
            self.recording.lock().take();
            self.pieces.lock().take();
            self.target.lock().take();
        }
        self.fail(crash_message());
    }

    pub fn set_status(&self, s: Status, detail: Option<String>) {
        let (s, detail) = {
            let mut cur = self.status.lock();
            let next = if s == Status::Ready && !self.listener_ok.load(Ordering::Relaxed) {
                (Status::Error, Some(self.key_message()))
            } else {
                (s, detail)
            };
            *cur = next.clone();
            next
        };
        self.host
            .emit("status", json!({ "status": s, "detail": detail }));
        let show_overlay = self.settings.lock().show_overlay;
        self.host.status_changed(s, show_overlay);
    }

    // ------------------------------------------------------------ model

    /// Never waits on a transcription: the status the app reads on its main thread asks this.
    pub fn active_model(&self) -> Option<String> {
        self.engine_id.lock().clone()
    }

    fn set_engine(&self, engine: Option<(String, Engine)>) {
        // Both change under the engine's lock, so a reader never sees an id ahead of what's loaded for long.
        let mut slot = self.engine.lock();
        *self.engine_id.lock() = engine.as_ref().map(|(id, _)| id.clone());
        *slot = engine;
    }

    /// Load the chosen model in the background. Models stay loaded: unloading made the
    /// first dictation after a break wait for a reload in the app Silas replaces.
    pub fn load_model(self: &Shared) {
        let id = self.settings.lock().model.clone();
        if !models::installed(&id) {
            // Fall back to any installed model rather than sitting idle.
            if let Some(m) = models::MODELS.iter().find(|m| models::installed(m.id)) {
                self.settings.lock().model = m.id.to_string();
                return self.load_model();
            }
            self.set_engine(None);
            self.set_status(Status::NeedsModel, None);
            return;
        }
        if self.active_model().as_deref() == Some(id.as_str()) {
            self.set_status(Status::Ready, None);
            return;
        }
        {
            let mut loading = self.loading.lock();
            if !should_start_load(loading.as_deref(), &id) {
                return;
            }
            *loading = Some(id.clone());
        }
        self.set_status(Status::Loading, Some("Loading the speech model".into()));
        let me = self.clone();
        std::thread::spawn(move || {
            let t = Instant::now();
            // A panic in the runtime must not leave the status at Loading for good.
            let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Engine::load(&id).map(|mut engine| {
                    // Warm up so the first real dictation is as fast as the rest.
                    let _ = engine.transcribe(&vec![0.0; 16_000], "en");
                    engine
                })
            }))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the speech runtime crashed")));
            // Done loading: free the slot (unless a newer load has taken it).
            {
                let mut loading = me.loading.lock();
                if loading.as_deref() == Some(id.as_str()) {
                    *loading = None;
                }
            }
            match loaded {
                Ok(engine) => {
                    // Another model may have been chosen while this one loaded. Don't leave the
                    // status at Loading: load whatever is chosen now (a no-op if its own load is
                    // already running and will report Ready).
                    let current = me.settings.lock().model.clone();
                    if load_is_stale(&current, &id) {
                        me.load_model();
                        return;
                    }
                    me.set_engine(Some((id.clone(), engine)));
                    log::info!("loaded {id} in {:.1}s", t.elapsed().as_secs_f32());
                    me.set_status(Status::Ready, None);
                }
                Err(e) => me.set_status(
                    Status::Error,
                    Some(format!("Could not load the model: {e}")),
                ),
            }
        });
    }

    pub fn download_model(self: &Shared, id: &str) -> anyhow::Result<()> {
        {
            let mut d = self.downloading.lock();
            if d.iter().any(|x| x == id) {
                anyhow::bail!("already downloading");
            }
            d.push(id.to_string());
        }
        let host = self.host.clone();
        let idc = id.to_string();
        let cancelled = || self.cancelled_downloads.lock().iter().any(|x| x == id);
        let result = models::download(
            id,
            &|downloaded, total, stage| {
                host.emit(
                    "model-progress",
                    json!({ "id": idc, "downloaded": downloaded, "total": total, "stage": stage }),
                );
            },
            &cancelled,
        );
        let was_cancelled = cancelled();
        self.downloading.lock().retain(|x| x != id);
        self.cancelled_downloads.lock().retain(|x| x != id);
        if was_cancelled {
            self.host.emit("model-progress", json!({ "id": id, "downloaded": 0, "total": 0, "stage": CANCELLED }));
            anyhow::bail!(CANCELLED);
        }
        if let Err(e) = &result {
            self.host.emit("model-progress", json!({ "id": id, "downloaded": 0, "total": 0, "stage": "error", "message": e.to_string() }));
            return result;
        }
        if self.active_model().is_none() {
            self.settings.lock().model = id.to_string();
            self.save_settings();
            self.load_model();
        }
        Ok(())
    }

    /// Stop a download in progress; it ends with `CANCELLED` and leaves nothing behind. False
    /// when that model isn't downloading.
    pub fn cancel_download(&self, id: &str) -> bool {
        if !self.downloading.lock().iter().any(|x| x == id) {
            return false;
        }
        let mut c = self.cancelled_downloads.lock();
        if !c.iter().any(|x| x == id) {
            c.push(id.to_string());
        }
        true
    }

    /// Load this model. With `auto` false the user chose it, and from then on nothing but the
    /// user changes it; setup's own choice (`auto`) leaves the device pick free to apply.
    pub fn select_model(self: &Shared, id: &str, auto: bool) {
        let allowed = {
            let mut s = self.settings.lock();
            let allowed = allows_model_change(&s.model_source, auto, models::installed(&s.model));
            if allowed {
                s.model = id.to_string();
                if !auto {
                    s.model_source = "user".into();
                }
            }
            allowed
        };
        if !allowed {
            // Setup asked for its pick, but the user's own model is here: make sure that one is
            // loaded, so setup never ends with no model.
            self.load_model();
            return;
        }
        self.save_settings();
        self.set_engine(None);
        self.load_model();
    }

    // ------------------------------------------------------------ device pick

    /// At startup, in the background: keep this computer's own benchmark pick if it has a passing
    /// one; else look this computer up in the registry, then in the community results (only a pick
    /// at least `benchmark::MIN_AGREE` senders agree on), and with no answer from either, measure
    /// the active model once.
    pub fn start_device_pick(self: &Shared) {
        let me = self.clone();
        std::thread::spawn(move || me.device_pick());
    }

    fn device_pick(self: &Shared) {
        let key = benchmark::device().key.clone();
        let lookup = benchmark::LookupDone;
        let report = benchmark::load_report();
        let plan = plan_device_pick(
            report.as_ref(),
            || benchmark::fetch_registry().lookup(&key).map(|e| e.model.clone()),
            || benchmark::fetch_community(&key).lookup(&key),
        );
        drop(lookup);
        if let DevicePlan::Remote(model, source) = plan {
            // A device the registry or community knows needs no benchmark.
            let any_installed = models::MODELS.iter().any(|m| models::installed(m.id));
            // (applies, the model changed)
            let (apply, changed) = {
                let mut s = self.settings.lock();
                let apply = remote_pick_applies(&s.model_source, source, models::installed(&model), any_installed);
                let changed = apply && s.model != model;
                if apply {
                    s.model = model.clone();
                    s.model_source = source.into();
                }
                (apply, changed)
            };
            if apply {
                self.save_settings();
                if changed && models::installed(&model) {
                    self.wait_quiet();
                    self.load_model();
                }
            }
            return;
        }
        if plan == DevicePlan::Keep {
            return;
        }
        while self.active_model().is_none() {
            std::thread::sleep(Duration::from_secs(2));
        }
        match self.begin_benchmark() {
            Ok(guard) => {
                if let Err(e) = self.run_benchmark(Scope::Auto, guard) {
                    log::warn!("device benchmark failed: {e}");
                }
            }
            Err(e) => log::info!("device benchmark skipped: {e}"),
        }
    }

    /// Claim the single benchmark slot; it frees itself when the guard drops.
    pub fn begin_benchmark(self: &Shared) -> anyhow::Result<BenchGuard> {
        anyhow::ensure!(
            !self.bench_running.swap(true, Ordering::AcqRel),
            "A benchmark is already running"
        );
        Ok(BenchGuard(self.clone()))
    }

    /// A dictation is under way or a model is loading: not a moment to time anything.
    fn busy(&self) -> bool {
        matches!(
            self.status().0,
            Status::Recording | Status::Transcribing | Status::Typing | Status::Loading
        ) || self.recording.lock().is_some()
    }

    /// A dictation is recording, being written or being typed: a file being transcribed waits (file.rs).
    pub(crate) fn dictating(&self) -> bool {
        matches!(self.status().0, Status::Recording | Status::Transcribing | Status::Typing) || self.recording.lock().is_some()
    }

    /// Run 16 kHz audio through the loaded model, holding it for this audio only (file.rs).
    pub(crate) fn transcribe_audio(&self, audio: &[f32], language: &str) -> anyhow::Result<String> {
        match self.engine.lock().as_mut() {
            Some((_, engine)) => engine.transcribe(audio, language),
            None => anyhow::bail!("No model is loaded"),
        }
    }

    /// Hold until no dictation is happening, so a recording is never delayed by more than
    /// the one clip in flight.
    fn wait_quiet(&self) {
        while self.busy() {
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Hold until nothing is happening and nothing has for `idle`, so loading a second model
    /// next to the live one doesn't land in the middle of someone's dictating.
    fn wait_idle(&self, idle: Duration) {
        while self.busy() || self.last_activity.lock().elapsed() < idle {
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Run one benchmark clip. `run` does the transcribing (holding whatever lock it needs for
    /// that clip only) and returns its own compute time. A clip that overlapped a dictation
    /// is timed against a busy machine, so it runs again.
    fn quiet_clip(&self, run: &mut dyn FnMut() -> Timed) -> Timed {
        loop {
            // Snapshot first, then require quiet: a dictation that starts in between bumps
            // the counter and the clip runs again.
            let before = self.dictations.load(Ordering::Acquire);
            if self.busy() {
                std::thread::sleep(Duration::from_millis(300));
                continue;
            }
            let out = run()?;
            if self.dictations.load(Ordering::Acquire) == before && !self.busy() {
                return Ok(out);
            }
        }
    }

    /// Measure the active model, and with `all` every other installed one. Saves the report,
    /// emits `benchmark-progress` then `benchmark-done` (or `benchmark-error`), and moves to the
    /// pick unless the user chose a model themselves.
    pub fn run_benchmark(
        self: &Shared,
        scope: Scope,
        _guard: BenchGuard,
    ) -> anyhow::Result<Report> {
        let result = self.measure(scope);
        match &result {
            Ok(report) => {
                self.host.emit("benchmark-done", json!(report));
                self.apply_pick(report);
            }
            Err(e) => {
                self.host
                    .emit("benchmark-error", json!({ "message": e.to_string() }));
            }
        }
        result
    }

    fn measure(self: &Shared, scope: Scope) -> anyhow::Result<Report> {
        let active = self
            .active_model()
            .ok_or_else(|| anyhow::anyhow!("No model is loaded yet"))?;
        let mut ids = vec![active.clone()];
        // The other installed models (nothing is downloaded for a benchmark).
        let others: Vec<String> = models::MODELS
            .iter()
            .filter(|m| m.id != active && models::installed(m.id))
            .map(|m| m.id.to_string())
            .collect();
        if scope == Scope::All {
            ids.extend(others.iter().cloned());
        }
        let mut fresh: Vec<ModelResult> = vec![];
        let mut next = 0;
        while next < ids.len() {
            let id = &ids[next];
            next += 1;
            let progress = |clip: usize, clips: usize| {
                self.host.emit(
                    "benchmark-progress",
                    json!({ "model": id, "clip": clip, "clips": clips }),
                );
            };
            let run = if *id == active {
                // The live engine, one clip per lock, so a dictation waits at most one clip.
                let mut clip = |samples: &[f32]| -> anyhow::Result<(String, Duration)> {
                    self.quiet_clip(&mut || {
                        let mut engine = self.engine.lock();
                        match engine.as_mut() {
                            Some((loaded, e)) if loaded == id => {
                                let t = Instant::now();
                                let text = e.transcribe(samples, "en")?;
                                Ok((text, t.elapsed()))
                            }
                            _ => anyhow::bail!("The active model changed during the benchmark"),
                        }
                    })
                };
                benchmark::run_suite(id, &mut clip, &progress)
            } else {
                self.wait_idle(Duration::from_secs(30));
                // A dictation may start during the load; quiet_clip reruns any clip it touches.
                models::Engine::load(id).and_then(|mut engine| {
                    let mut clip = |samples: &[f32]| -> anyhow::Result<(String, Duration)> {
                        self.quiet_clip(&mut || {
                            let t = Instant::now();
                            let text = engine.transcribe(samples, "en")?;
                            Ok((text, t.elapsed()))
                        })
                    };
                    benchmark::run_suite(id, &mut clip, &progress)
                })
            };
            match run {
                Ok(r) => {
                    // Automatic runs measure every installed model (no downloads), so a more
                    // accurate model already on disk is never passed over.
                    if scope == Scope::Auto && *id == active {
                        ids.extend(others.iter().cloned());
                    }
                    fresh.push(r);
                }
                // A model that won't load shouldn't sink the rest; the active one must work.
                Err(e) if *id != active => log::warn!("benchmark of {id} failed: {e}"),
                Err(e) => return Err(e),
            }
        }
        // Keep earlier results for models this run skipped, but only from this version of Silas
        // (load_report already drops another speech set): numbers from another build aren't comparable.
        let mut results: Vec<ModelResult> = benchmark::load_report()
            .filter(|r| r.silas == env!("CARGO_PKG_VERSION"))
            .map(|r| r.results)
            .unwrap_or_default()
            .into_iter()
            .filter(|old| {
                models::spec(&old.model).is_some() && !fresh.iter().any(|f| f.model == old.model)
            })
            .collect();
        results.extend(fresh);
        results.sort_by_key(|r| models::MODELS.iter().position(|m| m.id == r.model));
        let report = Report::new(benchmark::device().clone(), results);
        benchmark::save_report(&report);
        Ok(report)
    }

    /// Move to the benchmark's pick if it is installed, passes both bars, and the user
    /// hasn't chosen. A pick that misses a bar is only shown, never switched to.
    fn apply_pick(self: &Shared, report: &Report) {
        let Some(pick) = report.pick.as_deref() else {
            return;
        };
        let passes = report
            .results
            .iter()
            .any(|r| r.model == pick && benchmark::passes(r));
        if !passes || !models::installed(pick) {
            return;
        }
        let switch = {
            let mut s = self.settings.lock();
            let switch = s.model_source != "user" && s.model != pick;
            if switch {
                s.model = pick.to_string();
                s.model_source = "benchmark".into();
            }
            switch
        };
        if switch {
            self.save_settings();
            self.wait_quiet();
            self.load_model();
        }
    }

    pub fn save_settings(&self) {
        store::save("settings.json", &*self.settings.lock());
    }

    /// Replace the settings with `settings` as the interface sent them. The model and who
    /// chose it stay the core's: a benchmark can switch models while the UI holds an older
    /// copy, and saving that copy must not undo the switch. Updates the hotkey, saves, and
    /// refreshes the status label. Returns the stored settings and the ones they replaced.
    pub fn apply_settings(self: &Shared, settings: Settings) -> Result<(Settings, Settings), String> {
        let mut settings = settings;
        let trigger = Trigger::parse(&settings.hotkey)
            .ok_or_else(|| format!("{} is not a key Silas can use", settings.hotkey))?;
        // Choosing a key ends the second key. Otherwise it's the core's, like the model: the
        // interface's copy may lag. The retype key is checked against this one.
        settings.hotkey_fallback = {
            let cur = self.settings.lock();
            if settings.hotkey == cur.hotkey { cur.hotkey_fallback.clone() } else { None }
        };
        let again = again::trigger(&settings)?;
        settings.update_channel = settings.effective_channel().into();
        let old = {
            let mut cur = self.settings.lock();
            settings.model = cur.model.clone();
            settings.model_source = cur.model_source.clone();
            std::mem::replace(&mut *cur, settings.clone())
        };
        {
            let mut m = self.machine.lock();
            m.trigger = trigger;
            m.fallback = settings.hotkey_fallback.as_deref().and_then(Trigger::parse);
            m.tap_to_toggle = settings.tap_to_toggle;
            m.again.set(again);
        }
        self.sync_blocked();
        self.save_settings();
        if old.hotkey != settings.hotkey && self.status().0 == Status::Ready {
            self.set_status(Status::Ready, None); // refresh the label
        }
        Ok((settings, old))
    }

    /// Pause (`false`) or resume (`true`) dictation. Pausing drops a recording in progress and
    /// ignores the hotkey; it never unloads the model.
    pub fn set_enabled(self: &Shared, on: bool) {
        self.enabled.store(on, Ordering::Release);
        if !on {
            self.machine.lock().reset();
            self.cancel_recording();
        }
        self.sync_blocked();
    }

    /// This keyboard has the microphone key, so the second key has done its job.
    fn retire_fallback(&self) {
        if self.settings.lock().hotkey != "Dictation" {
            return;
        }
        let had = self.settings.lock().hotkey_fallback.take().is_some();
        if had {
            self.machine.lock().fallback = None;
            self.save_settings();
        }
    }

    /// Swallow the dictation key only while it is doing Silas's work. Paused, the key goes
    /// back to the system.
    pub fn sync_blocked(&self) {
        let want = if self.is_enabled() {
            crate::keys::blocked_hotkeys(&self.machine.lock().trigger)
        } else {
            Vec::new()
        };
        if let Ok(mut set) = self.blocked.lock() {
            set.clear();
            set.extend(want);
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    // ------------------------------------------------------------ keys

    /// Every key event from the global listener lands here. A panic in it ends the dictation with
    /// an error instead of taking the listener down.
    pub fn on_key(self: &Shared, ev: &handy_keys::KeyEvent) {
        self.survive(true, || self.handle_key(ev));
    }

    fn handle_key(self: &Shared, ev: &handy_keys::KeyEvent) {
        use handy_keys::Modifiers as M;
        self.mods_held.store(ev.modifiers.intersects(M::CMD | M::CTRL | M::OPT | M::SHIFT), Ordering::Relaxed);
        if !self.is_enabled() {
            // Paused: the hotkey does nothing, but picking a new one still works while a listener runs.
            self.capture_key(ev);
            return;
        }
        if self.typing.load(Ordering::Relaxed) {
            // Silas's own text arrives as key events too, so only keys it never sends
            // (Escape, deleting, moving the cursor) mean the user did something mid-typing.
            match key_while_typing(ev) {
                WhileTyping::Stop => {
                    self.stop_typing.store(true, Ordering::Relaxed);
                    self.typed_over.store(true, Ordering::Relaxed);
                }
                WhileTyping::Edit => self.typed_over.store(true, Ordering::Relaxed),
                WhileTyping::Other => {
                    // The dictation key itself: say why nothing starts.
                    if self.machine.lock().is_trigger_press(ev) {
                        self.refuse(Notice::Busy, "Still typing your last dictation. Press Escape to stop it.");
                    }
                }
            }
            return;
        }
        if self.capture_key(ev) {
            return;
        }
        if ev.is_key_down && ev.key == Some(handy_keys::Key::Dictation) {
            self.retire_fallback();
        }
        let now = Instant::now();
        let out = self.machine.lock().on_event(ev, now);
        if out == Out::Nothing && !self.machine.lock().is_active() {
            self.feed_watch(ev, now);
        }
        self.act(out);
        self.again_key(ev);
    }

    /// A pending "pick the hotkey" request takes this key event. True when it did. Escape
    /// cancels it; a key that types text is refused with a message.
    fn capture_key(&self, ev: &handy_keys::KeyEvent) -> bool {
        // Take the sender in its own statement: an `if let` would keep the lock held.
        let capture = self.capture.lock().take();
        if let Some(tx) = capture {
            match crate::keys::pick(ev) {
                Some(picked) => {
                    let _ = tx.send(picked.into_result());
                }
                None => {
                    // Put it back only if no newer capture has started meanwhile.
                    let mut slot = self.capture.lock();
                    if slot.is_none() {
                        *slot = Some(tx);
                    }
                }
            }
            return true;
        }
        false
    }

    /// End a hotkey pick the user gave up on (the Cancel button).
    pub fn cancel_capture(&self) -> bool {
        let capture = self.capture.lock().take();
        capture.is_some_and(|tx| tx.send(Err(CANCELLED.to_string())).is_ok())
    }

    /// A command from the control socket (`control.rs`), stamped when it arrived so a quick tap
    /// still reads as one when the microphone was slow to open. Paused, it does nothing, like the
    /// hotkey.
    pub fn on_control(self: &Shared, command: Control, at: Instant) {
        self.survive(true, || {
            if !self.is_enabled() {
                return;
            }
            // As with the physical key: nothing starts while the last dictation is being typed.
            if self.typing.load(Ordering::Relaxed) && matches!(command, Control::Press | Control::Toggle) {
                self.refuse(Notice::Busy, "Still typing your last dictation. Press Escape to stop it.");
                return;
            }
            let out = {
                let mut m = self.machine.lock();
                match command {
                    Control::Press => m.external(true, at),
                    Control::Release => m.external(false, at),
                    Control::Toggle => m.toggle_from_ui(),
                    Control::Cancel => m.cancel(),
                }
            };
            self.act(out);
        });
    }

    pub fn act(self: &Shared, out: Out) {
        match out {
            Out::Start => self.start_recording(),
            Out::Finish => self.finish_recording(Ending::Key),
            Out::Cancel => self.cancel_recording(),
            Out::Nothing => {}
        }
    }

    fn feed_watch(self: &Shared, ev: &handy_keys::KeyEvent, now: Instant) {
        let mut guard = self.watch.lock();
        let Some(w) = guard.as_mut() else { return };
        // A password field has the keyboard: nothing typed there is ever learned from.
        if crate::focus::secure_input() {
            *guard = None;
            return;
        }
        let focus = w.focus;
        if let Watch::Done(edit) = w.on_event(ev, now) {
            *guard = None;
            drop(guard);
            self.finish_watch(focus, edit);
        }
    }

    /// Called on a timer so an edit is learned even if the user never presses Return.
    pub fn tick(self: &Shared) {
        let mut guard = self.watch.lock();
        let focus = guard.as_ref().and_then(|w| w.focus);
        if let Some(Watch::Done(edit)) = guard.as_ref().and_then(|w| w.idle(Instant::now())) {
            *guard = None;
            drop(guard);
            self.finish_watch(focus, edit);
        }
    }

    /// Learn the edit only if the same app still has focus: the keys may have gone
    /// somewhere else after a click we can't see.
    fn finish_watch(self: &Shared, focus: Option<u64>, edit: Option<(String, String)>) {
        let Some((typed, fixed)) = edit else { return };
        if !may_learn(focus, crate::focus::focused(), crate::focus::secure_input()) {
            log::info!("focus moved during the edit, or a password field has it; not learning from it");
            return;
        }
        self.learn_from_edit(&typed, &fixed, false, None);
    }

    // ------------------------------------------------------------ dictation

    fn start_recording(self: &Shared) {
        let pressed = Instant::now();
        // A new dictation ends any edit we were following.
        let watching = self.watch.lock().take();
        if let Some(w) = watching {
            if let Some(Watch::Done(edit)) = w.idle(Instant::now() + Duration::from_secs(3600)) {
                self.finish_watch(w.focus, edit);
            }
        }
        let (status, detail) = self.status();
        let loaded = self.active_model().is_some();
        match press(status, detail.as_deref(), loaded, self.listener_ok.load(Ordering::Relaxed)) {
            Press::Start => {}
            Press::StartFromError => {
                // Claim the error from its recovery timer; if the timer got there first it is
                // resetting the key machine right now, so let this press go.
                let g = self.recover_gen.load(Ordering::Relaxed);
                if self
                    .recover_gen
                    .compare_exchange(g, g + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_err()
                {
                    self.machine.lock().reset();
                    return;
                }
            }
            Press::ShowMain => {
                self.machine.lock().reset();
                self.host.show_main();
                return;
            }
            Press::Refuse(n, message) => {
                self.machine.lock().reset();
                self.refuse(n, &message);
                return;
            }
        }
        // macOS hides every key from other apps while a password field has the keyboard, and
        // anything typed would land in the password: never start then.
        if crate::focus::secure_input() {
            self.machine.lock().reset();
            self.fail(secure_start_message());
            return;
        }
        // The hotkey never shows the system mic prompt; only onboarding's Allow asks.
        if !crate::permissions::mic_ready_for_recording() {
            self.machine.lock().reset();
            self.fail(mic_message());
            return;
        }
        // Note where the keyboard is now, at key-down. On Hyprland the overlay can take the
        // keyboard as it opens, so the target is noted before it shows (asking Hyprland takes a
        // few milliseconds); elsewhere the mic doesn't wait for it.
        #[cfg(target_os = "linux")]
        let capture = {
            let target = crate::focus::Target::capture();
            move || target
        };
        #[cfg(not(target_os = "linux"))]
        let capture = {
            let thread = std::thread::spawn(crate::focus::Target::capture);
            move || thread.join().ok().flatten()
        };
        // Music playing out loud would end up in the dictation. Only sends a message; never waits.
        if self.settings.lock().pause_media {
            self.media.pause();
        }
        // The overlay comes up now, before the microphone is open (that can take a moment with
        // a Bluetooth headset); its dot fills in when the first sound arrives.
        self.set_status(Status::Recording, None);
        self.pending_end.store(END_NONE, Ordering::Relaxed);
        let device = self.settings.lock().microphone.clone();
        let host = self.host.clone();
        match Recording::start(device.as_deref(), move |lvl| host.level(lvl)) {
            Ok(r) => {
                // The mic is already running; publish the target before the recording, so a
                // release can never find a recording without its target.
                *self.target.lock() = capture();
                self.dictations.fetch_add(1, Ordering::AcqRel);
                let (raw, rate) = r.tap();
                let me = self.clone();
                let language = self.settings.lock().language.clone();
                *self.pieces.lock() = Some(crate::pieces::Pieces::start(raw, rate, move |audio| {
                    match me.engine.lock().as_mut() {
                        Some((_, engine)) => engine.transcribe(audio, &language),
                        None => anyhow::bail!("No model is loaded"),
                    }
                }));
                {
                    // Escape (or `silas-ctl cancel`) on another thread while the mic was opening
                    // already reset the key machine and found no recording to stop. Checking
                    // under the recording lock means a cancel either sees this recording or
                    // happened before the check.
                    let mut live = self.recording.lock();
                    let active = self.machine.lock().is_active();
                    match after_mic_opened(active, self.pending_end.load(Ordering::Relaxed)) {
                        MicOpened::Keep => *live = Some(Live { rec: r, started: Instant::now(), pressed }),
                        MicOpened::Drop => {
                            drop(live);
                            self.target.lock().take();
                            self.pieces.lock().take();
                            drop(r);
                            self.set_status(Status::Ready, None);
                            return;
                        }
                        // Released before the mic opened (a slow Bluetooth headset): the
                        // release found nothing to finish, so finish what was heard now.
                        MicOpened::Finish => {
                            *live = Some(Live { rec: r, started: Instant::now(), pressed });
                            drop(live);
                            self.pending_end.store(END_NONE, Ordering::Relaxed);
                            self.finish_recording(Ending::Key);
                            return;
                        }
                    }
                }
                let gen = self.recording_gen.fetch_add(1, Ordering::Relaxed) + 1;
                self.cue("start");
                // Hands-free mode left on by mistake would grow without end; stop at 5 minutes.
                let me = self.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(MAX_RECORDING);
                    if me.recording_gen.load(Ordering::Relaxed) == gen
                        && me.recording.lock().is_some()
                    {
                        me.machine.lock().reset();
                        me.finish_recording(Ending::Limit);
                    }
                });
            }
            Err(e) => {
                let _ = capture();
                self.media.resume(Duration::ZERO);
                self.machine.lock().reset();
                self.fail(format!("Microphone: {e}"));
            }
        }
    }

    /// A press that starts nothing: the error cue and a line saying why. At most one a second,
    /// so a held key's repeats don't chime on.
    fn refuse(self: &Shared, n: Notice, message: &str) {
        let due = {
            let mut last = self.last_busy.lock();
            let now = Instant::now();
            let due = busy_cue_due(*last, now);
            if due {
                *last = Some(now);
            }
            due
        };
        if due {
            self.cue("error");
            self.notice(n, message);
        }
    }

    fn cancel_recording(self: &Shared) {
        // Release the lock before set_status, which waits on the main thread for the tray.
        let recording = {
            let mut live = self.recording.lock();
            if live.is_none() {
                self.pending_end.store(END_CANCEL, Ordering::Relaxed);
            }
            live.take()
        };
        self.target.lock().take();
        self.pieces.lock().take();
        self.media.resume(Duration::ZERO);
        if let Some(live) = recording {
            drop(live);
            self.set_status(Status::Ready, None);
        }
    }

    fn finish_recording(self: &Shared, ending: Ending) {
        let recording = {
            let mut live = self.recording.lock();
            if live.is_none() {
                // A release while the mic is still opening: start_recording finishes it.
                self.pending_end.store(END_FINISH, Ordering::Relaxed);
            }
            live.take()
        };
        let Some(live) = recording else {
            return;
        };
        self.cue("stop");
        // Once the tail is recorded, what was playing plays again. Whether the tail is skipped is
        // known only once write() reads the buffer, so wait out the longest one: media never
        // resumes into a tail that is still recording.
        self.media.resume(tail_for(ending, false));
        // Carried by value: a new dictation may start before this one finishes typing.
        let target = self.target.lock().take();
        let pieces = self.pieces.lock().take();
        let me = self.clone();
        std::thread::spawn(move || {
            let guard = me.clone();
            guard.survive(false, move || me.write(live, ending, target, pieces));
        });
    }

    /// Transcribes, types and keeps a finished recording. Runs on its own thread.
    fn write(
        self: &Shared,
        live: Live,
        ending: Ending,
        target: Option<crate::focus::Target>,
        pieces: Option<crate::pieces::Pieces>,
    ) {
        let me = self;
        let Live { rec, started, pressed } = live;
        let released = Instant::now();
        if let Some(p) = &pieces {
            p.key_up();
        }
        // A short tail first, so the last word isn't cut off; after that, what's said isn't
        // dictation. If the speaker had already stopped, there is no last word to wait for.
        let quiet = ending == Ending::Key && {
            let (raw, rate) = rec.tap();
            let buffer = raw.lock();
            crate::pieces::ends_quiet(&buffer, rate)
        };
        let tail = tail_for(ending, quiet);
        std::thread::sleep(tail);
        let mut rec = rec;
        rec.stop();
        // How long the microphone took to give its first sound after the key went down: words
        // said in that gap are lost.
        let mic_gap_ms = rec.first_sample().map(|t| t.saturating_duration_since(pressed).as_millis() as u64);
        if let Some(ms) = mic_gap_ms {
            log::info!("key down to first sample: {ms} ms");
        }
        let secs = rec.seconds();
        if secs - tail.as_secs_f32() < 0.35 {
            drop((pieces, rec));
            me.set_status(Status::Ready, None);
            return;
        }
        me.set_status(Status::Transcribing, None);
        let done = pieces.map(|p| p.finish()).unwrap_or_default();
        let mut rest = rec.finish_from(done.raw_samples);
        if quiet {
            crate::pieces::pad_quiet(&mut rest);
        }
        let language = me.settings.lock().language.clone();
        let raw = match crate::pieces::transcript(done, &rest, |audio| {
            let mut engine = me.engine.lock();
            match engine.as_mut() {
                Some((_, e)) => e.transcribe(audio, &language),
                None => Err(anyhow::anyhow!("No model is loaded")),
            }
        }) {
            Ok(t) => t,
            Err(e) => {
                me.fail(format!("Transcription failed: {e}"));
                return;
            }
        };
        let s = me.settings.lock().clone();
        let text = me.dict.lock().apply(&raw, s.remove_fillers);
        // The app's style, spoken code and snippets (`core/shaping.rs`).
        let (mut text, s) = me.shape(target.as_ref(), &text, s);
        if nothing_to_type(&text) {
            me.set_status(Status::Ready, None);
            return;
        }
        if s.trailing_space && !text.ends_with(char::is_whitespace) {
            text.push(' ');
        }
        use crate::focus::Restore;
        let restored = target.as_ref().map_or(Restore::Elsewhere, |t| t.restore());
        let latency = released.elapsed().as_millis() as u64;
        // Typing into the window that stole focus is the bug; keep the text in History. A
        // password field gets nothing, and the text isn't kept anywhere.
        let mut lost_focus = match delivery(crate::focus::secure_input(), restored) {
            Delivery::Drop => {
                me.fail(secure_finish_message());
                return;
            }
            Delivery::HistoryOnly => true,
            Delivery::Type => false,
        };
        let mut typing_error: Option<String> = None;
        let mut stopped = false;
        // Guard the typing only when Silas knows where the text belongs.
        let guarded = matches!(
            restored,
            Restore::InPlace | Restore::Restored | Restore::Elsewhere
        );
        let still_here = |after_focus_key: bool| {
            !me.stop_typing.load(Ordering::Relaxed)
                && (!guarded
                    || target
                        .as_ref()
                        .is_none_or(|t| t.still_here(after_focus_key)))
        };
        // Keys typed while the user still holds Super or Ctrl from a keybinding would reach the
        // compositor as shortcuts, so wait for the keys to come up, or don't type.
        #[cfg(target_os = "linux")]
        if !lost_focus && me.listener_ok.load(Ordering::Relaxed) {
            let until = Instant::now() + Duration::from_secs(3);
            while me.mods_held.load(Ordering::Relaxed) && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            if me.mods_held.load(Ordering::Relaxed) {
                typing_error = Some("Let go of Ctrl, Alt, Shift and Super so Silas can type".into());
            }
        }
        if !lost_focus && typing_error.is_none() {
            me.set_status(Status::Typing, None);
            me.typed_over.store(false, Ordering::Relaxed);
            me.stop_typing.store(false, Ordering::Relaxed);
            me.typing.store(true, Ordering::Relaxed);
            let typed = typer::type_text(
                &text,
                typer::Pace::from_setting(&s.typing_speed),
                s.newline_as_shift_enter,
                &still_here,
            );
            // Let the last synthetic events drain before reading the keyboard again.
            std::thread::sleep(Duration::from_millis(60));
            me.typing.store(false, Ordering::Relaxed);
            stopped = me.stop_typing.swap(false, Ordering::Relaxed);
            match typed {
                Ok(finished) => lost_focus = !finished && !stopped,
                Err(e) => typing_error = Some(e.to_string()),
            }
        }
        let typed_over = me.typed_over.load(Ordering::Relaxed);
        if !lost_focus
            && !stopped
            && typing_error.is_none()
            && s.learn_from_edits
            && !typed_over
            && crate::focus::us_layout()
        {
            *me.watch.lock() = Some(EditWatch::new(
                &text,
                Instant::now(),
                crate::focus::focused(),
            ));
        }
        let item = HistoryItem {
            id: Utc::now().timestamp_millis().to_string(),
            ts: Utc::now().to_rfc3339(),
            words: text.split_whitespace().count(),
            text: text.trim_end().to_string(),
            raw,
            duration_s: started
                .elapsed()
                .as_secs_f32()
                .min(secs.max(0.0) + 1.0)
                .max(secs),
            latency_ms: latency,
            corrected: None,
            file: None,
        };
        {
            let mut h = me.history.lock();
            h.insert(0, item.clone());
            h.truncate(store::HISTORY_KEEP);
            store::save("history.json", &*h);
        }
        me.save_dictionary();
        *me.last_activity.lock() = Instant::now();
        // `typed` is true only when the text really went out as keystrokes: the app moves Silas to the Trash
        // after the first one, never after a dictation that only reached History.
        let mut payload = json!(&item);
        payload["typed"] = json!(!lost_focus && !stopped && typing_error.is_none());
        payload["micGapMs"] = json!(mic_gap_ms);
        me.host.emit("transcript", payload);
        if let Some(e) = typing_error {
            me.fail(format!("{e}. Your full text is in History."));
        } else if lost_focus {
            me.fail("Another window took the keyboard, so Silas stopped typing. Your full text is in History.".into());
        } else if stopped {
            me.notice(Notice::Stopped, "Stopped typing. Your full text is in History.");
            me.set_status(Status::Ready, None);
        } else {
            me.set_status(Status::Ready, None);
        }
    }

    fn recover_soon(self: &Shared) {
        let me = self.clone();
        let gen = self.recover_gen.load(Ordering::Relaxed);
        std::thread::spawn(move || {
            std::thread::sleep(RECOVER_AFTER);
            if me.status().0 == Status::Error
                && me
                    .recover_gen
                    .compare_exchange(gen, gen + 1, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            {
                me.machine.lock().reset();
                if me.active_model().is_some() {
                    me.set_status(Status::Ready, None);
                } else {
                    me.load_model();
                }
            }
        });
    }

    fn cue(&self, which: &str) {
        // Tests never make a sound.
        if cfg!(test) || !self.settings.lock().sounds {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let sound = match which {
                "start" => "Tink",
                "error" => "Basso",
                _ => "Pop",
            };
            let _ = std::process::Command::new("afplay")
                .args([
                    "-v",
                    "0.25",
                    &format!("/System/Library/Sounds/{sound}.aiff"),
                ])
                .spawn();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = which;
    }

    // ------------------------------------------------------------ dictionary and sync

    /// Save the dictionary and let sync know it changed. Every edit goes through here (or calls
    /// [`Core::dictionary_changed`] after saving under its own lock).
    pub fn save_dictionary(&self) {
        store::save("dictionary.json", &*self.dict.lock());
        self.dictionary_changed();
    }

    /// The person changed the dictionary. Call with the dictionary lock released.
    pub fn dictionary_changed(&self) {
        if let Some(s) = self.sync.get() {
            s.local_changed();
        }
    }

    /// Start dictionary sync for this data folder, with the key in the OS keychain. A merge that
    /// changes the dictionary replaces it in place, so the next dictation uses it, and the
    /// interface gets a `dictionary` event.
    pub fn start_sync(self: &Shared) -> Arc<crate::sync_engine::SyncEngine> {
        use crate::sync_engine::{Options, SyncEngine};
        let engine = self.sync.get_or_init(|| {
            let dir = store::data_dir();
            let keys = Arc::new(crate::sync_key::OsKeyStore::new(&dir));
            let dict = Arc::new(CoreDict(Arc::downgrade(self)));
            let mut o = Options::new(dir, keys, dict);
            let host = self.host.clone();
            o.emit = Arc::new(move |event, payload| host.emit(event, payload));
            SyncEngine::new(o)
        });
        engine.start();
        engine.clone()
    }

    // ------------------------------------------------------------ learning

    pub fn learn_from_edit(
        &self,
        typed: &str,
        fixed: &str,
        explicit: bool,
        history_id: Option<&str>,
    ) -> LearnResult {
        let result = self.dict.lock().learn(typed, fixed, explicit);
        self.save_dictionary();
        if let Some(id) = history_id {
            let mut h = self.history.lock();
            if let Some(item) = h.iter_mut().find(|i| i.id == id) {
                item.corrected = Some(fixed.to_string());
            }
            store::save("history.json", &*h);
        }
        // Fixes taught in History show their result in place; only edits made in other
        // apps need the toast.
        if !result.is_empty() && !explicit {
            log::info!("learned {:?}", result);
            self.host.emit("learned", json!(&result));
        }
        result
    }

    pub fn stats(&self) -> Stats {
        let h = self.history.lock();
        let today = Local::now().date_naive();
        let mut s = Stats::default();
        let mut speak_secs = 0f32;
        // Text from a file wasn't dictated: it would swamp the speaking pace and time saved.
        for i in h.iter().filter(|i| i.file.is_none()) {
            s.dictations += 1;
            s.words_total += i.words as u64;
            speak_secs += i.duration_s;
            if chrono::DateTime::parse_from_rfc3339(&i.ts)
                .map(|d| d.with_timezone(&Local).date_naive() == today)
                .unwrap_or(false)
            {
                s.words_today += i.words as u64;
            }
        }
        if speak_secs > 0.0 {
            s.avg_wpm = s.words_total as f32 / (speak_secs / 60.0);
        }
        // Typing at 40 words a minute versus speaking it.
        s.minutes_saved = (s.words_total as f32 / 40.0 - speak_secs / 60.0).max(0.0);
        s.learned = self
            .dict
            .lock()
            .corrections
            .iter()
            .filter(|c| c.source == "learned")
            .count() as u64;
        s
    }
}

/// Whether the overlay is up for this status.
pub fn overlay_status(s: Status) -> bool {
    matches!(s, Status::Recording | Status::Transcribing | Status::Typing)
}

/// How long a notice keeps the overlay up: about as long as the error it explains.
pub const NOTICE_FOR: Duration = Duration::from_secs(4);

/// Whether the overlay is up. A dictation's statuses show it while the setting is on. A notice
/// shows it until `notice_until` whatever the setting and the status, since a failure nobody
/// sees reads as Silas doing nothing.
pub fn overlay_shown(status: Status, enabled: bool, notice_until: Option<Instant>, now: Instant) -> bool {
    (enabled && overlay_status(status)) || notice_until.is_some_and(|until| now < until)
}

const END_NONE: u8 = 0;
const END_FINISH: u8 = 1;
const END_CANCEL: u8 = 2;

#[derive(Debug, PartialEq)]
enum MicOpened {
    Keep,
    Finish,
    Drop,
}

/// What to do with a mic that just opened. `active`: the key gesture is still going. `pending`:
/// how it ended if the end already ran and found no recording. An end that hasn't run yet will
/// find the recording kept here.
fn after_mic_opened(active: bool, pending: u8) -> MicOpened {
    match (active, pending) {
        (true, _) | (false, END_NONE) => MicOpened::Keep,
        (false, END_FINISH) => MicOpened::Finish,
        _ => MicOpened::Drop,
    }
}

/// An empty or blank result types nothing, but a spoken "new line" alone is a line break to type.
fn nothing_to_type(text: &str) -> bool {
    text.trim().is_empty() && !text.contains('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review first-F2 on #56: a release before a slow mic opened threw the dictation away,
    /// the same as a cancel.
    #[test]
    fn a_release_before_the_mic_opens_still_finishes_and_a_cancel_drops() {
        assert_eq!(after_mic_opened(false, END_FINISH), MicOpened::Finish);
        assert_eq!(after_mic_opened(false, END_CANCEL), MicOpened::Drop);
        // The end hasn't run yet: keep it, and the end takes it.
        assert_eq!(after_mic_opened(false, END_NONE), MicOpened::Keep);
        assert_eq!(after_mic_opened(true, END_NONE), MicOpened::Keep);
    }

    /// Review first-F1 on #56: `silas-ctl press` while typing went on to start a recording
    /// instead of saying why nothing starts, as the physical key does.
    #[test]
    fn a_control_press_while_typing_is_refused_as_busy() {
        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("ctl-typing");
        core.typing.store(true, Ordering::Relaxed);
        core.on_control(Control::Press, Instant::now());
        assert_eq!(
            seen.notices(),
            vec![("busy".to_string(), "Still typing your last dictation. Press Escape to stop it.".to_string())]
        );
        assert!(!core.machine.lock().is_active());
        assert!(core.recording.lock().is_none());
        done(dir);
    }

    /// Review sol-F2 on #23: an error's notice went out while its status hid the overlay, and
    /// with the main window hidden nothing showed it.
    #[test]
    fn a_spoken_line_break_alone_is_still_typed() {
        let none: &[crate::snippets::Snippet] = &[];
        let shaped = crate::style::render("new line", crate::style::Style::Code, none);
        assert_eq!(shaped, "\n");
        assert!(!nothing_to_type(&shaped));
        assert!(nothing_to_type(""));
        assert!(nothing_to_type("  "));
    }

    #[test]
    fn a_notice_keeps_the_overlay_up_until_it_runs_out() {
        let now = Instant::now();
        let until = Some(now + NOTICE_FOR);
        assert!(overlay_shown(Status::Error, true, until, now));
        // The overlay setting is off and the main window is hidden: the notice still shows.
        assert!(overlay_shown(Status::Error, false, until, now));
        assert!(overlay_shown(Status::Ready, false, until, now + NOTICE_FOR / 2));
        assert!(!overlay_shown(Status::Error, true, until, now + NOTICE_FOR));
        assert!(!overlay_shown(Status::Error, true, None, now));
        // Without a notice, as before: the setting and the status decide.
        assert!(overlay_shown(Status::Recording, true, None, now));
        assert!(!overlay_shown(Status::Recording, false, None, now));
        assert!(overlay_shown(Status::Typing, true, Some(now), now));
    }

    #[test]
    fn the_mac_key_message_explains_a_stale_grant() {
        let m = key_access_message_for("macos");
        // home.ts finds the banner by this phrase.
        assert!(m.contains("can't hear your key"));
        assert!(m.contains("Privacy & Security, Accessibility"));
        assert!(m.contains("Fix Permission"));
        assert!(m.contains("click the minus button to remove it, click plus and add Silas"));
        // Off and on keeps the stale code requirement; never suggest it.
        let lower = m.to_lowercase();
        for banned in ["toggle", "off and on", "on and off"] {
            assert!(!lower.contains(banned), "{m}");
        }
        assert!(!m.contains(['\u{2014}', '\u{2013}', '\u{203a}']), "{m}");
    }

    use crate::store::TEST_DIR as STORE_LOCK;

    #[test]
    fn the_microphone_key_is_swallowed_only_while_it_is_ours() {
        use handy_keys::{Key, Modifiers};
        let dir = std::env::temp_dir().join(format!("pc-blocked-{}", std::process::id()));
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        let core = Core::new(Arc::new(NullHost));
        let holds = |core: &Shared| {
            core.blocked
                .lock()
                .unwrap()
                .iter()
                .any(|h| h.key == Some(Key::Dictation) && h.modifiers == Modifiers::FN)
        };
        let mut s = core.settings.lock().clone();
        s.hotkey = "Dictation".into();
        core.apply_settings(s.clone()).unwrap();
        assert!(holds(&core), "chosen: Silas takes the press");
        core.set_enabled(false);
        assert!(!holds(&core), "paused: the key goes back to the system");
        core.set_enabled(true);
        assert!(holds(&core), "resumed");
        s.hotkey = "CmdRight".into();
        core.apply_settings(s).unwrap();
        assert!(!holds(&core), "another key: nothing is swallowed");
        assert!(core.blocked.lock().unwrap().is_empty());
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_second_key_retires_on_the_first_press_of_the_microphone_key() {
        use handy_keys::{Key, KeyEvent, Modifiers};
        let dir = std::env::temp_dir().join(format!("pc-retire-{}", std::process::id()));
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        let core = Core::new(Arc::new(NullHost));
        let mut s = core.settings.lock().clone();
        s.hotkey = "Dictation".into();
        s.hotkey_fallback = Some("CmdRight".into());
        *core.settings.lock() = s;
        core.machine.lock().fallback = Trigger::parse("CmdRight");
        core.on_key(&KeyEvent { modifiers: Modifiers::FN, key: Some(Key::Dictation), is_key_down: true, changed_modifier: None });
        assert_eq!(core.settings.lock().hotkey_fallback, None);
        assert_eq!(core.machine.lock().fallback, None);
        let saved: Settings = store::load("settings.json");
        assert_eq!(saved.hotkey_fallback, None, "it stays gone after a restart");
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A hotkey this build can't parse (a hand edit, a newer build's key) used to stay in the settings: the engine
    /// fell back to the default, but the interface sends the saved one with every change, so every switch failed
    /// to save with "Banana is not a key Silas can use".
    #[test]
    fn a_saved_key_that_isnt_one_never_blocks_a_settings_change() {
        let dir = std::env::temp_dir().join(format!("pc-badkey-{}", std::process::id()));
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        std::fs::write(store::data_dir().join("settings.json"), r#"{"hotkey":"Banana","onboarded":true,"hotkeyVersion":2}"#).unwrap();
        let core = Core::new(Arc::new(NullHost));
        let mut s = core.settings.lock().clone();
        assert_eq!(s.hotkey, store::default_hotkey());
        s.sounds = !s.sounds;
        let saved = core.apply_settings(s.clone());
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(saved.map(|(now, _)| now.sounds), Ok(s.sounds));
    }

    #[test]
    fn a_paused_engine_still_takes_the_key_being_picked() {
        use handy_keys::{Key, KeyEvent, Modifiers};
        let dir = std::env::temp_dir().join(format!("pc-capture-{}", std::process::id()));
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        let core = Core::new(Arc::new(NullHost));
        core.set_enabled(false);
        let (tx, rx) = std::sync::mpsc::channel();
        *core.capture.lock() = Some(tx);
        core.on_key(&KeyEvent {
            modifiers: Modifiers::empty(),
            key: Some(Key::F13),
            is_key_down: true,
            changed_modifier: None,
        });
        assert_eq!(rx.try_recv(), Ok(Ok("F13".to_string())));
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Parakeet v3 move saves settings.json on a first start too. The fresh-install check ran after that save,
    /// saw a file and left a nightly build on stable.
    #[test]
    fn a_fresh_nightly_install_with_old_onnx_files_stays_on_nightly() {
        struct Nightly;
        impl Host for Nightly {
            fn emit(&self, _: &str, _: serde_json::Value) {}
            fn level(&self, _: f32) {}
            fn status_changed(&self, _: Status, _: bool) {}
            fn show_main(&self) {}
            fn listener_returned(&self) {}
            fn is_nightly(&self) -> bool {
                true
            }
        }
        let dir = std::env::temp_dir().join(format!("pc-fresh-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        let old = store::models_dir().join("parakeet-v3");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join(".complete"), b"ok").unwrap();
        let core = Core::new(Arc::new(Nightly));
        assert!(core.first_run && store::exists("settings.json"), "src-tauri's first-run setup reads first_run");
        let (live, model) = { let s = core.settings.lock(); (s.update_channel.clone(), s.model.clone()) };
        let saved: Settings = store::load("settings.json");
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(model, models::PARAKEET_V3_ONNX);
        assert_eq!((live.as_str(), saved.update_channel.as_str()), ("nightly", "nightly"));
    }

    /// A start that moved the ONNX files and quit before saving settings: the next start moves nothing, and used
    /// to leave the user on a GGUF model they don't have.
    #[test]
    fn a_restart_after_an_unsaved_move_finishes_it() {
        let dir = std::env::temp_dir().join(format!("pc-unsaved-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _store = STORE_LOCK.lock();
        store::set_data_dir(Some(dir.clone()));
        let onnx = models::model_path(models::PARAKEET_V3_ONNX);
        std::fs::create_dir_all(&onnx).unwrap();
        std::fs::write(onnx.join(".complete"), b"ok").unwrap();
        std::fs::write(onnx.join(".migrated"), b"").unwrap(); // left by the move
        std::fs::write(store::data_dir().join("settings.json"), r#"{"model":"parakeet-v3","onboarded":true}"#).unwrap();
        let core = Core::new(Arc::new(NullHost));
        let live = core.settings.lock().model.clone();
        let saved: Settings = store::load("settings.json");
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!((live.as_str(), saved.model.as_str()), (models::PARAKEET_V3_ONNX, models::PARAKEET_V3_ONNX));
    }

    /// Records what the engine tells the interface.
    #[derive(Default)]
    struct Seen(Mutex<Vec<(String, serde_json::Value)>>);

    impl Host for Seen {
        fn emit(&self, e: &str, p: serde_json::Value) {
            self.0.lock().push((e.to_string(), p));
        }
        fn level(&self, _: f32) {}
        fn status_changed(&self, _: Status, _: bool) {}
        fn show_main(&self) {}
        fn listener_returned(&self) {}
    }

    impl Seen {
        fn notices(&self) -> Vec<(String, String)> {
            self.0
                .lock()
                .iter()
                .filter(|(e, _)| e == "notice")
                .map(|(_, p)| (p["kind"].as_str().unwrap().to_string(), p["message"].as_str().unwrap().to_string()))
                .collect()
        }
    }

    /// A core over a temporary data folder, with `Seen` as its host. Nothing here opens the
    /// microphone: every dictation below starts from a state that refuses before it.
    fn seen_core(name: &str) -> (Shared, Arc<Seen>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pc-{name}-{}", std::process::id()));
        store::set_data_dir(Some(dir.clone()));
        let seen = Arc::new(Seen::default());
        let core = Core::new(seen.clone());
        (core, seen, dir)
    }

    /// Waits for the recovery timer to clear an error. A fixed sleep of a few RECOVER_AFTERs failed on a slow
    /// Windows runner (2026-11-18), so this polls with a deadline far past the timer instead.
    fn cleared(core: &Shared) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if core.status().0 != Status::Error {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn done(dir: std::path::PathBuf) {
        // Let a recovery timer (`RECOVER_AFTER`, short in tests) finish inside this folder.
        std::thread::sleep(Duration::from_millis(250));
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn key_down(k: handy_keys::Key) -> handy_keys::KeyEvent {
        handy_keys::KeyEvent { modifiers: handy_keys::Modifiers::empty(), key: Some(k), is_key_down: true, changed_modifier: None }
    }

    #[test]
    fn messages_send_you_to_the_silas_app_and_use_no_dashes() {
        assert!(mic_message().contains("Open Silas and click Allow"));
        let mut all = vec![mic_message(), secure_start_message(), secure_finish_message(), crash_message()];
        all.extend(["macos", "linux", "windows"].map(key_access_message_for));
        for m in all {
            assert!(!m.contains(['\u{2014}', '\u{2013}']), "{m}");
        }
    }

    #[test]
    fn a_press_while_busy_says_why_instead_of_doing_nothing() {
        for (st, says) in [
            (Status::Loading, "still loading"),
            (Status::Recording, "finishing"),
            (Status::Transcribing, "writing"),
            (Status::Typing, "Escape"),
        ] {
            match press(st, None, true, true) {
                Press::Refuse(Notice::Busy, m) => assert!(m.contains(says), "{st:?}: {m}"),
                other => panic!("{st:?}: {other:?}"),
            }
        }
        assert_eq!(press(Status::Ready, None, true, true), Press::Start);
        assert_eq!(press(Status::Error, Some("x"), true, true), Press::StartFromError);
        assert_eq!(press(Status::NeedsModel, None, false, true), Press::ShowMain);
        // An error that can't be retried yet says what it is again.
        assert_eq!(press(Status::Error, Some("No model"), false, true), Press::Refuse(Notice::Error, "No model".into()));

        // Through the engine: a busy press leaves one line for the overlay and changes nothing.
        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("busy");
        core.set_status(Status::Transcribing, None);
        core.start_recording();
        assert_eq!(seen.notices(), vec![("busy".to_string(), "Still writing your last dictation.".to_string())]);
        assert_eq!(core.status().0, Status::Transcribing);
        assert!(!core.machine.lock().is_active(), "the key machine starts over");
        // A held key's repeats don't chime again within the second.
        core.start_recording();
        assert_eq!(seen.notices().len(), 1);
        done(dir);
    }

    #[test]
    fn busy_cues_come_at_most_once_a_second() {
        let t = Instant::now();
        assert!(busy_cue_due(None, t));
        assert!(!busy_cue_due(Some(t), t + Duration::from_millis(400)));
        assert!(busy_cue_due(Some(t), t + BUSY_EVERY));
    }

    #[test]
    fn a_sync_merge_replaces_the_live_dictionary_and_says_so() {
        use crate::sync_engine::DictStore;
        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("sync-merge");
        // Sync was never set up here, so starting it reads no keychain and opens no port.
        let engine = core.start_sync();
        assert!(!engine.status().enabled && engine.status().port.is_none());
        CoreDict(Arc::downgrade(&core)).update(&mut |d| {
            d.add_term("Parish1", "user");
            true
        });
        // The next dictation uses it, the file has it, and the window hears about it.
        assert_eq!(core.dict.lock().apply("parish1 is up", false), "Parish1 is up");
        assert!(store::read_raw("dictionary.json").contains("Parish1"));
        let events = seen.0.lock();
        let (_, payload) = events.iter().find(|(e, _)| e == "dictionary").expect("a dictionary event");
        assert_eq!(payload["terms"][0]["text"], "Parish1");
        drop(events);
        // A merge that changes nothing writes nothing and says nothing.
        let before = seen.0.lock().len();
        CoreDict(Arc::downgrade(&core)).update(&mut |_| false);
        assert_eq!(seen.0.lock().len(), before);
        done(dir);
    }

    #[test]
    fn a_failed_dictation_shows_why_then_clears() {
        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("fail");
        core.fail("Microphone: unplugged".into());
        let events: Vec<String> = seen.0.lock().iter().map(|(e, _)| e.clone()).collect();
        // The line first, so the overlay shows it instead of hiding for the Error status.
        assert_eq!(events, vec!["notice", "status"]);
        assert_eq!(seen.notices(), vec![("error".to_string(), "Microphone: unplugged".to_string())]);
        assert_eq!(core.status(), (Status::Error, Some("Microphone: unplugged".into())));
        assert!(cleared(&core), "the error clears on its own");
        done(dir);
    }

    #[test]
    fn escape_while_typing_stops_the_typing() {
        use handy_keys::Key;
        assert_eq!(key_while_typing(&key_down(Key::Escape)), WhileTyping::Stop);
        assert_eq!(key_while_typing(&key_down(Key::Delete)), WhileTyping::Edit);
        assert_eq!(key_while_typing(&key_down(Key::LeftArrow)), WhileTyping::Edit);
        assert_eq!(key_while_typing(&key_down(Key::A)), WhileTyping::Other, "Silas's own text");
        let mut up = key_down(Key::Escape);
        up.is_key_down = false;
        assert_eq!(key_while_typing(&up), WhileTyping::Other);

        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("escape");
        let mut s = core.settings.lock().clone();
        s.hotkey = "F13".into();
        core.apply_settings(s).unwrap();
        core.typing.store(true, Ordering::Relaxed);
        core.on_key(&key_down(Key::A));
        assert!(!core.stop_typing.load(Ordering::Relaxed), "typing goes on");
        core.on_key(&key_down(Key::Escape));
        assert!(core.stop_typing.load(Ordering::Relaxed), "type_text's still_here reads this and stops");
        assert!(core.typed_over.load(Ordering::Relaxed), "and nothing is learned from that text");
        // The dictation key while typing says why it does nothing.
        core.on_key(&key_down(Key::F13));
        assert_eq!(seen.notices().len(), 1);
        assert!(seen.notices()[0].1.contains("Escape"));
        done(dir);
    }

    #[test]
    fn a_panic_mid_dictation_ends_in_an_error_not_at_writing() {
        let _store = STORE_LOCK.lock();
        let (core, seen, dir) = seen_core("panic");
        core.set_status(Status::Typing, None);
        core.typing.store(true, Ordering::Relaxed);
        core.survive(false, || panic!("the speech runtime crashed"));
        assert_eq!(core.status(), (Status::Error, Some(crash_message())));
        assert!(!core.typing.load(Ordering::Relaxed), "the next keys are the user's again");
        assert_eq!(seen.notices()[0].0, "error");
        assert!(cleared(&core), "and it recovers");
        // From the key handler too: the key machine starts over.
        core.machine.lock().toggle_from_ui();
        core.survive(true, || panic!("in on_key"));
        assert!(!core.machine.lock().is_active());
        assert_eq!(core.status().0, Status::Error);
        done(dir);
    }

    #[test]
    fn escape_cancels_a_hotkey_pick_and_typing_keys_are_refused() {
        let _store = STORE_LOCK.lock();
        let (core, _seen, dir) = seen_core("pick");
        let pick = |k: handy_keys::Key| {
            let (tx, rx) = std::sync::mpsc::channel();
            *core.capture.lock() = Some(tx);
            core.on_key(&key_down(k));
            rx.try_recv().unwrap()
        };
        assert_eq!(pick(handy_keys::Key::Escape), Err(CANCELLED.to_string()));
        assert!(pick(handy_keys::Key::A).unwrap_err().contains("The A key types text"));
        assert_eq!(pick(handy_keys::Key::F13), Ok("F13".to_string()));
        // The Cancel button.
        let (tx, rx) = std::sync::mpsc::channel();
        *core.capture.lock() = Some(tx);
        assert!(core.cancel_capture());
        assert_eq!(rx.try_recv().unwrap(), Err(CANCELLED.to_string()));
        assert!(!core.cancel_capture(), "nothing left to cancel");
        done(dir);
    }

    #[test]
    fn a_password_field_gets_nothing_and_nothing_is_kept_or_learned() {
        use crate::focus::Restore;
        for r in [Restore::InPlace, Restore::Restored, Restore::Elsewhere, Restore::Failed] {
            assert_eq!(delivery(true, r), Delivery::Drop, "{r:?}");
        }
        assert_eq!(delivery(false, Restore::Failed), Delivery::HistoryOnly);
        assert_eq!(delivery(false, Restore::InPlace), Delivery::Type);
        assert!(secure_start_message().starts_with("A password field has the keyboard"));
        assert!(!may_learn(Some(1), Some(1), true));
        assert!(may_learn(Some(1), Some(1), false));
        assert!(!may_learn(Some(1), Some(2), false), "focus moved");
        assert!(may_learn(None, None, false), "where the system can't tell, as before");
    }

    #[test]
    fn the_last_word_gets_a_tail_and_the_time_limit_does_not() {
        assert_eq!(tail_for(Ending::Key, false), TAIL);
        assert!(TAIL >= Duration::from_millis(150) && TAIL <= Duration::from_millis(300), "{TAIL:?}");
        assert_eq!(tail_for(Ending::Limit, false), Duration::ZERO);
        assert_eq!(tail_for(Ending::Limit, true), Duration::ZERO);
    }

    #[test]
    fn a_release_after_the_speaker_stopped_waits_no_tail() {
        assert_eq!(tail_for(Ending::Key, true), Duration::ZERO);
    }

    #[test]
    fn only_a_running_download_can_be_cancelled() {
        let _store = STORE_LOCK.lock();
        let (core, _seen, dir) = seen_core("dlcancel");
        assert!(!core.cancel_download("parakeet-v3"));
        core.downloading.lock().push("parakeet-v3".into());
        assert!(core.cancel_download("parakeet-v3"));
        assert!(core.cancel_download("parakeet-v3"), "twice is fine");
        assert_eq!(core.cancelled_downloads.lock().len(), 1);
        done(dir);
    }

    #[test]
    fn a_load_for_a_model_no_longer_chosen_is_stale() {
        assert!(load_is_stale("cohere-int4", "parakeet-v3"));
        assert!(!load_is_stale("parakeet-v3", "parakeet-v3"));
    }

    #[test]
    fn one_model_is_never_loaded_twice_at_once() {
        assert!(should_start_load(None, "parakeet-v3"));
        assert!(should_start_load(Some("cohere-int4"), "parakeet-v3"));
        assert!(!should_start_load(Some("parakeet-v3"), "parakeet-v3"));
    }

    /// Review first-F2 and sol-F2 on #64: the registry and community lookups ran before the
    /// local report, so a community pick overrode this computer's own benchmark.
    #[test]
    fn this_computers_own_benchmark_wins_over_any_lookup() {
        let r = |m: &str, speed: f32, wer: f32| ModelResult { model: m.into(), speed, wer, ..Default::default() };
        let device = benchmark::build_device("macos", "Apple M5", "macOS 27", 10, 32, None);
        let local = Report::new(device.clone(), vec![r("parakeet-v3", 20.0, 0.05), r("cohere-int4", 12.0, 0.03)]);
        assert_eq!(local.pick.as_deref(), Some("cohere-int4"));
        let asked = std::cell::Cell::new(0);
        let lookup = |m: &str| {
            asked.set(asked.get() + 1);
            Some(m.to_string())
        };
        // A passing local pick: no lookup is consulted, and nothing changes.
        let plan = plan_device_pick(Some(&local), || lookup("parakeet-v3"), || lookup("parakeet-v3"));
        assert_eq!((plan, asked.get()), (DevicePlan::Keep, 0));
        // A local report with no passing pick still lets the registry, then the community, answer.
        let failing = Report::new(device, vec![r("parakeet-v3", 4.0, 0.05)]);
        assert_eq!(
            plan_device_pick(Some(&failing), || None, || Some("parakeet-v3".into())),
            DevicePlan::Remote("parakeet-v3".into(), "community")
        );
        assert_eq!(
            plan_device_pick(Some(&failing), || Some("cohere-int4".into()), || panic!("the registry answered")),
            DevicePlan::Remote("cohere-int4".into(), "registry")
        );
        assert_eq!(plan_device_pick(Some(&failing), || None, || None), DevicePlan::Keep);
        // No report and no answer: measure.
        assert_eq!(plan_device_pick(None, || None, || None), DevicePlan::Measure);
    }

    /// Review first-F1 on #64: a community pick, which anyone can post, chose the model first-run
    /// setup downloads.
    #[test]
    fn a_community_pick_never_chooses_a_download() {
        // Nothing installed yet: the registry may choose the download, the community may not.
        assert!(remote_pick_applies("default", "registry", false, false));
        assert!(!remote_pick_applies("default", "community", false, false));
        // A model that is here may be switched to from either.
        assert!(remote_pick_applies("default", "community", true, true));
        assert!(remote_pick_applies("default", "registry", true, true));
        // Never to one that isn't downloaded while another works, and never over a choice.
        assert!(!remote_pick_applies("default", "registry", false, true));
        assert!(!remote_pick_applies("default", "community", false, true));
        assert!(!remote_pick_applies("user", "registry", true, true));
        assert!(!remote_pick_applies("benchmark", "community", true, true));
    }

    #[test]
    fn automatic_changes_never_override_the_user() {
        assert!(!allows_model_change("user", true, true));
        assert!(allows_model_change("user", false, true));
        // The user's model was deleted: setup may load its pick.
        assert!(allows_model_change("user", true, false));
        for src in ["default", "registry", "community", "benchmark"] {
            assert!(allows_model_change(src, true, true));
            assert!(allows_model_change(src, false, true));
        }
    }
}
