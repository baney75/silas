//! Spoken code: "cargo test dash dash release" becomes `cargo test --release`, "all caps node
//! underscore env equals production" becomes `NODE_ENV=production`.
//!
//! A pure pass over the text the dictionary produced. It runs only for the Terminal and Code
//! editor styles (`style.rs`), and every rule looks at the words on either side, so ordinary
//! prose said in a code app keeps its words: "a dash of salt", "to underscore the point",
//! "the new line of laptops" and "he wrote it in all caps." stay as they were.

/// Where the code goes. A shell wants `NAME=value`; an editor wants `name = value`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Flavor {
    Shell,
    Code,
}

/// Words that mark prose. A symbol word next to one of these is a word, not a symbol.
const STOP: &[&str] = &[
    "a", "an", "the", "this", "that", "these", "those", "some", "any", "each", "every", "my", "your",
    "his", "her", "its", "our", "their", "to", "of", "for", "in", "on", "at", "by", "with", "from",
    "into", "onto", "off", "out", "up", "down", "over", "under", "about", "as", "and", "or", "but",
    "nor", "so", "if", "then", "than", "is", "are", "was", "were", "be", "been", "am", "do", "does",
    "did", "has", "have", "had", "will", "would", "can", "could", "should", "may", "might", "must",
    "not", "no", "it", "i", "you", "he", "she", "we", "they", "me", "him", "us", "them", "what",
    "which", "who", "when", "where", "why", "how", "there", "here", "very", "just", "also", "too",
    "i'll", "i'm", "it's", "we'll", "you'll", "let's", "quite", "such",
];

/// Commands that take a path or `.` right after them: "cd slash etc" is `cd /etc`, not `cd/etc`.
const PATH_COMMANDS: &[&str] = &[
    "cd", "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "vim", "nvim", "vi", "nano",
    "less", "more", "head", "tail", "source", "open", "code", "cursor", "zed", "chmod", "chown",
    "bat", "add", "stat", "du", "tree", "pushd",
];

/// The words that name a symbol on their own. Used to look ahead ("slash etc slash hosts").
const SYMBOL_WORDS: &[&str] = &[
    "dash", "dot", "slash", "backslash", "underscore", "tilde", "equals", "pipe",
];

fn stop(key: &str) -> bool {
    STOP.contains(&key)
}

fn is_symbol_word(key: &str) -> bool {
    SYMBOL_WORDS.contains(&key)
}

#[derive(Debug, Clone)]
struct Word {
    pre: String,
    core: String,
    post: String,
    key: String,
}

impl Word {
    fn whole(&self) -> String {
        format!("{}{}{}", self.pre, self.core, self.post)
    }
}

fn words(text: &str) -> Vec<Word> {
    text.split_whitespace()
        .map(|tok| {
            let start = tok.find(|c: char| !"\"'([{".contains(c)).unwrap_or(tok.len());
            let end = tok
                .rfind(|c: char| !".,;:!?\"')]}".contains(c))
                .map_or(start, |i| i + tok[i..].chars().next().map_or(1, char::len_utf8));
            let (pre, core, post) = if start >= end {
                ("", tok, "")
            } else {
                (&tok[..start], &tok[start..end], &tok[end..])
            };
            Word { pre: pre.into(), core: core.into(), post: post.into(), key: core.to_lowercase() }
        })
        .collect()
}

#[derive(Debug)]
struct Piece {
    text: String,
    /// A space goes before it.
    space: bool,
}

#[derive(Default)]
struct Out {
    pieces: Vec<Piece>,
    /// The last symbol attaches to whatever comes next.
    glue: bool,
    /// "all caps": the next word, and the words joined to it by `_`, go upper case.
    upper_next: bool,
    upper: Vec<usize>,
}

impl Out {
    fn word(&mut self, text: String) {
        if self.upper_next {
            self.upper.push(self.pieces.len());
            self.upper_next = false;
        }
        self.pieces.push(Piece { text, space: !self.glue });
        self.glue = false;
    }

