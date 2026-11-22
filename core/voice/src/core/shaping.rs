//! The step in `write` between the dictionary and the typer: the app the text goes to picks a
//! style (`style.rs`), which shapes the speech; snippets are typed as saved (`snippets.rs`).

use super::{Core, Shared};
use crate::focus::Target;
use crate::store::Settings;
use crate::style::{self, AppInfo, Style};

impl Core {
    /// The text to type and the settings to type it with (the style decides Shift+Return).
    pub(super) fn shape(self: &Shared, target: Option<&Target>, text: &str, mut s: Settings) -> (String, Settings) {
        let style = self.note_app(target).map_or(Style::Prose, |a| a.style);
        let snippets = self.dict.lock().snippets.clone();
        let text = style::render(text, style, &snippets);
        s.newline_as_shift_enter = style.shift_enter(s.newline_as_shift_enter);
        (text, s)
    }

    /// The app the text goes to: the one that had the keyboard at key-down where the system can
    /// say, else the one that has it now. Kept for Settings and sent as the `app` event.
    pub(super) fn note_app(&self, target: Option<&Target>) -> Option<AppInfo> {
        let id = target.and_then(Target::app).or_else(crate::focus::focused_app)?;
        let info = style::describe(&id, &self.settings.lock().app_styles);
        *self.app.lock() = Some(id);
        self.host.emit("app", serde_json::json!(&info));
        Some(info)
    }

    /// The app the last dictation went to, with the style it gets now. None before the first
    /// dictation, or where the system doesn't name apps (Linux outside Hyprland).
    pub fn last_app(&self) -> Option<AppInfo> {
        let id = self.app.lock().clone()?;
        Some(style::describe(&id, &self.settings.lock().app_styles))
    }
}
