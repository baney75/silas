//! App styles: how Silas shapes text for the app it types into.
//!
//! Silas knows the focused app (`focus::focused_app`: a macOS bundle id, a Windows exe name, a
//! Hyprland window class) and picks a style for it: the user's choice in Settings first, else the
//! built-in table below, else Prose.
//!
//! | Style | Spoken code | Trailing period | Capital at the start | New line |
//! | --- | --- | --- | --- | --- |
//! | Prose | off | kept | kept | the Shift+Return setting |
//! | Terminal | shell (`NAME=value`) | dropped | lower case | the Shift+Return setting |
//! | Code | code (`x = 1`) | dropped when it reads as code | kept | the Shift+Return setting |
//! | Chat | off | kept | kept | always Shift+Return |

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::snippets::{self, Seg, Snippet};
use crate::spoken::{self, Flavor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Prose,
    Terminal,
    Code,
    Chat,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Prose, Style::Terminal, Style::Code, Style::Chat];

    /// The settings name: "prose", "terminal", "code", "chat".
    pub fn name(self) -> &'static str {
        match self {
            Style::Prose => "prose",
            Style::Terminal => "terminal",
            Style::Code => "code",
            Style::Chat => "chat",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.name() == s.trim().to_lowercase())
    }

    fn spoken_code(self) -> Option<Flavor> {
        match self {
            Style::Terminal => Some(Flavor::Shell),
            Style::Code => Some(Flavor::Code),
            Style::Prose | Style::Chat => None,
        }
    }

    /// Whether a new line goes out as Shift+Return. Chat boxes send on Return, so Chat always
    /// uses Shift+Return; elsewhere the user's setting decides.
    pub fn shift_enter(self, setting: bool) -> bool {
        self == Style::Chat || setting
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

/// An app Silas knows without being told.
#[derive(Debug, Clone, Copy)]
pub struct KnownApp {
    pub os: Os,
    /// macOS bundle id, Windows exe name (lower case), or Hyprland window class.
    pub id: &'static str,
    pub name: &'static str,
    pub style: Style,
}

const fn app(os: Os, id: &'static str, name: &'static str, style: Style) -> KnownApp {
    KnownApp { os, id, name, style }
}

use Os::{Linux, Mac, Windows};
use Style::{Chat, Code, Terminal};

/// Every app with a built-in style, for every OS. Anything not here is Prose until the user
/// assigns it. Ids are compared without regard to case.
pub const KNOWN: &[KnownApp] = &[
    // ---- macOS bundle ids
    app(Mac, "com.apple.Terminal", "Terminal", Terminal),
    app(Mac, "com.googlecode.iterm2", "iTerm2", Terminal),
    app(Mac, "com.mitchellh.ghostty", "Ghostty", Terminal),
    app(Mac, "net.kovidgoyal.kitty", "kitty", Terminal),
    app(Mac, "org.alacritty", "Alacritty", Terminal),
    app(Mac, "com.github.wez.wezterm", "WezTerm", Terminal),
    app(Mac, "dev.warp.Warp-Stable", "Warp", Terminal),
    app(Mac, "com.microsoft.VSCode", "Visual Studio Code", Code),
    app(Mac, "com.microsoft.VSCodeInsiders", "VS Code Insiders", Code),
    app(Mac, "com.todesktop.230313mzl4w4u92", "Cursor", Code),
    app(Mac, "dev.zed.Zed", "Zed", Code),
    app(Mac, "com.apple.dt.Xcode", "Xcode", Code),
    app(Mac, "com.sublimetext.4", "Sublime Text", Code),
    app(Mac, "com.jetbrains.intellij", "IntelliJ IDEA", Code),
    app(Mac, "com.jetbrains.pycharm", "PyCharm", Code),
    app(Mac, "com.jetbrains.WebStorm", "WebStorm", Code),
    app(Mac, "com.google.android.studio", "Android Studio", Code),
    app(Mac, "com.tinyspeck.slackmacgap", "Slack", Chat),
    app(Mac, "com.hnc.Discord", "Discord", Chat),
    app(Mac, "com.apple.MobileSMS", "Messages", Chat),
    app(Mac, "ru.keepcoder.Telegram", "Telegram", Chat),
    app(Mac, "net.whatsapp.WhatsApp", "WhatsApp", Chat),
    app(Mac, "org.whispersystems.signal-desktop", "Signal", Chat),
    app(Mac, "com.microsoft.teams2", "Microsoft Teams", Chat),
    // ---- Windows exe names
    app(Windows, "windowsterminal.exe", "Windows Terminal", Terminal),
    app(Windows, "cmd.exe", "Command Prompt", Terminal),
    app(Windows, "powershell.exe", "PowerShell", Terminal),
    app(Windows, "pwsh.exe", "PowerShell", Terminal),
    app(Windows, "wezterm-gui.exe", "WezTerm", Terminal),
    app(Windows, "alacritty.exe", "Alacritty", Terminal),
    app(Windows, "mintty.exe", "Git Bash", Terminal),
    app(Windows, "code.exe", "Visual Studio Code", Code),
    app(Windows, "code - insiders.exe", "VS Code Insiders", Code),
    app(Windows, "cursor.exe", "Cursor", Code),
    app(Windows, "zed.exe", "Zed", Code),
    app(Windows, "devenv.exe", "Visual Studio", Code),
    app(Windows, "sublime_text.exe", "Sublime Text", Code),
    app(Windows, "idea64.exe", "IntelliJ IDEA", Code),
    app(Windows, "pycharm64.exe", "PyCharm", Code),
    app(Windows, "webstorm64.exe", "WebStorm", Code),
    app(Windows, "slack.exe", "Slack", Chat),
    app(Windows, "discord.exe", "Discord", Chat),
    app(Windows, "ms-teams.exe", "Microsoft Teams", Chat),
    app(Windows, "telegram.exe", "Telegram", Chat),
    app(Windows, "whatsapp.exe", "WhatsApp", Chat),
    app(Windows, "signal.exe", "Signal", Chat),
    // ---- Linux (Hyprland window classes)
    app(Linux, "com.mitchellh.ghostty", "Ghostty", Terminal),
    app(Linux, "kitty", "kitty", Terminal),
    app(Linux, "Alacritty", "Alacritty", Terminal),
    app(Linux, "foot", "foot", Terminal),
    app(Linux, "footclient", "foot", Terminal),
    app(Linux, "org.wezfurlong.wezterm", "WezTerm", Terminal),
    app(Linux, "org.kde.konsole", "Konsole", Terminal),
    app(Linux, "org.gnome.Terminal", "GNOME Terminal", Terminal),
    app(Linux, "org.gnome.Ptyxis", "Ptyxis", Terminal),
    app(Linux, "com.gexperts.Tilix", "Tilix", Terminal),
    app(Linux, "XTerm", "xterm", Terminal),
    app(Linux, "code", "Visual Studio Code", Code),
    app(Linux, "code-oss", "Code - OSS", Code),
    app(Linux, "cursor", "Cursor", Code),
    app(Linux, "dev.zed.Zed", "Zed", Code),
    app(Linux, "neovide", "Neovide", Code),
    app(Linux, "sublime_text", "Sublime Text", Code),
    app(Linux, "jetbrains-idea", "IntelliJ IDEA", Code),
    app(Linux, "jetbrains-pycharm", "PyCharm", Code),
    app(Linux, "Slack", "Slack", Chat),
    app(Linux, "discord", "Discord", Chat),
    app(Linux, "vesktop", "Vesktop", Chat),
    app(Linux, "signal", "Signal", Chat),
    app(Linux, "org.telegram.desktop", "Telegram", Chat),
    app(Linux, "Element", "Element", Chat),
];

/// The built-in entry for an app id, from any OS.
pub fn known(id: &str) -> Option<&'static KnownApp> {
    let id = id.trim();
    KNOWN.iter().find(|a| a.id.eq_ignore_ascii_case(id))
}

