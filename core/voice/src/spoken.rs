//! Spoken code: "cargo test dash dash release" becomes `cargo test --release`, "all caps node
//! underscore env equals production" becomes `NODE_ENV=production`.
//!
//! A pure pass over the text the dictionary produced. It runs only for the Terminal and Code
//! editor styles (`style.rs`), and every rule looks at the words on either side, so ordinary
//! prose said in a code app keeps its words: "a dash of salt", "to underscore the point",
//! "the new line of laptops" and "he wrote it in all caps." stay as they were.
//!
//! The one-word symbols that are also English words (underscore, equals, pipe, dot, tilde) and
//! "all caps" need a code signal as well: another symbol or rule next to them, a line that
//! starts with a command, or a neighbour that already looks like code. "Results underscore the
//! need" and "a red dot appears" stay prose. A line break never comes from spoken code in a
//! terminal, where it is Return and would run a half-dictated command.

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
/// The ones in `LOOSE_COMMANDS` are English words too, and count only at the start of a line
/// or after another command ("git add dot", not "I need more dot points").
const PATH_COMMANDS: &[&str] = &[
    "cd", "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "vim", "nvim", "vi", "nano",
    "less", "more", "head", "tail", "source", "open", "code", "cursor", "zed", "chmod", "chown",
    "bat", "add", "stat", "du", "tree", "pushd",
];

const LOOSE_COMMANDS: &[&str] = &[
    "open", "code", "cursor", "zed", "add", "less", "more", "head", "tail", "touch", "source", "stat",
    "tree",
];

/// Commands that mark a line as a command line when they start it.
const LINE_COMMANDS: &[&str] = &[
    "cd", "ls", "git", "npm", "npx", "pnpm", "yarn", "bun", "bunx", "cargo", "rustup", "rustc",
    "python3", "pip", "pip3", "uv", "deno", "docker", "kubectl", "helm", "ssh", "scp", "rsync",
    "curl", "wget", "grep", "rg", "sed", "awk", "jq", "ps", "sudo", "brew", "apt", "chmod", "chown",
    "mkdir", "rmdir", "rm", "mv", "cp", "vim", "nvim", "export", "tar", "gh", "cat",
];

/// Commands that read from a pipe: "pipe grep" is `| grep`.
const PIPE_TARGETS: &[&str] = &[
    "grep", "rg", "wc", "sort", "uniq", "xargs", "jq", "head", "tail", "less", "awk", "sed", "tee",
    "cut", "tr", "fzf", "cat", "bat", "column",
];

/// What follows a dot in a file or host name: "build dot sh", "example dot com".
const EXTENSIONS: &[&str] = &[
    "sh", "rs", "md", "py", "js", "ts", "tsx", "jsx", "mjs", "cjs", "json", "toml", "yaml", "yml",
    "txt", "html", "css", "com", "org", "net", "io", "dev", "lock", "csv", "sql", "swift",
    "kt", "rb", "cpp", "hpp", "java", "xml", "ini", "cfg", "conf", "config", "zsh", "bash", "vue",
    "svelte", "lua", "wasm", "png", "svg", "pdf", "zip", "gz", "gitignore",
];

/// Files whose name starts with a dot: "use dot env files" is `use .env files`.
const DOTFILES: &[&str] = &[
    "env", "gitignore", "bashrc", "zshrc", "npmrc", "nvmrc", "vscode", "github", "dockerignore",
    "editorconfig", "prettierrc", "eslintrc",
];

/// Names that read as code next to a symbol: "console dot log", "self dot name".
const CODE_WORDS: &[&str] = &[
    "self", "os", "sys", "std", "console", "req", "res", "ctx", "args", "kwargs", "src", "lib", "env",
    "json", "foo", "obj", "err", "utils",
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

/// A spoken-code rule starts at `w[j]`: a symbol word, or a two-word one like "open paren".
fn starts_code(w: &[Word], j: usize) -> bool {
    let key = |j: usize| w.get(j).map_or("", |x| x.key.as_str());
    is_symbol_word(key(j))
        || matches!(
            (key(j), key(j + 1)),
            ("all", "caps" | "cap")
                | ("double", "dash" | "equals")
                | ("open" | "close" | "left" | "right", "paren" | "parenthesis" | "parens" | "bracket" | "brace" | "curly")
                | ("at" | "dollar", "sign")
                | ("forward" | "back", "slash")
                | ("camel" | "snake" | "pascal" | "kebab" | "constant", "case")
        )
}

/// A word that already looks like code: a digit or code symbol in it, a dotted name, one
/// lower-case letter other than "a" and "i" (`x`, `n`), or a name like `self` or `console`.
fn looks_code(core: &str) -> bool {
    let mut chars = core.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return c.is_ascii_digit() || c.is_ascii_lowercase() && c != 'a' && c != 'i';
    }
    let b = core.as_bytes();
    CODE_WORDS.contains(&core.to_lowercase().as_str())
        || core.chars().any(|c| c.is_ascii_digit() || "_/\\=|~@$(){}[]<>".contains(c))
        || (1..b.len().saturating_sub(1)).any(|k| b[k] == b'.' && b[k - 1].is_ascii_alphanumeric() && b[k + 1].is_ascii_alphanumeric())
}

