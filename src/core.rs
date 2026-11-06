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

/// What the interface around the engine provides. The engine never names a UI toolkit:
/// the Tauri app (Windows, Linux, Android) and the native Mac app both implement this.
///
/// Calls may come from any thread, including while the engine holds its own short locks,
/// so an implementation must return quickly and must not call back into [`Core`] methods
/// that take the status lock (`set_status`).
pub trait Host: Send + Sync + 'static {
    /// A named event with a JSON payload: `status`, `transcript`, `learned`,
    /// `model-progress`, `benchmark-progress`, `benchmark-done`, `benchmark-error`.
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
    /// What messages call the app ("Silas" in the Tauri app, "Polycarp" in the native one).
    fn product_name(&self) -> String {
        "Silas".into()
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
    pub settings: Mutex<Settings>,
    pub dict: Mutex<Dictionary>,
    pub history: Mutex<Vec<HistoryItem>>,
    status: Mutex<(Status, Option<String>)>,
    engine: Mutex<Option<(String, Engine)>>,
    recording: Mutex<Option<(Recording, Instant)>>,
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
    /// The user pressed keys while Silas was typing, so its copy of the text is off.
    typed_over: AtomicBool,
    /// Bumped per recording so the length limit only stops the one it was set for.
    recording_gen: std::sync::atomic::AtomicU64,
    /// Bumped when a press starts from an error, so that error's recovery timer stands down.
    recover_gen: std::sync::atomic::AtomicU64,
    pub capture: Mutex<Option<std::sync::mpsc::Sender<String>>>,
    pub last_activity: Mutex<Instant>,
    downloading: Mutex<Vec<String>>,
    /// A benchmark is running; only one at a time.
    bench_running: AtomicBool,
    /// Recordings started so far; a benchmark clip that saw this change overlapped a dictation.
    dictations: AtomicU64,
    /// The model a background load is working on, so one model is never loaded twice at once.
    loading: Mutex<Option<String>>,
    /// False while the interface has paused dictation: keys are ignored, nothing records.
    /// The model stays loaded.
    enabled: AtomicBool,
}

pub type Shared = Arc<Core>;

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

pub fn key_access_message(product: &str) -> String {
    key_access_message_for(std::env::consts::OS, product)
}

/// The macOS text also covers a stale grant: after a build with a different signature, the
/// Accessibility switch can still look on for the old one while this build is not trusted.
/// Turning that switch off and on keeps the old signature, so the text never suggests it:
/// Fix Permission clears the entry, and removing and re-adding Silas by hand does the same.
fn key_access_message_for(os: &str, p: &str) -> String {
    match os {
        "macos" => format!("{p} can't hear your key yet. Click Fix Permission on the {p} home screen. If that doesn't work, open System Settings, Privacy & Security, Accessibility, select {p}, click the minus button to remove it, click plus and add {p} from Applications, then turn on its switch."),
        "linux" => format!("{p} can't read the keyboard. Reinstall the package, or add the udev rule from the README, then log out and back in."),
        _ => format!("{p} couldn't start its keyboard listener. Restart {p}."),
    }
}

