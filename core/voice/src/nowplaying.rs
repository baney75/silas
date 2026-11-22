//! Pauses whatever is playing while you dictate, then plays exactly those players again.
//!
//! Music or a video playing out loud ends up in the microphone and in the transcript. When a dictation
//! starts, every player that is playing is paused and noted; when it ends or is cancelled, only the
//! noted players that are still paused play again. Something the user had paused already is never
//! touched, and neither is a player they started, stopped or closed while dictating.
//!
//! The work runs on one thread of its own, in order, so the key thread never waits on a media player:
//! talking to a player can take tens of milliseconds, and a hung one could take seconds.
//!
//! Linux: MPRIS over D-Bus. macOS: the MediaRemote private framework, loaded at run time; recent macOS
//! versions keep the playing state from apps outside Apple, and then nothing is paused. Windows: the
//! system media transport controls.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use parking_lot::Mutex;

/// The media players on this computer, as the platform sees them. Ids are whatever the platform
/// names a player by; they only need to stay the same for one dictation.
pub trait Players: Send {
    /// The players playing right now.
    fn playing(&mut self) -> Vec<String>;
    /// Pause one. True when it was paused.
    fn pause(&mut self, id: &str) -> bool;
    /// Whether this player is still there and paused (not playing, not stopped, not closed).
    fn is_paused(&mut self, id: &str) -> bool;
    fn play(&mut self, id: &str);
}

/// Which players Silas paused, and so may play again.
#[derive(Debug, Default)]
pub struct Ledger {
    paused: Vec<String>,
}

impl Ledger {
    /// Pause every player that is playing, and note it.
    pub fn pause(&mut self, players: &mut dyn Players) {
        for id in players.playing() {
            if players.pause(&id) && !self.paused.contains(&id) {
                self.paused.push(id);
            }
        }
    }

    /// Play again what Silas paused, if it is still paused. A player the user started, stopped or
    /// closed meanwhile is theirs again.
    pub fn resume(&mut self, players: &mut dyn Players) {
        for id in std::mem::take(&mut self.paused) {
            if players.is_paused(&id) {
                players.play(&id);
            }
        }
    }

    pub fn paused(&self) -> &[String] {
        &self.paused
    }
}

enum Job {
    Pause,
    /// Play again after this long (the recording's tail, so the music doesn't come back into it).
    Resume(Duration),
}

/// The handle the core holds. Calls only send a message; the thread starts on first use, so a
/// computer where nobody turns this on never talks to a player.
pub struct MediaPause {
    tx: Mutex<Option<Sender<Job>>>,
    make: fn() -> Box<dyn Players>,
}

impl MediaPause {
    /// This platform's players. Tests get none: they must never pause the music of whoever runs them.
    pub fn system() -> Self {
        let make: fn() -> Box<dyn Players> = if cfg!(test) { || Box::new(NoPlayers) } else { system_players };
        Self::with(make)
    }

    pub fn with(make: fn() -> Box<dyn Players>) -> Self {
        Self { tx: Mutex::new(None), make }
    }

    /// A dictation is starting: pause what's playing. Returns at once.
    pub fn pause(&self) {
        self.send(Job::Pause, true);
    }

    /// The dictation ended or was cancelled: after `after`, play again what was paused. Returns at once.
    pub fn resume(&self, after: Duration) {
        self.send(Job::Resume(after), false);
    }

    fn send(&self, job: Job, start: bool) {
        let mut tx = self.tx.lock();
        if tx.is_none() {
            // Nothing was ever paused, so there is nothing to resume.
            if !start {
                return;
            }
            let (sender, jobs) = mpsc::channel();
            let make = self.make;
            let spawned = std::thread::Builder::new()
                .name("silas-media".into())
                .spawn(move || run(make(), jobs));
            if spawned.is_err() {
                return;
            }
            *tx = Some(sender);
        }
        if let Some(sender) = tx.as_ref() {
            let _ = sender.send(job);
        }
    }
}