/// An app as Settings shows it: its id, a name, the style in use and the built-in one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub style: Style,
    pub builtin: Style,
}

/// The style for `id`: the user's assignment (`Settings::app_styles`), else the table, else Prose.
pub fn describe(id: &str, assigned: &BTreeMap<String, String>) -> AppInfo {
    let builtin = known(id).map_or(Style::Prose, |a| a.style);
    let chosen = assigned
        .iter()
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(id.trim()))
        .and_then(|(_, v)| Style::parse(v));
    AppInfo {
        id: id.to_string(),
        name: known(id).map_or_else(|| id.to_string(), |a| a.name.to_string()),
        style: chosen.unwrap_or(builtin),
        builtin,
    }
}

/// The text to type for `style`: snippets first (typed exactly as saved), then the style's
/// spoken code, trailing period and first capital on the speech around them.
pub fn render(text: &str, style: Style, snippets: &[Snippet]) -> String {
    let segs = snippets::expand(text, snippets);
    let count = segs.len();
    let mut out = String::new();
    for (k, seg) in segs.into_iter().enumerate() {
        let speech = match seg {
            Seg::Snippet(t) => {
                out.push_str(&t);
                continue;
            }
            Seg::Speech(t) => t,
        };
        // Keep the spaces around a piece that sits next to a snippet.
        let lead = &speech[..speech.len() - speech.trim_start().len()];
        let trail = &speech[speech.trim_end().len()..];
        let mut body = speech.trim().to_string();
        if let Some(flavor) = style.spoken_code() {
            body = spoken::format(&body, flavor);
        }
        if k == 0 && style == Style::Terminal {
            body = lower_start(&body);
        }
        let ends = k + 1 == count;
        if ends && (style == Style::Terminal || (style == Style::Code && spoken::reads_as_code(&body))) {
            body = drop_period(&body);
        }
        out.push_str(lead);
        out.push_str(&body);
        out.push_str(trail);
    }
    out
}

/// "Cargo test" → "cargo test". A word with other capitals (README, NODE_ENV, GitHub) and "I"
/// keep theirs.
fn lower_start(text: &str) -> String {
    let word: String = text.chars().take_while(|c| c.is_alphanumeric() || *c == '\'').collect();
    let mut chars = word.chars();
    let Some(first) = chars.next() else { return text.to_string() };
    let rest_lower = chars.all(|c| !c.is_uppercase());
    if !first.is_uppercase() || !rest_lower || word == "I" || word.starts_with("I'") {
        return text.to_string();
    }
    first.to_lowercase().chain(text.chars().skip(1)).collect()
}

