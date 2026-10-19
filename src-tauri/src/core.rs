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
    pub capture: Mutex<Option<std::sync::mpsc::Sender<String>>>,
    pub last_activity: Mutex<Instant>,
    downloading: Mutex<Vec<String>>,
}

pub type Shared = Arc<Core>;

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
            capture: Mutex::new(None),
            last_activity: Mutex::new(Instant::now()),
            downloading: Mutex::new(vec![]),
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
        let show = self.settings.lock().show_overlay
            && matches!(s, Status::Recording | Status::Transcribing | Status::Typing);
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
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
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

    pub fn select_model(self: &Shared, id: &str) {
        self.settings.lock().model = id.to_string();
        self.save_settings();
        *self.engine.lock() = None;
        self.load_model();
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
        *self.target.lock() = crate::focus::Target::capture();
        let device = self.settings.lock().microphone.clone();
        let app = self.app.clone();
        match Recording::start(device.as_deref(), move |lvl| {
            let _ = app.emit_to("overlay", "level", lvl);
        }) {
            Ok(r) => {
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
            let latency = released.elapsed().as_millis() as u64;
            let target = me.target.lock().take();
            if let Some(t) = target {
                t.restore();
            }
            me.set_status(Status::Typing, None);
            me.typed_over.store(false, Ordering::Relaxed);
            me.typing.store(true, Ordering::Relaxed);
            let typed = typer::type_text(
                &text,
                typer::Pace::from_setting(&s.typing_speed),
                s.newline_as_shift_enter,
            );
            // Let the last synthetic events drain before reading the keyboard again.
            std::thread::sleep(Duration::from_millis(60));
            me.typing.store(false, Ordering::Relaxed);
            if let Err(e) = typed {
                me.set_status(Status::Error, Some(e.to_string()));
                me.recover_soon();
                return;
            }
            let typed_over = me.typed_over.load(Ordering::Relaxed);
            if s.learn_from_edits && !typed_over && crate::focus::us_layout() {
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
            me.set_status(Status::Ready, None);
        });
    }

    fn recover_soon(self: &Shared) {
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            if me.status().0 == Status::Error {
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