fn run(mut players: Box<dyn Players>, jobs: Receiver<Job>) {
    let mut ledger = Ledger::default();
    let mut next = jobs.recv().ok();
    while let Some(job) = next.take() {
        match job {
            Job::Pause => ledger.pause(players.as_mut()),
            Job::Resume(after) => {
                // A new dictation inside the wait keeps the music paused instead of letting it blip.
                match jobs.recv_timeout(after) {
                    Ok(job) => {
                        next = Some(job);
                        continue;
                    }
                    Err(RecvTimeoutError::Timeout) => ledger.resume(players.as_mut()),
                    Err(RecvTimeoutError::Disconnected) => {
                        ledger.resume(players.as_mut());
                        return;
                    }
                }
            }
        }
        next = jobs.recv().ok();
    }
}

/// No players: tests, and platforms without a backend.
pub struct NoPlayers;

impl Players for NoPlayers {
    fn playing(&mut self) -> Vec<String> {
        vec![]
    }
    fn pause(&mut self, _: &str) -> bool {
        false
    }
    fn is_paused(&mut self, _: &str) -> bool {
        false
    }
    fn play(&mut self, _: &str) {}
}

#[cfg(target_os = "linux")]
fn system_players() -> Box<dyn Players> {
    Box::new(linux::Mpris::default())
}

#[cfg(target_os = "macos")]
fn system_players() -> Box<dyn Players> {
    Box::new(mac::NowPlaying)
}