/// One final period goes; an ellipsis stays.
fn drop_period(text: &str) -> String {
    if text.ends_with('.') && !text.ends_with("..") {
        text[..text.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_known_id_is_listed_once_per_os_and_agrees_across_oses() {
        for os in [Os::Mac, Os::Windows, Os::Linux] {
            let mut seen = HashSet::new();
            for a in KNOWN.iter().filter(|a| a.os == os) {
                assert!(seen.insert(a.id.to_lowercase()), "{} is listed twice for {os:?}", a.id);
                assert_eq!(a.id, a.id.trim());
            }
            for style in [Style::Terminal, Style::Code, Style::Chat] {
                assert!(KNOWN.iter().any(|a| a.os == os && a.style == style), "{os:?} has no {style:?} app");
            }
        }
        // The same id on two systems (Ghostty) must mean the same style, since lookup ignores the OS.
        for a in KNOWN {
            assert_eq!(known(a.id).map(|k| k.style), Some(a.style), "{}", a.id);
        }
        // Windows ids are exe names in lower case, as `focus::exe_name` reports them.
        for a in KNOWN.iter().filter(|a| a.os == Os::Windows) {
            assert!(a.id.ends_with(".exe") && a.id == a.id.to_lowercase(), "{}", a.id);
        }
    }

    #[test]
    fn known_apps_get_their_style_and_the_rest_get_prose() {
        let none = BTreeMap::new();
        assert_eq!(describe("com.mitchellh.ghostty", &none).style, Style::Terminal);
        assert_eq!(describe("COM.APPLE.TERMINAL", &none).style, Style::Terminal);
        assert_eq!(describe("WindowsTerminal.exe", &none).style, Style::Terminal);
        assert_eq!(describe("Code", &none).style, Style::Code);
        assert_eq!(describe("com.microsoft.VSCode", &none).name, "Visual Studio Code");
        assert_eq!(describe("com.tinyspeck.slackmacgap", &none).style, Style::Chat);
        let other = describe("org.mozilla.firefox", &none);
        assert_eq!((other.style, other.builtin, other.name.as_str()), (Style::Prose, Style::Prose, "org.mozilla.firefox"));
    }

    #[test]
    fn the_users_choice_beats_the_table() {
        let mut mine = BTreeMap::new();
        mine.insert("Code".to_string(), "prose".to_string());
        mine.insert("org.mozilla.firefox".to_string(), "chat".to_string());
        mine.insert("kitty".to_string(), "nonsense".to_string());
        let code = describe("code", &mine);
        assert_eq!((code.style, code.builtin), (Style::Prose, Style::Code));
        assert_eq!(describe("org.mozilla.firefox", &mine).style, Style::Chat);
        // A value that isn't a style falls back to the table.
        assert_eq!(describe("kitty", &mine).style, Style::Terminal);
    }

    #[test]
    fn terminal_types_commands_without_the_period_or_the_capital() {
        let r = |s| render(s, Style::Terminal, &[]);
        assert_eq!(r("Cargo test dash dash release dash dash features bench."), "cargo test --release --features bench");
        assert_eq!(r("Export all caps node underscore env equals production."), "export NODE_ENV=production");
        assert_eq!(r("Git status."), "git status");
        assert_eq!(r("I think the build is broken."), "I think the build is broken");
        assert_eq!(r("README first."), "README first");
        assert_eq!(r("Wait..."), "wait...");
    }

    #[test]
    fn code_keeps_sentences_and_drops_the_period_after_code() {
        let r = |s| render(s, Style::Code, &[]);
        assert_eq!(r("Open src slash main dot rs."), "Open src/main.rs");
        assert_eq!(r("This function returns the sum."), "This function returns the sum.");
        assert_eq!(r("Let camel case total count equals zero."), "Let totalCount = zero");
    }

    #[test]
    fn prose_and_chat_type_what_was_said() {
        for style in [Style::Prose, Style::Chat] {
            let s = "Run cargo test dash dash release.";
            assert_eq!(render(s, style, &[]), s);
        }
        assert!(Style::Chat.shift_enter(false));
        assert!(!Style::Terminal.shift_enter(false));
        assert!(Style::Prose.shift_enter(true));
    }

    #[test]
    fn snippets_are_typed_as_saved_in_every_style() {
        let snips = vec![Snippet { trigger: "deploy".into(), text: "Run ./deploy.sh --prod.".into(), added: String::new() }];
        for style in Style::ALL {
            assert_eq!(render("Deploy.", style, &snips), "Run ./deploy.sh --prod.", "{style:?}");
        }
        assert_eq!(render("Now insert deploy please.", Style::Terminal, &snips), "now Run ./deploy.sh --prod. please");
        assert_eq!(Style::parse(" Chat "), Some(Style::Chat));
        assert_eq!(Style::parse("loud"), None);
    }
}
