//! The running app: owns settings, the dictionary, the loaded model and the
//! record → transcribe → type → learn loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Local, Utc};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::audio::Recording;
use crate::benchmark::{self, ModelResult, Report};
use crate::dictionary::{Dictionary, LearnResult};
use crate::keys::{EditWatch, Machine, Out, Trigger, Watch};
use crate::models::{self, Engine};
use crate::store::{self, HistoryItem, Settings, Stats};
use crate::typer;

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
    pub app: AppHandle,
    pub settings: Mutex<Settings>,
    pub dict: Mutex<Dictionary>,
    pub history: Mutex<Vec<HistoryItem>>,
    status: Mutex<(Status, Option<String>)>,
    engine: Mutex<Option<(String, Engine)>>,
    recording: Mutex<Option<(Recording, Instant)>>,
    /// Where the keyboard was when the hotkey went down; the text goes back there.
    target: Mutex<Option<crate::focus::Target>>,
    pub machine: Mutex<Machine>,
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
}

pub type Shared = Arc<Core>;

/// Frees the benchmark slot when dropped.
pub struct BenchGuard(Shared);

impl Drop for BenchGuard {
    fn drop(&mut self) {
        self.0.bench_running.store(false, Ordering::Release);
    }
}

const MAX_RECORDING: Duration = Duration::from_secs(300);

pub fn key_access_message() -> String {
    if cfg!(target_os = "macos") {
        "Silas can't hear your key yet. Turn on Silas in System Settings › Privacy & Security › Accessibility.".into()
    } else if cfg!(target_os = "linux") {
        "Silas can't read the keyboard. Reinstall the package, or add the udev rule from the README, then log out and back in.".into()
    } else {
        "Silas couldn't start its keyboard listener. Restart Silas.".into()
    }
}