#[cfg(target_os = "windows")]
fn system_players() -> Box<dyn Players> {
    Box::new(windows_smtc::Sessions::default())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn system_players() -> Box<dyn Players> {
    Box::new(NoPlayers)
}

#[cfg(target_os = "linux")]
mod linux {
    //! MPRIS: every player owns a bus name under `org.mpris.MediaPlayer2.` and reports PlaybackStatus.

    use std::time::Duration;
    use zbus::blocking::Connection;
    use zbus::zvariant::OwnedValue;

    const PREFIX: &str = "org.mpris.MediaPlayer2.";
    const PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
    /// A player that doesn't answer in this long is left alone.
    const TIMEOUT: Duration = Duration::from_millis(500);

    #[derive(Default)]
    pub struct Mpris {
        bus: Option<Connection>,
    }

    impl Mpris {
        fn bus(&mut self) -> Option<&Connection> {
            if self.bus.is_none() {
                let built = zbus::blocking::connection::Builder::session()
                    .map(|b| b.method_timeout(TIMEOUT))
                    .and_then(|b| b.build());
                match built {
                    Ok(c) => self.bus = Some(c),
                    Err(e) => {
                        log::info!("media: no session bus, so nothing is paused: {e}");
                        return None;
                    }
                }
            }
            self.bus.as_ref()
        }

        fn status(&mut self, name: &str) -> Option<String> {
            let bus = self.bus()?;
            let reply = bus
                .call_method(Some(name), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(PLAYER, "PlaybackStatus"))
                .ok()?;
            let value: OwnedValue = reply.body().deserialize().ok()?;
            String::try_from(value).ok()
        }

        fn call(&mut self, name: &str, method: &str) -> bool {
            let Some(bus) = self.bus() else { return false };
            match bus.call_method(Some(name), PATH, Some(PLAYER), method, &()) {
                Ok(_) => true,
                Err(e) => {
                    log::info!("media: {method} on {name} failed: {e}");
                    false
                }
            }
        }
    }

    impl super::Players for Mpris {
        fn playing(&mut self) -> Vec<String> {
            let Some(bus) = self.bus() else { return vec![] };
            let names: Vec<String> = match bus
                .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
                .and_then(|r| r.body().deserialize::<Vec<String>>())
            {
                Ok(n) => n,
                Err(e) => {
                    log::info!("media: could not list players: {e}");
                    // The bus may have gone away; connect again next time.
                    self.bus = None;
                    return vec![];
                }
            };
            names
                .into_iter()
                // playerctld only mirrors another player; pausing that player is enough.
                .filter(|n| n.starts_with(PREFIX) && !n.starts_with("org.mpris.MediaPlayer2.playerctld"))
                .filter(|n| self.status(n).as_deref() == Some("Playing"))
                .collect()
        }

        fn pause(&mut self, id: &str) -> bool {
            self.call(id, "Pause")
        }

        fn is_paused(&mut self, id: &str) -> bool {
            self.status(id).as_deref() == Some("Paused")
        }

        fn play(&mut self, id: &str) {
            self.call(id, "Play");
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! MediaRemote, the private framework behind Control Center's Now Playing, loaded with dlopen the way
    //! focus.rs loads SkyLight. It names one player: the app that's playing. Its process id is the
    //! player's id, so Play is never sent to a different app (with nothing playing, Play opens Music).
    //! Since macOS 15.4 it tells apps outside Apple nothing about what's playing; then nothing is paused.

    use block2::RcBlock;
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::mpsc;
    use std::sync::OnceLock;
    use std::time::Duration;

    extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
    }
    const RTLD_LAZY: c_int = 0x1;
    const RTLD_GLOBAL: c_int = 0x8;
    /// MRMediaRemoteCommand values.
    const PLAY: u32 = 0;
    const PAUSE: u32 = 1;
    /// How long to wait for MediaRemote to answer a question.
    const ANSWER: Duration = Duration::from_millis(300);

    type IsPlaying = unsafe extern "C" fn(*mut c_void, &block2::Block<dyn Fn(u8)>);
    type Pid = unsafe extern "C" fn(*mut c_void, &block2::Block<dyn Fn(c_int)>);
    type SendCommand = unsafe extern "C" fn(u32, *const c_void) -> u8;

    struct MediaRemote {
        is_playing: IsPlaying,
        pid: Pid,
        send: SendCommand,
    }

    fn media_remote() -> Option<&'static MediaRemote> {
        static MR: OnceLock<Option<MediaRemote>> = OnceLock::new();
        MR.get_or_init(|| unsafe {
            let h = dlopen(c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote".as_ptr(), RTLD_LAZY | RTLD_GLOBAL);
            if h.is_null() {
                log::info!("media: MediaRemote isn't there, so nothing is paused");
                return None;
            }
            let sym = |name: &std::ffi::CStr| Some(dlsym(h, name.as_ptr())).filter(|p| !p.is_null());
            Some(MediaRemote {
                is_playing: std::mem::transmute::<*mut c_void, IsPlaying>(sym(c"MRMediaRemoteGetNowPlayingApplicationIsPlaying")?),
                pid: std::mem::transmute::<*mut c_void, Pid>(sym(c"MRMediaRemoteGetNowPlayingApplicationPID")?),
                send: std::mem::transmute::<*mut c_void, SendCommand>(sym(c"MRMediaRemoteSendCommand")?),
            })
        })
        .as_ref()
    }

    /// The now-playing app's process id, if it is playing. None when MediaRemote doesn't say.
    fn now_playing() -> Option<(c_int, bool)> {
        let mr = media_remote()?;
        let (tx, rx) = mpsc::channel();
        let tx2 = tx.clone();
        let pid_block: RcBlock<dyn Fn(c_int)> = RcBlock::new(move |pid: c_int| {
            let _ = tx.send((Some(pid), None));
        });
        let playing_block: RcBlock<dyn Fn(u8)> = RcBlock::new(move |playing: u8| {
            let _ = tx2.send((None, Some(playing != 0)));
        });
        unsafe {
            let queue = dispatch_get_global_queue(0, 0);
            (mr.pid)(queue, &*pid_block);
            (mr.is_playing)(queue, &*playing_block);
        }
        let (mut pid, mut playing) = (None, None);
        while pid.is_none() || playing.is_none() {
            match rx.recv_timeout(ANSWER) {
                Ok((p, s)) => {
                    pid = pid.or(p);
                    playing = playing.or(s);
                }
                Err(_) => return None,
            }
        }
        let pid = pid?;
        (pid > 0).then_some((pid, playing?))
    }

    pub struct NowPlaying;

    impl super::Players for NowPlaying {
        fn playing(&mut self) -> Vec<String> {
            match now_playing() {
                Some((pid, true)) => vec![pid.to_string()],
                _ => vec![],
            }
        }

        fn pause(&mut self, _: &str) -> bool {
            media_remote().is_some_and(|mr| unsafe { (mr.send)(PAUSE, std::ptr::null()) != 0 })
        }

        fn is_paused(&mut self, id: &str) -> bool {
            matches!(now_playing(), Some((pid, false)) if pid.to_string() == id)
        }

        fn play(&mut self, _: &str) {
            if let Some(mr) = media_remote() {
                unsafe { (mr.send)(PLAY, std::ptr::null()) };
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_smtc {
    //! The system media transport controls: one session per app that plays media (a browser, Spotify).
    //! A session's id is its app's AppUserModelID.

    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSession as Session,
        GlobalSystemMediaTransportControlsSessionManager as Manager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
    };

    #[derive(Default)]
    pub struct Sessions {
        manager: Option<Manager>,
    }

    impl Sessions {
        fn sessions(&mut self) -> Vec<Session> {
            if self.manager.is_none() {
                match Manager::RequestAsync().and_then(|op| op.get()) {
                    Ok(m) => self.manager = Some(m),
                    Err(e) => {
                        log::info!("media: no media sessions, so nothing is paused: {e}");
                        return vec![];
                    }
                }
            }
            let Some(manager) = &self.manager else { return vec![] };
            match manager.GetSessions() {
                Ok(list) => list.into_iter().collect(),
                Err(e) => {
                    log::info!("media: could not list media sessions: {e}");
                    self.manager = None;
                    vec![]
                }
            }
        }

        fn with_status(&mut self, id: &str, status: PlaybackStatus) -> Vec<Session> {
            self.sessions()
                .into_iter()
                .filter(|s| s.SourceAppUserModelId().is_ok_and(|a| a == id))
                .filter(|s| s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).is_ok_and(|p| p == status))
                .collect()
        }
    }

    impl super::Players for Sessions {
        fn playing(&mut self) -> Vec<String> {
            let mut ids: Vec<String> = self
                .sessions()
                .into_iter()
                .filter(|s| s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).is_ok_and(|p| p == PlaybackStatus::Playing))
                .filter_map(|s| s.SourceAppUserModelId().ok().map(|a| a.to_string()))
                .collect();
            ids.dedup();
            ids
        }

        fn pause(&mut self, id: &str) -> bool {
            let mut paused = false;
            for s in self.with_status(id, PlaybackStatus::Playing) {
                paused |= s.TryPauseAsync().and_then(|op| op.get()).unwrap_or(false);
            }
            paused
        }

        fn is_paused(&mut self, id: &str) -> bool {
            !self.with_status(id, PlaybackStatus::Paused).is_empty()
        }

        fn play(&mut self, id: &str) {
            for s in self.with_status(id, PlaybackStatus::Paused) {
                let _ = s.TryPlayAsync().and_then(|op| op.get());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum State {
        Playing,
        Paused,
        Stopped,
    }

    /// Players in memory, with every call the ledger made.
    #[derive(Clone, Default)]
    struct Fake {
        players: Arc<Mutex<Vec<(String, State)>>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Fake {
        fn new(players: &[(&str, State)]) -> Self {
            let f = Fake::default();
            *f.players.lock() = players.iter().map(|(n, s)| (n.to_string(), *s)).collect();
            f
        }
        fn set(&self, id: &str, state: Option<State>) {
            let mut p = self.players.lock();
            p.retain(|(n, _)| n != id);
            if let Some(s) = state {
                p.push((id.into(), s));
            }
        }
        fn state(&self, id: &str) -> Option<State> {
            self.players.lock().iter().find(|(n, _)| n == id).map(|(_, s)| *s)
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().clone()
        }
    }

    impl Players for Fake {
        fn playing(&mut self) -> Vec<String> {
            self.players.lock().iter().filter(|(_, s)| *s == State::Playing).map(|(n, _)| n.clone()).collect()
        }
        fn pause(&mut self, id: &str) -> bool {
            self.calls.lock().push(format!("pause {id}"));
            let was = self.state(id);
            if was == Some(State::Playing) {
                self.set(id, Some(State::Paused));
            }
            was.is_some()
        }
        fn is_paused(&mut self, id: &str) -> bool {
            self.state(id) == Some(State::Paused)
        }
        fn play(&mut self, id: &str) {
            self.calls.lock().push(format!("play {id}"));
            self.set(id, Some(State::Playing));
        }
    }

    #[test]
    fn only_what_silas_paused_plays_again() {
        let mut f = Fake::new(&[("music", State::Playing), ("podcast", State::Paused), ("video", State::Stopped)]);
        let mut ledger = Ledger::default();
        ledger.pause(&mut f);
        assert_eq!(ledger.paused(), ["music"]);
        assert_eq!(f.state("music"), Some(State::Paused));
        ledger.resume(&mut f);
        assert_eq!(f.state("music"), Some(State::Playing));
        assert_eq!(f.state("podcast"), Some(State::Paused), "the user paused it; it stays paused");
        assert_eq!(f.state("video"), Some(State::Stopped));
        assert_eq!(f.calls(), ["pause music", "play music"]);
        assert!(ledger.paused().is_empty());
    }

    #[test]
    fn a_player_the_user_changed_while_dictating_is_left_alone() {
        let mut f = Fake::new(&[("a", State::Playing), ("b", State::Playing), ("c", State::Playing)]);
        let mut ledger = Ledger::default();
        ledger.pause(&mut f);
        // While dictating: the user starts a again themselves, stops b, and closes c.
        f.set("a", Some(State::Playing));
        f.set("b", Some(State::Stopped));
        f.set("c", None);
        ledger.resume(&mut f);
        assert_eq!(f.calls(), ["pause a", "pause b", "pause c"], "nothing is played again");
        assert_eq!(f.state("b"), Some(State::Stopped));
        assert_eq!(f.state("c"), None);
    }

    #[test]
    fn a_resume_with_nothing_paused_does_nothing() {
        let mut f = Fake::new(&[("podcast", State::Paused)]);
        let mut ledger = Ledger::default();
        ledger.resume(&mut f);
        ledger.pause(&mut f);
        ledger.resume(&mut f);
        assert!(f.calls().is_empty());
        assert_eq!(f.state("podcast"), Some(State::Paused));
    }

    #[test]
    fn a_pause_twice_still_plays_each_player_once() {
        let mut f = Fake::new(&[("music", State::Playing)]);
        let mut ledger = Ledger::default();
        ledger.pause(&mut f);
        f.set("music", Some(State::Playing));
        ledger.pause(&mut f);
        assert_eq!(ledger.paused(), ["music"]);
        ledger.resume(&mut f);
        assert_eq!(f.calls().iter().filter(|c| *c == "play music").count(), 1);
    }

    static SLOW: std::sync::OnceLock<Fake> = std::sync::OnceLock::new();

    /// A player that takes half a second to answer anything.
    struct Slow(Fake);
    impl Players for Slow {
        fn playing(&mut self) -> Vec<String> {
            std::thread::sleep(Duration::from_millis(500));
            self.0.playing()
        }
        fn pause(&mut self, id: &str) -> bool {
            self.0.pause(id)
        }
        fn is_paused(&mut self, id: &str) -> bool {
            self.0.is_paused(id)
        }
        fn play(&mut self, id: &str) {
            self.0.play(id)
        }
    }

    fn wait_for(what: impl Fn() -> bool) -> bool {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < until {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn pausing_never_holds_up_the_key_and_resume_follows_in_order() {
        let fake = SLOW.get_or_init(|| Fake::new(&[("music", State::Playing)])).clone();
        let media = MediaPause::with(|| Box::new(Slow(SLOW.get().unwrap().clone())));
        let t = std::time::Instant::now();
        media.pause();
        media.resume(Duration::ZERO);
        assert!(t.elapsed() < Duration::from_millis(50), "the key thread waited {:?}", t.elapsed());
        // The resume waits for the slow pause before it, so the music comes back.
        assert!(wait_for(|| fake.calls() == ["pause music", "play music"]), "{:?}", fake.calls());
    }

    static QUICK: std::sync::OnceLock<Fake> = std::sync::OnceLock::new();

    #[test]
    fn a_new_dictation_inside_the_tail_keeps_the_music_paused() {
        let fake = QUICK.get_or_init(|| Fake::new(&[("music", State::Playing)])).clone();
        let media = MediaPause::with(|| Box::new(QUICK.get().unwrap().clone()));
        media.pause();
        media.resume(Duration::from_millis(300));
        media.pause();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(fake.calls(), ["pause music"], "no blip of music between the two");
        media.resume(Duration::ZERO);
        assert!(wait_for(|| fake.state("music") == Some(State::Playing)));
    }

    #[test]
    fn a_resume_before_any_pause_starts_nothing() {
        let media = MediaPause::with(|| -> Box<dyn Players> { panic!("no player is ever asked") });
        media.resume(Duration::ZERO);
        assert!(media.tx.lock().is_none());
    }
}