    /// `left`: no space before it. `right`: the next piece attaches with no space.
    fn sym(&mut self, text: &str, left: bool, right: bool) {
        self.pieces.push(Piece { text: text.into(), space: !(left || self.glue) });
        self.glue = right;
    }

    fn finish(mut self) -> String {
        for &start in &self.upper {
            let mut i = start;
            while i < self.pieces.len() {
                if i > start {
                    let p = &self.pieces[i];
                    let after_underscore = self.pieces[i - 1].text == "_";
                    if p.space || !(p.text == "_" || after_underscore) {
                        break;
                    }
                }
                self.pieces[i].text = self.pieces[i].text.to_uppercase();
                i += 1;
            }
        }
        let mut s = String::new();
        for p in &self.pieces {
            if p.space && !s.is_empty() && !s.ends_with('\n') {
                s.push(' ');
            }
            s.push_str(&p.text);
        }
        s
    }
}

/// Turn spoken symbols into code. Text with nothing to change comes back as it was.
pub fn format(text: &str, flavor: Flavor) -> String {
    let w = words(text);
    if !w.iter().any(|w| TRIGGERS.contains(&w.key.as_str())) {
        return text.to_string();
    }
    let mut out = Out::default();
    let mut i = 0;
    while i < w.len() {
        match rule(&w, i, flavor, &mut out) {
            0 => {
                out.word(w[i].whole());
                i += 1;
            }
            used => i += used,
        }
    }
    out.finish()
}

/// Any word that can start a rule. Text without one is returned untouched.
const TRIGGERS: &[&str] = &[
    "dash", "double", "dot", "slash", "forward", "back", "backslash", "underscore", "tilde",
    "equals", "pipe", "plus", "at", "dollar", "open", "close", "left", "right", "new", "newline",
    "all", "camel", "snake", "pascal", "kebab", "constant",
];