impl Core {
    pub fn new(host: Arc<dyn Host>) -> Shared {
        let mut settings: Settings = store::load("settings.json");
        // Settings from before per-device picks have no modelSource. A model other than the
        // default was chosen by the user then, so no benchmark or registry may move them off it.
        if store::exists("settings.json") {
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
        }
        // A fresh nightly install stays on nightly instead of jumping to stable.
        if !store::exists("settings.json")
            && host.is_nightly()
        {
            settings.update_channel = "nightly".into();
        }
        let trigger = Trigger::parse(&settings.hotkey)
            .or_else(|| Trigger::parse(store::default_hotkey()))
            .expect("default hotkey parses");
        let machine = Machine::new(trigger, settings.tap_to_toggle);
        let mut history: Vec<HistoryItem> = store::load("history.json");
        history.truncate(store::HISTORY_KEEP);
        Arc::new(Core {
            host,
            dict: Mutex::new(store::load("dictionary.json")),
            history: Mutex::new(history),
            status: Mutex::new((Status::Loading, None)),
            engine: Mutex::new(None),
            recording: Mutex::new(None),
            target: Mutex::new(None),
            machine: Mutex::new(machine),
            blocked: Default::default(),
            watch: Mutex::new(None),
            typing: AtomicBool::new(false),
            listener_ok: AtomicBool::new(false),
            typed_over: AtomicBool::new(false),
            recording_gen: std::sync::atomic::AtomicU64::new(0),
            recover_gen: std::sync::atomic::AtomicU64::new(0),
            capture: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            downloading: Mutex::new(vec![]),
            bench_running: AtomicBool::new(false),
            dictations: AtomicU64::new(0),
            loading: Mutex::new(None),
            enabled: AtomicBool::new(true),
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
                cur.0 == Status::Error && cur.1.as_deref() == Some(key_access_message(&self.host.product_name()).as_str());
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

    pub fn set_status(&self, s: Status, detail: Option<String>) {
        let (s, detail) = {
            let mut cur = self.status.lock();
            let next = if s == Status::Ready && !self.listener_ok.load(Ordering::Relaxed) {
                (Status::Error, Some(key_access_message(&self.host.product_name())))
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

    pub fn active_model(&self) -> Option<String> {
        self.engine.lock().as_ref().map(|(id, _)| id.clone())
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
            *self.engine.lock() = None;
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
                    *me.engine.lock() = Some((id.clone(), engine));
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
        let result = models::download(id, &|downloaded, total, stage| {
            host.emit(
                "model-progress",
                json!({ "id": idc, "downloaded": downloaded, "total": total, "stage": stage }),
            );
        });
        self.downloading.lock().retain(|x| x != id);
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
        *self.engine.lock() = None;
        self.load_model();
    }

    // ------------------------------------------------------------ device pick

    /// At startup, in the background: look this computer up in the registry, and with no
    /// answer there, measure the active model once.
    pub fn start_device_pick(self: &Shared) {
        let me = self.clone();
        std::thread::spawn(move || me.device_pick());
    }

    fn device_pick(self: &Shared) {
        let key = benchmark::device().key.clone();
        let entry = benchmark::fetch_registry().lookup(&key).cloned();
        if let Some(entry) = entry {
            // A device the registry knows needs no benchmark. Switch only if that model is
            // here, or if no model is installed yet (onboarding downloads the recommended one);
            // never strand someone who has a working model on one they haven't downloaded.
            let usable = models::installed(&entry.model)
                || !models::MODELS.iter().any(|m| models::installed(m.id));
            // (applies, the model changed)
            let (apply, changed) = {
                let mut s = self.settings.lock();
                let apply = s.model_source == "default" && usable;
                let changed = apply && s.model != entry.model;
                if apply {
                    s.model = entry.model.clone();
                    s.model_source = "registry".into();
                }
                (apply, changed)
            };
            if apply {
                self.save_settings();
                if changed && models::installed(&entry.model) {
                    self.wait_quiet();
                    self.load_model();
                }
            }
            return;
        }
        if benchmark::load_report().is_some() {
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
        // and this speech set: numbers from another build aren't comparable.
        let mut results: Vec<ModelResult> = benchmark::load_report()
            .filter(|r| r.silas == env!("CARGO_PKG_VERSION") && r.suite == benchmark::SUITE_ID)
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
            .ok_or_else(|| format!("{} is not a key {} can use", settings.hotkey, self.host.product_name()))?;
        if !cfg!(target_os = "macos") {
            settings.update_channel = "nightly".into();
        }
        let old = {
            let mut cur = self.settings.lock();
            settings.model = cur.model.clone();
            settings.model_source = cur.model_source.clone();
            std::mem::replace(&mut *cur, settings.clone())
        };
        {
            let mut m = self.machine.lock();
            m.trigger = trigger;
            m.tap_to_toggle = settings.tap_to_toggle;
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

    /// Swallow the dictation key only while it is doing Polycarp's work. Paused, the key goes
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

    /// Every key event from the global listener lands here.
    pub fn on_key(self: &Shared, ev: &handy_keys::KeyEvent) {
        if !self.is_enabled() {
            // Paused: the hotkey does nothing, but picking a new one still works while a listener runs.
            self.capture_key(ev);
            return;
        }
        if self.typing.load(Ordering::Relaxed) {
            // Silas's own text arrives as key events too, so only keys it never sends
            // (deleting, moving the cursor) mean the user edited mid-typing.
            use handy_keys::Key::*;
            if ev.is_key_down
                && matches!(
                    ev.key,
                    Some(
                        Delete
                            | ForwardDelete
                            | LeftArrow
                            | RightArrow
                            | UpArrow
                            | DownArrow
                            | Escape
                            | Home
                            | End
                            | PageUp
                            | PageDown
                    )
                )
            {
                self.typed_over.store(true, Ordering::Relaxed);
            }
            return;
        }
        if self.capture_key(ev) {
            return;
        }
        let now = Instant::now();
        let out = self.machine.lock().on_event(ev, now);
        if out == Out::Nothing && !self.machine.lock().is_active() {
            self.feed_watch(ev, now);
        }
        self.act(out);
    }

    /// A pending "pick the hotkey" request takes this key event. True when it did.
    fn capture_key(&self, ev: &handy_keys::KeyEvent) -> bool {
        // Take the sender in its own statement: an `if let` would keep the lock held.
        let capture = self.capture.lock().take();
        if let Some(tx) = capture {
            match crate::keys::describe(ev) {
                Some(name) => {
                    let _ = tx.send(name);
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

    pub fn act(self: &Shared, out: Out) {
        match out {
            Out::Start => self.start_recording(),
            Out::Finish => self.finish_recording(),
            Out::Cancel => self.cancel_recording(),
            Out::Nothing => {}
        }
    }

    fn feed_watch(self: &Shared, ev: &handy_keys::KeyEvent, now: Instant) {
        let mut guard = self.watch.lock();
        let Some(w) = guard.as_mut() else { return };
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
        if focus.is_some() && crate::focus::focused() != focus {
            log::info!("focus moved during the edit; not learning from it");
            return;
        }
        self.learn_from_edit(&typed, &fixed, false, None);
    }

    // ------------------------------------------------------------ dictation

    fn start_recording(self: &Shared) {
        // A new dictation ends any edit we were following.
        let watching = self.watch.lock().take();
        if let Some(w) = watching {
            if let Some(Watch::Done(edit)) = w.idle(Instant::now() + Duration::from_secs(3600)) {
                self.finish_watch(w.focus, edit);
            }
        }
        match self.status().0 {
            Status::Ready => {}
            // Errors clear on their own after a few seconds; a retry right away shouldn't
            // be ignored while a model is loaded and the key listener works.
            Status::Error
                if self.active_model().is_some() && self.listener_ok.load(Ordering::Relaxed) =>
            {
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
            Status::NeedsModel => {
                self.machine.lock().reset();
                self.host.show_main();
                return;
            }
            _ => {
                self.machine.lock().reset();
                return;
            }
        }
        // The hotkey never shows the system mic prompt; only onboarding's Allow asks.
        if !crate::permissions::mic_ready_for_recording() {
            self.machine.lock().reset();
            self.set_status(
                Status::Error,
                Some(format!(
                    "Microphone: not allowed yet. Open {} and click Allow for the microphone.",
                    self.host.product_name()
                )),
            );
            self.recover_soon();
            return;
        }
        // Note where the keyboard is now, at key-down, without making the mic wait for it.
        let capture = std::thread::spawn(crate::focus::Target::capture);
        let device = self.settings.lock().microphone.clone();
        let host = self.host.clone();
        match Recording::start(device.as_deref(), move |lvl| host.level(lvl)) {
            Ok(r) => {
                // The mic is already running; publish the target before the recording, so a
                // release can never find a recording without its target.
                *self.target.lock() = capture.join().ok().flatten();
                self.dictations.fetch_add(1, Ordering::AcqRel);
                *self.recording.lock() = Some((r, Instant::now()));
                let gen = self.recording_gen.fetch_add(1, Ordering::Relaxed) + 1;
                self.cue("start");
                self.set_status(Status::Recording, None);
                // Hands-free mode left on by mistake would grow without end; stop at 5 minutes.
                let me = self.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(MAX_RECORDING);
                    if me.recording_gen.load(Ordering::Relaxed) == gen
                        && me.recording.lock().is_some()
                    {
                        me.machine.lock().reset();
                        me.finish_recording();
                    }
                });
            }
            Err(e) => {
                self.machine.lock().reset();
                self.set_status(Status::Error, Some(format!("Microphone: {e}")));
                self.recover_soon();
            }
        }
    }

    fn cancel_recording(self: &Shared) {
        // Release the lock before set_status, which waits on the main thread for the tray.
        let recording = self.recording.lock().take();
        self.target.lock().take();
        if let Some((r, _)) = recording {
            drop(r);
            self.set_status(Status::Ready, None);
        }
    }

    fn finish_recording(self: &Shared) {
        let recording = self.recording.lock().take();
        let Some((rec, started)) = recording else {
            return;
        };
        self.cue("stop");
        // Carried by value: a new dictation may start before this one finishes typing.
        let target = self.target.lock().take();
        let me = self.clone();
        std::thread::spawn(move || {
            let released = Instant::now();
            let samples = rec.finish();
            let secs = samples.len() as f32 / 16_000.0;
            if secs < 0.35 {
                me.set_status(Status::Ready, None);
                return;
            }
            me.set_status(Status::Transcribing, None);
            let language = me.settings.lock().language.clone();
            let raw = {
                let mut engine = me.engine.lock();
                match engine.as_mut() {
                    Some((_, e)) => e.transcribe(&samples, &language),
                    None => Err(anyhow::anyhow!("No model is loaded")),
                }
            };
            let raw = match raw {
                Ok(t) => t,
                Err(e) => {
                    me.set_status(Status::Error, Some(format!("Transcription failed: {e}")));
                    me.recover_soon();
                    return;
                }
            };
            let s = me.settings.lock().clone();
            let mut text = me.dict.lock().apply(&raw, s.remove_fillers);
            if text.trim().is_empty() {
                me.set_status(Status::Ready, None);
                return;
            }
            if s.trailing_space && !text.ends_with(char::is_whitespace) {
                text.push(' ');
            }
            use crate::focus::Restore;
            let restored = target.as_ref().map_or(Restore::Elsewhere, |t| t.restore());
            let latency = released.elapsed().as_millis() as u64;
            // Typing into the window that stole focus is the bug; keep the text in History.
            let mut lost_focus = restored == Restore::Failed;
            let mut typing_error: Option<String> = None;
            // Guard the typing only when Silas knows where the text belongs.
            let guarded = matches!(
                restored,
                Restore::InPlace | Restore::Restored | Restore::Elsewhere
            );
            let still_here = |after_focus_key: bool| {
                !guarded
                    || target
                        .as_ref()
                        .is_none_or(|t| t.still_here(after_focus_key))
            };
            if !lost_focus {
                me.set_status(Status::Typing, None);
                me.typed_over.store(false, Ordering::Relaxed);
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
                match typed {
                    Ok(finished) => lost_focus = !finished,
                    Err(e) => typing_error = Some(e.to_string()),
                }
            }
            let typed_over = me.typed_over.load(Ordering::Relaxed);
            if !lost_focus
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
            };
            {
                let mut h = me.history.lock();
                h.insert(0, item.clone());
                h.truncate(store::HISTORY_KEEP);
                store::save("history.json", &*h);
            }
            store::save("dictionary.json", &*me.dict.lock());
            *me.last_activity.lock() = Instant::now();
            // `typed` is true only when the text really went out as keystrokes: the app moves Silas to the Trash
            // after the first one, never after a dictation that only reached History.
            let mut payload = json!(&item);
            payload["typed"] = json!(!lost_focus && typing_error.is_none());
            me.host.emit("transcript", payload);
            if let Some(e) = typing_error {
                me.set_status(
                    Status::Error,
                    Some(format!("{e}. Your full text is in History.")),
                );
                me.recover_soon();
            } else if lost_focus {
                me.set_status(
                    Status::Error,
                    Some(format!("Another window took the keyboard, so {} stopped typing. Your full text is in History.", me.host.product_name())),
                );
                me.recover_soon();
            } else {
                me.set_status(Status::Ready, None);
            }
        });
    }

    fn recover_soon(self: &Shared) {
        let me = self.clone();
        let gen = self.recover_gen.load(Ordering::Relaxed);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
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
        if !self.settings.lock().sounds {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let sound = if which == "start" { "Tink" } else { "Pop" };
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

    // ------------------------------------------------------------ learning

    pub fn learn_from_edit(
        &self,
        typed: &str,
        fixed: &str,
        explicit: bool,
        history_id: Option<&str>,
    ) -> LearnResult {
        let result = self.dict.lock().learn(typed, fixed, explicit);
        store::save("dictionary.json", &*self.dict.lock());
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
        for i in h.iter() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mac_key_message_explains_a_stale_grant() {
        let m = key_access_message_for("macos", "Silas");
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

    #[test]
    fn the_microphone_key_is_swallowed_only_while_it_is_ours() {
        use handy_keys::{Key, Modifiers};
        let dir = std::env::temp_dir().join(format!("pc-blocked-{}", std::process::id()));
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
        assert!(holds(&core), "chosen: Polycarp takes the press");
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
    fn a_paused_engine_still_takes_the_key_being_picked() {
        use handy_keys::{Key, KeyEvent, Modifiers};
        let dir = std::env::temp_dir().join(format!("pc-capture-{}", std::process::id()));
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
        assert_eq!(rx.try_recv().as_deref(), Ok("F13"));
        store::set_data_dir(None);
        let _ = std::fs::remove_dir_all(dir);
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

    #[test]
    fn automatic_changes_never_override_the_user() {
        assert!(!allows_model_change("user", true, true));
        assert!(allows_model_change("user", false, true));
        // The user's model was deleted: setup may load its pick.
        assert!(allows_model_change("user", true, false));
        for src in ["default", "registry", "benchmark"] {
            assert!(allows_model_change(src, true, true));
            assert!(allows_model_change(src, false, true));
        }
    }
}