impl Core {
    pub fn new(app: AppHandle) -> Shared {
        let mut settings: Settings = store::load("settings.json");
        // Settings from before per-device picks have no modelSource. A model other than the
        // default was chosen by the user then, so no benchmark or registry may move them off it.
        if store::exists("settings.json") {
            store::migrate_model_source(&mut settings, &store::read_raw("settings.json"));
        }
        // A fresh nightly install stays on nightly instead of jumping to stable.
        if !store::exists("settings.json") && app.package_info().version.pre.as_str().starts_with("nightly") {
            settings.update_channel = "nightly".into();
        }
        let trigger = Trigger::parse(&settings.hotkey)
            .or_else(|| Trigger::parse(store::default_hotkey()))
            .expect("default hotkey parses");
        let machine = Machine::new(trigger, settings.tap_to_toggle);
        let mut history: Vec<HistoryItem> = store::load("history.json");
        history.truncate(store::HISTORY_KEEP);
        Arc::new(Core {
            app,
            dict: Mutex::new(store::load("dictionary.json")),
            history: Mutex::new(history),
            status: Mutex::new((Status::Loading, None)),
            engine: Mutex::new(None),
            recording: Mutex::new(None),
            target: Mutex::new(None),
            machine: Mutex::new(machine),
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
            let missing_key = cur.0 == Status::Error && cur.1.as_deref() == Some(key_access_message().as_str());
            (ok && missing_key) || (!ok && cur.0 == Status::Ready)
        };
        if ok {
            // The user just flipped the switch in System Settings; bring them back to Silas.
            if let Some(w) = self.app.get_webview_window("main") {
                if w.is_visible().unwrap_or(false) {
                    let _ = w.set_focus();
                }
            }
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
        self.set_status(if installed { Status::Loading } else { Status::NeedsModel }, None);
    }

    pub fn set_status(&self, s: Status, detail: Option<String>) {
        let (s, detail) = {
            let mut cur = self.status.lock();
            let next = if s == Status::Ready && !self.listener_ok.load(Ordering::Relaxed) {
                (Status::Error, Some(key_access_message()))
            } else {
                (s, detail)
            };
            *cur = next.clone();
            next
        };
        let _ = self
            .app
            .emit("status", json!({ "status": s, "detail": detail }));
        crate::tray::refresh(&self.app, s);
        self.overlay(s);
    }

    fn overlay(&self, s: Status) {
        let Some(w) = self.app.get_webview_window("overlay") else {
            return;
        };
        let enabled = self.settings.lock().show_overlay;
        let show = enabled && overlay_status(s);
        if show {
            if let Ok(Some(monitor)) = w
                .primary_monitor()
                .or_else(|_| w.current_monitor())
            {
                let size = monitor.size();
                let pos = monitor.position();
                let scale = monitor.scale_factor();
                let (ww, wh) = (260.0 * scale, 64.0 * scale);
                let x = pos.x as f64 + (size.width as f64 - ww) / 2.0;
                let y = pos.y as f64 + size.height as f64 - wh - 72.0 * scale;
                let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
            }
        }
        place_overlay(&self.app, w, enabled);
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
        self.set_status(Status::Loading, Some("Loading the speech model".into()));
        let me = self.clone();
        std::thread::spawn(move || {
            let t = Instant::now();
            match Engine::load(&id) {
                Ok(mut engine) => {
                    // Warm up so the first real dictation is as fast as the rest.
                    let _ = engine.transcribe(&vec![0.0; 16_000], "en");
                    // Another model may have been chosen while this one loaded.
                    if me.settings.lock().model != id {
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
        let app = self.app.clone();
        let idc = id.to_string();
        let result = models::download(id, &|downloaded, total, stage| {
            let _ = app.emit(
                "model-progress",
                json!({ "id": idc, "downloaded": downloaded, "total": total, "stage": stage }),
            );
        });
        self.downloading.lock().retain(|x| x != id);
        if let Err(e) = &result {
            let _ = self.app.emit("model-progress", json!({ "id": id, "downloaded": 0, "total": 0, "stage": "error", "message": e.to_string() }));
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
        {
            let mut s = self.settings.lock();
            s.model = id.to_string();
            if !auto {
                s.model_source = "user".into();
            }
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
            let usable = models::installed(&entry.model) || !models::MODELS.iter().any(|m| models::installed(m.id));
            let apply = {
                let mut s = self.settings.lock();
                let apply = s.model_source == "default" && usable;
                if apply {
                    s.model = entry.model.clone();
                    s.model_source = "registry".into();
                }
                apply
            };
            if apply {
                self.save_settings();
                if models::installed(&entry.model) {
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
                if let Err(e) = self.run_benchmark(false, guard) {
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

    /// Hold until no dictation is happening, so a recording is never delayed by more than
    /// the one clip in flight.
    fn wait_quiet(&self) {
        loop {
            let busy = matches!(
                self.status().0,
                Status::Recording | Status::Transcribing | Status::Typing | Status::Loading
            ) || self.recording.lock().is_some();
            if !busy {
                return;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// Measure the active model, and with `all` every other installed one. Saves the report,
    /// emits `benchmark-progress` then `benchmark-done` (or `benchmark-error`), and moves to the
    /// pick unless the user chose a model themselves.
    pub fn run_benchmark(self: &Shared, all: bool, _guard: BenchGuard) -> anyhow::Result<Report> {
        let result = self.measure(all);
        match &result {
            Ok(report) => {
                let _ = self.app.emit("benchmark-done", report);
                if let Some(pick) = &report.pick {
                    self.apply_pick(pick);
                }
            }
            Err(e) => {
                let _ = self.app.emit("benchmark-error", json!({ "message": e.to_string() }));
            }
        }
        result
    }

    fn measure(self: &Shared, all: bool) -> anyhow::Result<Report> {
        let active = self
            .active_model()
            .ok_or_else(|| anyhow::anyhow!("No model is loaded yet"))?;
        let mut ids = vec![active.clone()];
        if all {
            ids.extend(
                models::MODELS
                    .iter()
                    .filter(|m| m.id != active && models::installed(m.id))
                    .map(|m| m.id.to_string()),
            );
        }
        let mut fresh: Vec<ModelResult> = vec![];
        for id in &ids {
            let progress = |clip: usize, clips: usize| {
                // Every clip starts after this returns, outside the timing.
                if clip < clips {
                    self.wait_quiet();
                }
                let _ = self.app.emit("benchmark-progress", json!({ "model": id, "clip": clip, "clips": clips }));
            };
            let run = if *id == active {
                // The live engine, one clip per lock, so a dictation waits at most one clip.
                let mut clip = |samples: &[f32]| -> anyhow::Result<String> {
                    let mut engine = self.engine.lock();
                    match engine.as_mut() {
                        Some((loaded, e)) if loaded == id => e.transcribe(samples, "en"),
                        _ => anyhow::bail!("The active model changed during the benchmark"),
                    }
                };
                benchmark::run_suite(id, &mut clip, &progress)
            } else {
                models::Engine::load(id).and_then(|mut engine| {
                    benchmark::run_suite(id, &mut |s: &[f32]| engine.transcribe(s, "en"), &progress)
                })
            };
            match run {
                Ok(r) => fresh.push(r),
                // A model that won't load shouldn't sink the rest; the active one must work.
                Err(e) if *id != active => log::warn!("benchmark of {id} failed: {e}"),
                Err(e) => return Err(e),
            }
        }
        // Keep earlier results for models this run skipped.
        let mut results: Vec<ModelResult> = benchmark::load_report()
            .map(|r| r.results)
            .unwrap_or_default()
            .into_iter()
            .filter(|old| models::spec(&old.model).is_some() && !fresh.iter().any(|f| f.model == old.model))
            .collect();
        results.extend(fresh);
        results.sort_by_key(|r| models::MODELS.iter().position(|m| m.id == r.model));
        let report = Report::new(benchmark::device().clone(), results);
        benchmark::save_report(&report);
        Ok(report)
    }

    /// Move to the benchmark's pick if it is installed and the user hasn't chosen.
    fn apply_pick(self: &Shared, pick: &str) {
        if !models::installed(pick) {
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
            self.load_model();
        }
    }

    pub fn save_settings(&self) {
        store::save("settings.json", &*self.settings.lock());
    }

    // ------------------------------------------------------------ keys

    /// Every key event from the global listener lands here.
    pub fn on_key(self: &Shared, ev: &handy_keys::KeyEvent) {
        if self.typing.load(Ordering::Relaxed) {
            // Silas's own text arrives as key events too, so only keys it never sends
            // (deleting, moving the cursor) mean the user edited mid-typing.
            use handy_keys::Key::*;
            if ev.is_key_down
                && matches!(
                    ev.key,
                    Some(Delete | ForwardDelete | LeftArrow | RightArrow | UpArrow | DownArrow | Escape | Home | End | PageUp | PageDown)
                )
            {
                self.typed_over.store(true, Ordering::Relaxed);
            }
            return;
        }
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
            return;
        }
        let now = Instant::now();
        let out = self.machine.lock().on_event(ev, now);
        if out == Out::Nothing && !self.machine.lock().is_active() {
            self.feed_watch(ev, now);
        }
        self.act(out);
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
            Status::Error if self.active_model().is_some() && self.listener_ok.load(Ordering::Relaxed) => {
                // Claim the error from its recovery timer; if the timer got there first it is
                // resetting the key machine right now, so let this press go.
                let g = self.recover_gen.load(Ordering::Relaxed);
                if self.recover_gen.compare_exchange(g, g + 1, Ordering::AcqRel, Ordering::Relaxed).is_err() {
                    self.machine.lock().reset();
                    return;
                }
            }
            Status::NeedsModel => {
                self.machine.lock().reset();
                crate::show_main(&self.app);
                return;
            }
            _ => {
                self.machine.lock().reset();
                return;
            }
        }
        // Note where the keyboard is now, at key-down, without making the mic wait for it.
        let capture = std::thread::spawn(crate::focus::Target::capture);
        let device = self.settings.lock().microphone.clone();
        let app = self.app.clone();
        match Recording::start(device.as_deref(), move |lvl| {
            let _ = app.emit_to("overlay", "level", lvl);
        }) {
            Ok(r) => {
                // The mic is already running; publish the target before the recording, so a
                // release can never find a recording without its target.
                *self.target.lock() = capture.join().ok().flatten();
                *self.recording.lock() = Some((r, Instant::now()));
                let gen = self.recording_gen.fetch_add(1, Ordering::Relaxed) + 1;
                self.cue("start");
                self.set_status(Status::Recording, None);
                // Hands-free mode left on by mistake would grow without end; stop at 5 minutes.
                let me = self.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(MAX_RECORDING);
                    if me.recording_gen.load(Ordering::Relaxed) == gen && me.recording.lock().is_some() {
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
            let guarded = matches!(restored, Restore::InPlace | Restore::Restored | Restore::Elsewhere);
            let still_here = |after_focus_key: bool| !guarded || target.as_ref().is_none_or(|t| t.still_here(after_focus_key));
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
            if !lost_focus && typing_error.is_none() && s.learn_from_edits && !typed_over && crate::focus::us_layout() {
                *me.watch.lock() = Some(EditWatch::new(&text, Instant::now(), crate::focus::focused()));
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
            let _ = me.app.emit("transcript", &item);
            if let Some(e) = typing_error {
                me.set_status(Status::Error, Some(format!("{e}. Your full text is in History.")));
                me.recover_soon();
            } else if lost_focus {
                me.set_status(
                    Status::Error,
                    Some("Another window took the keyboard, so Silas stopped typing. Your full text is in History.".into()),
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
                && me.recover_gen.compare_exchange(gen, gen + 1, Ordering::AcqRel, Ordering::Relaxed).is_ok()
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
            let _ = self.app.emit("learned", &result);
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

fn overlay_status(s: Status) -> bool {
    matches!(s, Status::Recording | Status::Transcribing | Status::Typing)
}

/// Shows or hides the overlay on the main thread, deciding there from the current status, not
/// the one this call was made for: two status changes racing from worker threads can queue
/// their calls in either order, and the window must end in the state the last status asks for.
///
/// On macOS, showing puts it on the Space in front, full-screen apps included. Tao sets "all
/// Spaces" once at launch, but a hidden window misses Spaces made after that (a window going
/// full screen) and comes up on its old Space, covered. Clearing and setting the behavior again
/// before each show puts it back on every Space; full-screen auxiliary lets it sit over a
/// full-screen app. `orderFrontRegardless` orders it in without making it key or activating
/// Silas, which runs in the background.
fn place_overlay(app: &AppHandle, w: tauri::WebviewWindow, enabled: bool) {
    let app = app.clone();
    let _ = w.clone().run_on_main_thread(move || {
        // Only the status lock, held for a copy: no thread holds it while waiting on this one.
        let show = enabled && app.try_state::<Shared>().is_some_and(|c| overlay_status(c.status().0));
        #[cfg(target_os = "macos")]
        order_overlay(&w, show);
        #[cfg(not(target_os = "macos"))]
        let _ = if show { w.show() } else { w.hide() };
    });
}

/// Runs on the main thread. The window is looked up here, so it can't be gone by the time
/// it's used.
#[cfg(target_os = "macos")]
fn order_overlay(w: &tauri::WebviewWindow, show: bool) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    // NSWindowCollectionBehavior: CanJoinAllSpaces | Stationary | IgnoresCycle |
    // FullScreenAuxiliary.
    const BEHAVIOR: usize = 1 | 1 << 4 | 1 << 6 | 1 << 8;
    let Ok(ns) = w.ns_window() else { return };
    let Some(win) = (unsafe { (ns as *mut AnyObject).as_ref() }) else { return };
    unsafe {
        if !show {
            let _: () = msg_send![win, orderOut: std::ptr::null::<AnyObject>()];
            return;
        }
        // Already up (Recording, then Writing): leave its Spaces alone so it can't blink.
        let visible: bool = msg_send![win, isVisible];
        if !visible {
            let _: () = msg_send![win, setCollectionBehavior: 0usize];
            let _: () = msg_send![win, setCollectionBehavior: BEHAVIOR];
        }
        let _: () = msg_send![win, orderFrontRegardless];
    }
}