/// Apply the rule that starts at `w[i]`, if one does. Returns how many words it used (0: none).
fn rule(w: &[Word], i: usize, flavor: Flavor, out: &mut Out) -> usize {
    let key = |j: usize| w.get(j).map_or("", |x| x.key.as_str());
    let k0 = key(i);
    let k1 = key(i + 1);
    let prev = i.checked_sub(1).map(|j| &w[j]);
    let last = |j: usize| j + 1 == w.len();
    // A word joins the one before it only inside a sentence, never across ". ".
    let free_left = prev.is_some_and(|p| p.post.is_empty() && !stop(&p.key)) && w[i].pre.is_empty();
    // `n` words make the symbol; the word after them is free to attach to.
    let free_right = |n: usize| {
        w[i + n - 1].post.is_empty() && w.get(i + n).is_some_and(|x| !stop(&x.key) && x.pre.is_empty())
    };
    let prev_key = prev.map_or("", |p| p.key.as_str());
    let after_command = prev.is_some_and(|p| p.post.is_empty() && PATH_COMMANDS.contains(&p.key.as_str()));
    let determiner = matches!(prev_key, "a" | "an" | "the" | "your" | "my" | "our" | "their" | "his" | "her" | "this" | "that");

    // ---- case commands
    if k0 == "all" && matches!(k1, "caps" | "cap") && free_right(2) && w[i].post.is_empty() {
        out.upper_next = true;
        return 2;
    }
    if k1 == "case" && matches!(k0, "camel" | "snake" | "pascal" | "kebab" | "constant") && w[i].post.is_empty() && !determiner {
        return case_run(w, i, k0, out);
    }

    // ---- line breaks
    let line = match (k0, k1) {
        ("new", "line") => Some(("\n", 2)),
        ("new", "paragraph") => Some(("\n\n", 2)),
        ("newline", _) => Some(("\n", 1)),
        _ => None,
    };
    if let Some((brk, n)) = line {
        let prose_before = determiner
            || matches!(prev_key, "whole" | "entire" | "every" | "each" | "one" | "another" | "its" | "brand");
        let prose_after = matches!(key(i + n), "of" | "for" | "to" | "is" | "was" | "and" | "that" | "in" | "on");
        if !prose_before && !prose_after && w[i].post.is_empty() || (!prose_before && last(i + n - 1)) {
            out.pieces.push(Piece { text: brk.into(), space: false });
            out.glue = true;
            return n;
        }
        return 0;
    }

    // ---- two-word symbols
    if (k0 == "dash" && k1 == "dash") || (k0 == "double" && k1 == "dash") {
        if !w[i].post.is_empty() {
            return 0;
        }
        if free_right(2) {
            out.sym("--", out.glue, true);
            return 2;
        }
        if last(i + 1) {
            out.sym("--", out.glue, false);
            return 2;
        }
        return 0;
    }
    let prefix = match (k0, k1) {
        ("at", "sign") => Some("@"),
        ("dollar", "sign") => Some("$"),
        _ => None,
    };
    if let Some(sym) = prefix {
        if !determiner && w[i].post.is_empty() && free_right(2) {
            out.sym(sym, out.glue, true);
            return 2;
        }
        return 0;
    }
    let bracket = match (k0, k1) {
        ("open" | "left", "paren" | "parenthesis" | "parens") => Some(("(", true, true)),
        ("close" | "right", "paren" | "parenthesis" | "parens") => Some((")", false, true)),
        ("open" | "left", "bracket") => Some(("[", true, true)),
        ("close" | "right", "bracket") => Some(("]", false, true)),
        ("open" | "left", "brace" | "curly") => Some(("{", true, false)),
        ("close" | "right", "brace" | "curly") => Some(("}", false, false)),
        _ => None,
    };
    if let Some((sym, opens, hugs)) = bracket {
        if determiner || !w[i].post.is_empty() {
            return 0;
        }
        if opens {
            // `print(` hugs its name; `if (` doesn't.
            out.sym(sym, out.glue || (hugs && free_left), true);
        } else {
            out.sym(sym, hugs, false);
        }
        return 2;
    }
    if matches!((k0, k1), ("equals", "equals") | ("double", "equals")) {
        if (free_left || out.glue) && free_right(2) {
            spaced(out, "==", flavor);
            return 2;
        }
        return 0;
    }
    if matches!((k0, k1), ("forward", "slash") | ("back", "slash")) {
        let sym = if k0 == "back" { "\\" } else { "/" };
        return joiner(w, i, 2, sym, out, after_command);
    }

    // ---- one-word symbols
    match k0 {
        "dash" => dash(w, i, out),
        "dot" => {
            if matches!(prev_key, "polka" | "connect") {
                return 0;
            }
            joiner(w, i, 1, ".", out, after_command)
        }
        "slash" => joiner(w, i, 1, "/", out, after_command),
        "backslash" => joiner(w, i, 1, "\\", out, after_command),
        "underscore" => {
            let left = out.glue || free_left;
            let right = free_right(1);
            if left || right {
                out.sym("_", left, right);
                1
            } else {
                0
            }
        }
        "tilde" => {
            if determiner || !w[i].post.is_empty() && !last(i) {
                return 0;
            }
            out.sym("~", out.glue, false);
            1
        }
        "equals" => {
            if (free_left || out.glue) && free_right(1) {
                spaced(out, "=", flavor);
                1
            } else {
                0
            }
        }
        "pipe" => {
            let prose_prev = matches!(prev_key, "water" | "gas" | "drain" | "smoke" | "exhaust" | "copper");
            let prose_next = matches!(key(i + 1), "down" | "up" | "dream" | "dreams" | "organ" | "wrench" | "line" | "burst" | "bomb");
            if free_left && free_right(1) && !prose_prev && !prose_next {
                out.sym("|", false, false);
                1
            } else {
                0
            }
        }
        // `chmod +x`: only right after chmod, where "plus" can't be arithmetic.
        "plus" if prev_key == "chmod" => {
            let mode = key(i + 1);
            if !mode.is_empty() && mode.len() <= 3 && mode.chars().all(|c| "rwxst".contains(c)) && w[i].post.is_empty() {
                out.sym("+", false, true);
                1
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// `=` and `==`: glued in a shell (`NAME=value`), spaced in code (`x = 1`).
fn spaced(out: &mut Out, sym: &str, flavor: Flavor) {
    match flavor {
        Flavor::Shell => out.sym(sym, true, true),
        Flavor::Code => out.sym(sym, false, false),
    }
}

/// "dash": a flag before a short option (`ls -la`), a hyphen between two words (`kube-system`),
/// and a word in prose ("a dash of salt", "an em dash", "dash off a note").
fn dash(w: &[Word], i: usize, out: &mut Out) -> usize {
    if !w[i].post.is_empty() {
        return 0;
    }
    let Some(next) = w.get(i + 1) else { return 0 };
    if !next.pre.is_empty() {
        return 0;
    }
    let prev = i.checked_sub(1).map(|j| &w[j]);
    const PROSE_BEFORE: &[&str] = &[
        "em", "en", "made", "make", "makes", "making", "quick", "mad", "little", "slight", "final",
        "last", "short", "long", "yard",
    ];
    const PROSE_AFTER: &[&str] = &[
        "away", "back", "home", "cam", "cams", "board", "boards", "across", "through", "past",
        "around", "after", "forward", "ahead", "of", "off", "out", "to", "for", "into",
    ];
    if prev.is_some_and(|p| stop(&p.key) || PROSE_BEFORE.contains(&p.key.as_str())) {
        return 0;
    }
    let nk = next.key.as_str();
    if PROSE_AFTER.contains(&nk) {
        return 0;
    }
    let short = !nk.is_empty() && nk.chars().count() <= 2 && nk.chars().all(|c| c.is_ascii_alphanumeric());
    // "a" and "i" are words too: a flag only at the end, before a pause, or before another symbol.
    let lone_word_letter = matches!(nk, "a" | "i");
    let flag = short
        && if lone_word_letter {
            !next.post.is_empty() || w.get(i + 2).is_none_or(|x| is_symbol_word(&x.key))
        } else {
            !stop(nk)
        };
    if flag {
        out.sym("-", out.glue, true);
        return 1;
    }
    let joins = prev.is_some_and(|p| p.post.is_empty()) && !stop(nk);
    if joins {
        out.sym("-", true, true);
        return 1;
    }
    0
}

/// `.` `/` `\`: between two words (`build.sh`, `src/main`), after a path command with a space
/// (`cd /etc`, `git add .`), or starting a path that goes on ("slash etc slash hosts").
fn joiner(w: &[Word], i: usize, n: usize, sym: &str, out: &mut Out, after_command: bool) -> usize {
    let key = |j: usize| w.get(j).map_or("", |x| x.key.as_str());
    let prev = i.checked_sub(1).map(|j| &w[j]);
    let at_end = i + n == w.len();
    let own_post_ok = w[i + n - 1].post.is_empty() || at_end;
    if !own_post_ok {
        return 0;
    }
    let right = w[i + n - 1].post.is_empty()
        && w.get(i + n).is_some_and(|x| !stop(&x.key) && x.pre.is_empty());
    let free_left = prev.is_some_and(|p| p.post.is_empty() && !stop(&p.key)) && w[i].pre.is_empty();
    if out.glue && (right || at_end) {
        out.sym(sym, true, right);
        return n;
    }
    if after_command && (right || at_end) {
        out.sym(sym, false, right);
        return n;
    }
    if free_left && right {
        out.sym(sym, true, true);
        return n;
    }
    // A path said from its start: only when another symbol follows its first part.
    let sentence_start = prev.is_none_or(|p| !p.post.is_empty());
    if sentence_start && right && is_symbol_word(key(i + n + 1)) {
        out.sym(sym, false, true);
        return n;
    }
    0
}

/// "camel case user name" → `userName`; snake, pascal, kebab and constant likewise. Takes up to
/// six words, stopping at a pause, a small word or a symbol word.
fn case_run(w: &[Word], i: usize, style: &str, out: &mut Out) -> usize {
    let mut parts: Vec<&Word> = Vec::new();
    let mut j = i + 2;
    while j < w.len() && parts.len() < 6 {
        let x = &w[j];
        if stop(&x.key) || is_symbol_word(&x.key) || !x.pre.is_empty() || !x.core.chars().all(|c| c.is_alphanumeric()) {
            break;
        }
        parts.push(x);
        j += 1;
        if !x.post.is_empty() {
            break;
        }
    }
    if parts.is_empty() {
        return 0;
    }
    let lower: Vec<String> = parts.iter().map(|p| p.core.to_lowercase()).collect();
    let cap = |s: &str| {
        let mut c = s.chars();
        c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
    };
    let joined = match style {
        "camel" => lower.iter().enumerate().map(|(n, s)| if n == 0 { s.clone() } else { cap(s) }).collect(),
        "pascal" => lower.iter().map(|s| cap(s)).collect(),
        "snake" => lower.join("_"),
        "kebab" => lower.join("-"),
        _ => lower.join("_").to_uppercase(),
    };
    let post = parts.last().map_or(String::new(), |p| p.post.clone());
    out.word(joined + &post);
    2 + parts.len()
}

/// True when the text reads as a command, path or code rather than a sentence: a word with a
/// code symbol in it, a flag, or a dotted name like `build.sh`.
pub fn reads_as_code(text: &str) -> bool {
    text.split_whitespace().any(|tok| {
        let tok = tok.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if tok.starts_with('-') && tok.len() > 1 {
            return true;
        }
        if tok.contains(['_', '/', '\\', '=', '|', '~', '@', '(', ')', '[', ']', '{', '}', '$']) {
            return true;
        }
        let b = tok.as_bytes();
        (1..b.len().saturating_sub(1)).any(|k| b[k] == b'.' && b[k - 1].is_ascii_alphanumeric() && b[k + 1].is_ascii_alphabetic())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(s: &str) -> String {
        format(s, Flavor::Shell)
    }
    fn code(s: &str) -> String {
        format(s, Flavor::Code)
    }

    #[test]
    fn dash_dash_becomes_a_long_flag() {
        assert_eq!(shell("Cargo test dash dash release dash dash features bench."), "Cargo test --release --features bench.");
        assert_eq!(shell("git checkout dash dash"), "git checkout --");
        assert_eq!(shell("npm run build double dash watch"), "npm run build --watch");
    }

    #[test]
    fn a_dash_before_a_short_option_is_a_flag_and_between_words_a_hyphen() {
        assert_eq!(shell("kubectl get pods dash n kube dash system"), "kubectl get pods -n kube-system");
        assert_eq!(shell("ls dash la"), "ls -la");
        assert_eq!(shell("ls dash a"), "ls -a");
        assert_eq!(shell("git commit dash m"), "git commit -m");
        assert_eq!(shell("dash dash dry dash run"), "--dry-run");
    }

    #[test]
    fn underscore_dot_slash_and_all_caps_build_names() {
        assert_eq!(shell("Export all caps node underscore env equals production."), "Export NODE_ENV=production.");
        assert_eq!(shell("chmod plus x build dot sh"), "chmod +x build.sh");
        assert_eq!(shell("cd slash etc slash hosts"), "cd /etc/hosts");
        assert_eq!(shell("slash etc slash hosts"), "/etc/hosts");
        assert_eq!(shell("cd tilde slash projects"), "cd ~/projects");
        assert_eq!(shell("cd tilde slash dot config"), "cd ~/.config");
        assert_eq!(shell("cd dot dot"), "cd ..");
        assert_eq!(shell("git add dot"), "git add .");
        assert_eq!(shell("cat dot env"), "cat .env");
        assert_eq!(code("open src slash main dot rs"), "open src/main.rs");
        assert_eq!(code("underscore underscore init underscore underscore"), "__init__");
        assert_eq!(code("example dot com"), "example.com");
        assert_eq!(code("all caps readme dot md"), "README.md");
    }

    #[test]
    fn equals_pipe_brackets_and_prefixes() {
        assert_eq!(code("count equals five"), "count = five");
        assert_eq!(code("if x double equals y"), "if x == y");
        assert_eq!(shell("ps aux pipe grep silas"), "ps aux | grep silas");
        assert_eq!(code("print open paren name close paren"), "print(name)");
        assert_eq!(code("if open paren ready close paren"), "if (ready)");
        assert_eq!(code("items open bracket zero close bracket"), "items[zero]");
        assert_eq!(code("fn main open paren close paren open brace"), "fn main() {");
        assert_eq!(shell("ping at sign baney"), "ping @baney");
        assert_eq!(shell("echo dollar sign home"), "echo $home");
        assert_eq!(code("path back slash bin"), "path\\bin");
        assert_eq!(code("path forward slash bin"), "path/bin");
    }

    #[test]
    fn case_commands_join_the_words_that_follow() {
        assert_eq!(code("let camel case user name equals one"), "let userName = one");
        assert_eq!(code("snake case max retry count"), "max_retry_count");
        assert_eq!(code("pascal case http client"), "HttpClient");
        assert_eq!(code("kebab case main nav"), "main-nav");
        assert_eq!(code("constant case max size"), "MAX_SIZE");
        assert_eq!(code("camel case user name, then save"), "userName, then save");
    }

    #[test]
    fn new_line_and_new_paragraph_break_the_text() {
        assert_eq!(code("first item new line second item"), "first item\nsecond item");
        assert_eq!(code("Thanks. New paragraph. See you"), "Thanks.\n\nSee you");
    }

    #[test]
    fn ordinary_prose_is_left_alone() {
        for s in [
            "Add a dash of salt and stir.",
            "I need to dash to the store.",
            "Use an em dash here, not a hyphen.",
            "She made a quick dash for the door.",
            "Dash off a note to Sam.",
            "We want to underscore the point.",
            "The new line of laptops is out.",
            "Start a new line for each name.",
            "He wrote it in all caps.",
            "We type it in all caps and bold.",
            "Java uses camel case for names.",
            "She wore a polka dot dress.",
            "Connect the dots on the map.",
            "Be there on the dot.",
            "Two plus two is four.",
            "A water pipe burst in the basement.",
            "Pipe down, please.",
            "He likes to slash prices.",
            "Slash the budget by half.",
            "Meet me at the sign by the road.",
            "The dollar sign is on the key.",
            "Open the door and close the window.",
            "Everything equals the sum of its parts.",
            "Look at the tilde key.",
            "That was a close call.",
            "She wrote one new line.",
        ] {
            assert_eq!(shell(s), s, "shell changed prose: {s}");
            assert_eq!(code(s), s, "code changed prose: {s}");
        }
    }

    #[test]
    fn code_shapes_are_recognized_and_sentences_are_not() {
        for s in ["cargo test --release", "export NODE_ENV=production.", "cd ~/projects", "chmod +x build.sh.", "ls | wc", "print(name)"] {
            assert!(reads_as_code(s), "{s}");
        }
        for s in ["Send it tomorrow.", "Well-known names stay.", "It costs 3.50 today.", "Done!"] {
            assert!(!reads_as_code(s), "{s}");
        }
    }
}