/// The line `w[i]` is on starts with a command and is still a command when it reaches `w[i]`:
/// no small words and no message flag (`-m`) on the way, so "git commit dash m fix the main
/// pipe leak" is a message, not a pipeline.
fn command_line(w: &[Word], i: usize) -> bool {
    let start = (0..i).rev().find(|&j| !w[j].post.is_empty()).map_or(0, |j| j + 1);
    if start >= i || !LINE_COMMANDS.contains(&w[start].key.as_str()) {
        return false;
    }
    let run = &w[start..i];
    !run.iter().any(|x| stop(&x.key) || x.key == "message")
        && !run.windows(2).any(|p| p[0].key == "dash" && p[1].key == "m")
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

struct Out {
    pieces: Vec<Piece>,
    /// The last symbol attaches to whatever comes next.
    glue: bool,
    /// Plain words typed since the last code (a symbol, a word glued to one, a rule). 0: the
    /// last piece is code. `usize::MAX` after a pause or a line break.
    since_code: usize,
    /// "all caps": the next word, and the words joined to it by `_`, go upper case.
    upper_next: bool,
    upper: Vec<usize>,
}

impl Out {
    fn new() -> Self {
        Out { pieces: Vec::new(), glue: false, since_code: usize::MAX, upper_next: false, upper: Vec::new() }
    }

    fn word(&mut self, text: String) {
        if self.upper_next {
            self.upper.push(self.pieces.len());
            self.upper_next = false;
        }
        self.pieces.push(Piece { text, space: !self.glue });
        self.since_code = if self.glue { 0 } else { self.since_code.saturating_add(1) };
        self.glue = false;
    }

    /// `left`: no space before it. `right`: the next piece attaches with no space.
    fn sym(&mut self, text: &str, left: bool, right: bool) {
        self.pieces.push(Piece { text: text.into(), space: !(left || self.glue) });
        self.glue = right;
        self.since_code = 0;
    }

    /// Code is right before this point: a symbol, or at most one plain word after one.
    fn near_code(&self) -> bool {
        self.glue || self.since_code <= 1
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
    let mut out = Out::new();
    let mut i = 0;
    while i < w.len() {
        let used = match rule(&w, i, flavor, &mut out) {
            0 => {
                out.word(w[i].whole());
                1
            }
            used => used,
        };
        i += used;
        // A pause ends the code: what follows needs its own signal.
        if !w[i - 1].post.is_empty() {
            out.since_code = usize::MAX;
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
    let determiner = matches!(prev_key, "a" | "an" | "the" | "your" | "my" | "our" | "their" | "his" | "her" | "this" | "that");
    // A loose command ("add", "more") counts at the start of a line or on a command line.
    let after_command = prev.is_some_and(|p| p.post.is_empty() && PATH_COMMANDS.contains(&p.key.as_str()))
        && (!LOOSE_COMMANDS.contains(&prev_key) || i == 1 || !w[i - 2].post.is_empty() || command_line(w, i));
    // `n` words make the symbol: code on its left, a rule starting within the next two words
    // (no pause between), or a command line.
    let code_signal = |n: usize, out: &Out| {
        let ahead = w[i + n - 1].post.is_empty()
            && (starts_code(w, i + n) || w.get(i + n).is_some_and(|x| x.post.is_empty()) && starts_code(w, i + n + 1));
        out.near_code() || ahead || command_line(w, i)
    };
    let prev_code = prev.is_some_and(|p| p.post.is_empty() && looks_code(&p.core));
    let next_code = w[i].post.is_empty() && w.get(i + 1).is_some_and(|x| looks_code(&x.core));

    // ---- case commands
    if k0 == "all" && matches!(k1, "caps" | "cap") && free_right(2) && w[i].post.is_empty() && code_signal(2, out) {
        out.upper_next = true;
        out.since_code = 0;
        return 2;
    }
    if k1 == "case" && matches!(k0, "camel" | "snake" | "pascal" | "kebab" | "constant") && w[i].post.is_empty() && !determiner {
        // "We use camel case naming": an ordinary word right before, not code.
        let after_word = prev.is_some_and(|p| p.post.is_empty()) && !out.near_code();
        return case_run(w, i, k0, after_word, out);
    }

    // ---- line breaks
    let line = match (k0, k1) {
        ("new", "line") => Some(("\n", 2)),
        ("new", "paragraph") => Some(("\n\n", 2)),
        ("newline", _) => Some(("\n", 1)),
        _ => None,
    };
    if let Some((brk, n)) = line {
        // In a terminal a break is Return, which runs whatever is typed so far.
        if flavor == Flavor::Shell {
            return 0;
        }
        // Only as its own command: said between pauses ("Thanks. New paragraph. See you"), or
        // right after code (`{` then "new line"). Never after an ordinary word.
        let after_pause = prev.is_none_or(|p| !p.post.is_empty());
        let before_pause = !w[i + n - 1].post.is_empty() || last(i + n - 1);
        let whole = w[i].pre.is_empty() && (n == 1 || w[i].post.is_empty());
        let prose_after = matches!(key(i + n), "of" | "for" | "to" | "is" | "was" | "and" | "that" | "in" | "on");
        let after_code = out.glue || out.since_code == 0;
        if whole && !prose_after && ((after_pause && before_pause) || after_code) {
            out.pieces.push(Piece { text: brk.into(), space: false });
            out.glue = false;
            out.since_code = usize::MAX;
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
        return joiner(w, i, 2, sym, out, after_command, true);
    }

    // ---- one-word symbols
    match k0 {
        "dash" => dash(w, i, out),
        "dot" => {
            if matches!(prev_key, "polka" | "connect") {
                return 0;
            }
            if DOTFILES.contains(&k1) && w[i].post.is_empty() && !out.glue && !after_command && !prev_code
                && w.get(i + 1).is_some_and(|x| x.pre.is_empty())
            {
                out.sym(".", false, true);
                return 1;
            }
            // "build dot sh", "console dot log", "version 2 dot 0"; not "a red dot appears".
            let names = prev_code || next_code || w[i].post.is_empty() && EXTENSIONS.contains(&k1);
            joiner(w, i, 1, ".", out, after_command, names || code_signal(1, out))
        }
        "slash" => joiner(w, i, 1, "/", out, after_command, true),
        "backslash" => joiner(w, i, 1, "\\", out, after_command, true),
        "underscore" => {
            // Both sides: a name on the left (or a symbol), a name on the right. A run of them
            // (`__init__`) supplies its own missing side.
            let attach = out.glue || free_left;
            let left = attach || w[i].post.is_empty() && k1 == "underscore";
            let right = free_right(1) || out.pieces.last().is_some_and(|p| p.text == "_");
            if left && right && (code_signal(1, out) || prev_code || next_code) {
                out.sym("_", attach, free_right(1));
                1
            } else {
                0
            }
        }
        "tilde" => {
            if determiner || !w[i].post.is_empty() && !last(i) || !code_signal(1, out) {
                return 0;
            }
            out.sym("~", out.glue, false);
            1
        }
        "equals" => {
            if (free_left || out.glue) && free_right(1) && (code_signal(1, out) || prev_code) {
                spaced(out, "=", flavor);
                1
            } else {
                0
            }
        }
        "pipe" => {
            let prose_prev = matches!(prev_key, "water" | "gas" | "drain" | "smoke" | "exhaust" | "copper");
            let prose_next = matches!(key(i + 1), "down" | "up" | "dream" | "dreams" | "organ" | "wrench" | "line" | "burst" | "bomb");
            let code = code_signal(1, out) || prev_code || PIPE_TARGETS.contains(&k1);
            if free_left && free_right(1) && !prose_prev && !prose_next && code {
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
/// `signal`: the code signal two plain words need before they are joined.
fn joiner(w: &[Word], i: usize, n: usize, sym: &str, out: &mut Out, after_command: bool, signal: bool) -> usize {
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
    if free_left && right && signal {
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

/// "camel case user name" → `userName`; snake, pascal, kebab and constant likewise. Takes two
/// to six words, stopping at a pause, a small word or a symbol. It joins only when the run ends
/// where a name would: at code ("user name equals"), or at a pause or the end when no ordinary
/// word came right before (`after_word`). "Use snake case variable names in Python" and "We use
/// camel case naming here" stay prose.
fn case_run(w: &[Word], i: usize, style: &str, after_word: bool, out: &mut Out) -> usize {
    let mut parts: Vec<&Word> = Vec::new();
    let mut j = i + 2;
    let mut paused = false;
    while j < w.len() && parts.len() < 6 {
        let x = &w[j];
        if stop(&x.key) || starts_code(w, j) || !x.pre.is_empty() || !x.core.chars().all(|c| c.is_alphanumeric()) {
            break;
        }
        parts.push(x);
        j += 1;
        if !x.post.is_empty() {
            paused = true;
            break;
        }
    }
    let at_code = !paused && starts_code(w, j);
    let at_end = paused || j == w.len();
    if parts.len() < 2 || !(at_code || at_end && !after_word) {
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
    out.since_code = 0;
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
        assert_eq!(code("x equals five"), "x = five");
        assert_eq!(code("max underscore retries equals 3"), "max_retries = 3");
        assert_eq!(code("console dot log"), "console.log");
        assert_eq!(code("Use dot env files for secrets."), "Use .env files for secrets.");
        assert_eq!(code("Add the dot gitignore entry"), "Add the .gitignore entry");
        assert_eq!(code("version 2 dot 0"), "version 2.0");
        assert_eq!(shell("npm test pipe tee build dot txt"), "npm test | tee build.txt");
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
    fn new_line_said_as_its_own_command_breaks_the_text_in_code() {
        assert_eq!(code("first item. New line. Second item"), "first item.\nSecond item");
        assert_eq!(code("Thanks. New paragraph. See you"), "Thanks.\n\nSee you");
        assert_eq!(code("new line"), "\n");
        assert_eq!(code("fn main open paren close paren open brace new line"), "fn main() {\n");
        // After an ordinary word it is two words, not a break.
        assert_eq!(code("first item new line second item"), "first item new line second item");
    }

    #[test]
    fn terminal_never_breaks_a_line_from_spoken_code() {
        // A break in a terminal is Return: it would run a half-dictated command.
        for s in ["new line", "first item. New line. Second item", "Thanks. New paragraph. See you", "ls newline"] {
            assert!(!shell(s).contains('\n'), "shell broke the line: {s}");
        }
        assert_eq!(shell("git commit dash m fix new line handling"), "git commit -m fix new line handling");
        assert_eq!(code("git commit dash m fix new line handling"), "git commit -m fix new line handling");
    }

    /// Prose people dictate into Cursor, VS Code and terminal coding agents: comments, commit
    /// messages and prompts that name symbols and cases without meaning them as code.
    #[test]
    fn prose_dictated_to_coding_tools_is_left_alone() {
        for s in [
            "Fix the dash in the header so it lines up.",
            "Add a comment that explains why we use a dot here.",
            "Refactor this function to use snake case naming throughout.",
            "Rename the variables to camel case before the review.",
            "The tests fail when the path has a slash at the end.",
            "Make sure the config file ends with a new line.",
            "Write the constant names in all caps.",
            "Don't change the equals check in the parser.",
            "Use a pipe to send the output to the logger.",
            "Explain what the colon does in this type signature.",
            "The tilde in the version range allows patch updates.",
            "Remove the underscore prefix from private fields.",
            "Why does the dot product return a negative number?",
            "Split the string on each new line and trim the parts.",
            "Our team writes Python in snake case and JavaScript in camel case.",
            "Add a dash between the date and the time.",
            "Results underscore how slow the old parser was.",
            "Two plus two equals four in this test.",
            "Check that the main pipe closes when the reader exits.",
            "Commit with a message that says fix the slash command.",
            "Print a new line after each test result.",
            "Put the API key in all caps in the env file.",
            "Draw a dot at every point where the lines cross.",
            "Explain the difference between camel case and snake case.",
            "Strip the leading tilde from home paths.",
            "Format the dates with a dash, not a slash.",
            "This pipe dream of a rewrite can wait.",
            "Should the header use all caps or title case?",
            "Hard work equals results, so ship it.",
            "Snake case variable names are preferred here.",
            "Use snake case variable names in Python.",
            "New line handling broke on Windows.",
            "Fix newline handling in the parser.",
            "I need more dot points in the summary.",
            "Each red dot marks a failing test.",
            "Use camel case names for the props.",
        ] {
            assert_eq!(shell(s), s, "shell changed prose: {s}");
            assert_eq!(code(s), s, "code changed prose: {s}");
        }
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
            "Results underscore the need.",
            "Hard work equals success.",
            "The main pipe leaks.",
            "A red dot appears.",
            "Use tilde key.",
            "Type all caps please.",
            "Fix the missing new line.",
            "Python prefers snake case variables.",
            "We use camel case naming here",
            "count equals five",
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
