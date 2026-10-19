//! The personal dictionary: words Silas must spell right, corrections it applies after
//! transcription, and the learning rules that turn a user's edits into new corrections.
//!
//! Everything here is pure and deterministic so it can be tested without audio.

use std::collections::HashSet;
use std::sync::OnceLock;

use chrono::Utc;
use rphonetic::{DoubleMetaphone, Encoder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub text: String,
    pub source: String,
    pub added: String,
    #[serde(default)]
    pub uses: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Correction {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub count: u32,
    pub source: String,
    pub added: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub from: String,
    pub to: String,
    pub count: u32,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dictionary {
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub corrections: Vec<Correction>,
    #[serde(default)]
    pub pending: Vec<Pending>,
    #[serde(default = "default_fillers")]
    pub fillers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LearnResult {
    pub learned: Vec<Correction>,
    pub pending: Vec<Pending>,
    pub terms: Vec<String>,
}

impl LearnResult {
    pub fn is_empty(&self) -> bool {
        self.learned.is_empty() && self.pending.is_empty() && self.terms.is_empty()
    }
}

pub fn default_fillers() -> Vec<String> {
    [
        "um", "umm", "uh", "uhh", "uhm", "erm", "er", "ah", "hmm", "mm", "mhm",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for Dictionary {
    fn default() -> Self {
        Self {
            terms: vec![],
            corrections: vec![],
            pending: vec![],
            fillers: default_fillers(),
        }
    }
}

/// A fix seen this many times becomes automatic.
const PROMOTE_AFTER: u32 = 2;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn common_words() -> &'static HashSet<&'static str> {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        include_str!("../data/common-en.txt")
            .lines()
            .map(str::trim)
            .collect()
    })
}

pub fn is_common(word: &str) -> bool {
    common_words().contains(word.to_lowercase().as_str())
}

/// Top 5,000 words only. The full list holds many names (it comes from subtitles), so a
/// capitalized word outside this smaller set is treated as a name worth remembering.
pub fn is_everyday(word: &str) -> bool {
    static WORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
    WORDS
        .get_or_init(|| {
            include_str!("../data/common-en.txt")
                .lines()
                .take(5000)
                .map(str::trim)
                .collect()
        })
        .contains(word.to_lowercase().as_str())
}

// ---------------------------------------------------------------- tokens

/// One word with the punctuation that hugs it, e.g. `"(Parish,"` → pre `(`, core `Parish`, post `,`.
#[derive(Debug, Clone, PartialEq)]
struct Tok {
    pre: String,
    core: String,
    post: String,
}

impl Tok {
    fn key(&self) -> String {
        norm(&self.core)
    }
    fn render(&self) -> String {
        format!("{}{}{}", self.pre, self.core, self.post)
    }
}


fn split_tok(raw: &str) -> Tok {
    let chars: Vec<char> = raw.chars().collect();
    let mut start = 0;
    while start < chars.len() && !chars[start].is_alphanumeric() {
        start += 1;
    }
    let mut end = chars.len();
    while end > start && !chars[end - 1].is_alphanumeric() {
        end -= 1;
    }
    // keep inner punctuation (local.ai, E-R-O, don't) but not outer.
    let core: String = chars[start..end].iter().collect();
    Tok {
        pre: chars[..start].iter().collect(),
        core,
        post: chars[end..].iter().collect(),
    }
}

fn tokenize(text: &str) -> Vec<Tok> {
    text.split_whitespace().map(split_tok).collect()
}

fn render(toks: &[Tok]) -> String {
    toks.iter()
        .map(Tok::render)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase letters and digits only, with spoken digits folded ("one" → "1").
fn norm(word: &str) -> String {
    let lower: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    match lower.as_str() {
        "zero" => "0".into(),
        "one" => "1".into(),
        "two" => "2".into(),
        "three" => "3".into(),
        "four" => "4".into(),
        "five" => "5".into(),
        "six" => "6".into(),
        "seven" => "7".into(),
        "eight" => "8".into(),
        "nine" => "9".into(),
        _ => lower,
    }
}

fn norm_phrase(phrase: &str) -> Vec<String> {
    tokenize(phrase)
        .iter()
        .map(Tok::key)
        .filter(|k| !k.is_empty())
        .collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 1.0 = identical, 0.0 = nothing in common.
pub fn similarity(a: &str, b: &str) -> f32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let max = a.len().max(b.len());
    if max == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f32 / max as f32
}

/// Primary and alternate Double Metaphone codes ("omarchy" → AMRX / AMRK), full length.
fn phonetic(s: &str) -> [String; 2] {
    phonetic_max(s, None)
}

fn phonetic_max(s: &str, max: Option<usize>) -> [String; 2] {
    let letters: String = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return [String::new(), String::new()];
    }
    // Full-length codes: the default stops at 4, so every long name sharing a first
    // syllable sounded the same ("Proton Pass" and "Proton Drive" were both PRTN).
    let dm = DoubleMetaphone::new(max);
    [dm.encode(&letters), dm.encode_alternate(&letters)]
}

/// How alike two spellings sound and look, 0..1, for deciding whether a user's edit was a
/// real correction. Learning keeps the 4-character codes it was tuned with.
pub fn sound_alike(heard: &str, intended: &str) -> f32 {
    sound_alike_codes(heard, intended, Some(4))
}

/// The same score for snapping heard words to terms, with full-length codes so long names
/// sharing a first syllable don't sound identical.
fn sounds_like_term(heard: &str, term: &str) -> f32 {
    sound_alike_codes(heard, term, None)
}

fn sound_alike_codes(heard: &str, intended: &str, max: Option<usize>) -> f32 {
    let a: String = norm_phrase(heard).concat();
    let b: String = norm_phrase(intended).concat();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut score = similarity(&a, &b);
    let (pa, pb) = (phonetic_max(&a, max), phonetic_max(&b, max));
    for x in pa.iter().filter(|x| !x.is_empty()) {
        for y in pb.iter().filter(|y| !y.is_empty()) {
            if x == y {
                score = score.max(0.85);
            } else {
                score = score.max(similarity(x, y) * 0.8);
            }
        }
    }
    score
}

/// `target` is `first`, at most one more letter, then `second`.
fn halves_of(first: &str, second: &str, target: &str) -> bool {
    target.len() <= first.len() + second.len() + 1
        && target.starts_with(first)
        && target[first.len()..].ends_with(second)
}

/// Some full-length sound code of `a` equals one of `b`.
fn same_sound(a: &str, b: &str) -> bool {
    let (pa, pb) = (phonetic(a), phonetic(b));
    pa.iter().any(|x| !x.is_empty() && pb.contains(x))
}

/// The "s" or "'s" a heard word adds to a term ("GitHubs" → "GitHub" + "s"), so snapping
/// to the term does not drop it.
fn plural_suffix(heard: &str, term: &str) -> String {
    let t = norm(term);
    for suffix in ["'s", "\u{2019}s", "s"] {
        let Some(stem) = heard.strip_suffix(suffix) else {
            continue;
        };
        if term.ends_with(suffix) {
            return String::new();
        }
        let possessive = suffix != "s";
        if possessive || (!t.ends_with('s') && similarity(&norm(stem), &t) >= 0.82) {
            return suffix.into();
        }
    }
    String::new()
}

/// A name worth listening for: capitalized or with a digit, and not ordinary English.
fn is_name(w: &str) -> bool {
    (starts_upper(w) || w.chars().any(|c| c.is_ascii_digit())) && !is_everyday(w) && w.len() > 2
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(char::is_uppercase)
}

fn ends_sentence(t: &Tok) -> bool {
    t.post.contains(['.', '!', '?'])
}

// ---------------------------------------------------------------- applying

impl Dictionary {
    /// Turn the engine's text into what Silas types.
    pub fn apply(&mut self, text: &str, remove_fillers: bool) -> String {
        let mut toks = tokenize(text);
        if remove_fillers {
            toks = self.drop_fillers(toks);
            toks = drop_stutters(toks);
        }
        let mut locked = vec![false; toks.len()];
        self.apply_corrections(&mut toks, &mut locked);
        self.snap_terms(&mut toks, &mut locked);
        render(&toks)
    }

    fn drop_fillers(&self, toks: Vec<Tok>) -> Vec<Tok> {
        let fillers: HashSet<String> = self.fillers.iter().map(|f| norm(f)).collect();
        let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
        let mut capitalize_next = false;
        for (i, t) in toks.into_iter().enumerate() {
            if fillers.contains(&t.key()) && !t.key().is_empty() {
                // "Um, so" → "So"; "and, um, then" → "and then"; "done um." → "done."
                let sentence_start = i == 0 || out.last().is_some_and(ends_sentence);
                if sentence_start && starts_upper(&t.core) {
                    capitalize_next = true;
                }
                if t.post.contains(['.', '!', '?']) {
                    if let Some(prev) = out.last_mut() {
                        if !ends_sentence(prev) {
                            prev.post = prev.post.trim_end_matches(',').to_string()
                                + t.post.trim_start_matches(',').trim();
                        }
                    }
                } else if let Some(prev) = out.last_mut() {
                    // "lessons, um, remember" keeps its comma: the pause was a real break. After
                    // "and" or "I think" it was only the filler's pause.
                    const BRIDGE: &[&str] = &[
                        "and", "but", "or", "so", "then", "because", "think", "like", "well",
                        "if", "that", "the", "a", "to", "of", "i", "we", "you",
                    ];
                    if prev.post == "," && t.post == "," && BRIDGE.contains(&prev.key().as_str()) {
                        prev.post.clear();
                    }
                }
                continue;
            }
            let mut t = t;
            if capitalize_next {
                t.core = capitalize(&t.core);
                capitalize_next = false;
            }
            out.push(t);
        }
        out
    }

    fn apply_corrections(&mut self, toks: &mut Vec<Tok>, locked: &mut Vec<bool>) {
        let mut order: Vec<usize> = (0..self.corrections.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(norm_phrase(&self.corrections[i].from).len()));
        for ci in order {
            let from = norm_phrase(&self.corrections[ci].from);
            if from.is_empty() {
                continue;
            }
            let to = self.corrections[ci].to.clone();
            let mut i = 0;
            while i + from.len() <= toks.len() {
                let window_matches =
                    (0..from.len()).all(|k| !locked[i + k] && toks[i + k].key() == from[k]);
                if !window_matches {
                    i += 1;
                    continue;
                }
                let sentence_start = i == 0 || ends_sentence(&toks[i - 1]);
                let replaced = replace_span(toks, locked, i, from.len(), &to, sentence_start);
                self.corrections[ci].count += 1;
                i += replaced.max(1);
            }
        }
    }

    /// Every word Silas snaps heard text to: the terms, plus the names that corrections
    /// produce ("martry" → "Martyr" means "Martyr" is a word to listen for). The index is
    /// the term's, for counting uses; None for a name that only lives in a correction.
    fn snap_targets(&self) -> Vec<(String, Option<usize>)> {
        let mut out: Vec<(String, Option<usize>)> =
            self.terms.iter().enumerate().map(|(i, t)| (t.text.clone(), Some(i))).collect();
        for c in &self.corrections {
            for w in tokenize(&c.to).into_iter().map(|t| t.core) {
                if is_name(&w) && !out.iter().any(|(t, _)| t.eq_ignore_ascii_case(&w)) {
                    out.push((w, None));
                }
            }
        }
        out
    }

    fn snap_terms(&mut self, toks: &mut Vec<Tok>, locked: &mut Vec<bool>) {
        let mut targets = self.snap_targets();
        targets.sort_by_key(|(t, _)| std::cmp::Reverse(t.len()));
        // Protect every name already spelled right before any matching, so a near twin
        // ("Parish1" next to "Parish2") can never bend it, whatever order they run in. Only an
        // exact longer name may still claim it ("Baney Spaces" → "BaneySpaces").
        let mut protected = vec![false; toks.len()];
        // Matched by spelling, ignoring case and a plural or possessive ending ("parish2",
        // "Parish2's").
        for (term, _) in &targets {
            let keys = norm_phrase(term);
            let n = keys.len();
            if n == 0 {
                continue;
            }
            for i in 0..toks.len().saturating_sub(n - 1) {
                let fits = (0..n).all(|k| {
                    let key = toks[i + k].key();
                    key == keys[k]
                        || (k == n - 1 && key.strip_suffix('s').is_some_and(|s| s == keys[k]))
                });
                if fits {
                    protected[i..i + n].fill(true);
                }
            }
        }
        for (term, ti) in targets {
            let target: String = norm_phrase(&term).concat();
            // Names known only from a correction need some length to be safe to listen for:
            // "Isa" would turn "is a" into a name.
            if target.len() < 3 || (ti.is_none() && target.len() < 5) {
                continue;
            }
            let term_keys = norm_phrase(&term);
            let term_words = term_keys.len();
            let mut i = 0;
            while i < toks.len() {
                let mut hit: Option<usize> = None;
                let mut fuzzy = false;
                for w in (1..=(term_words + 1).min(3)).rev() {
                    if i + w > toks.len() || (i..i + w).any(|k| locked[k]) {
                        continue;
                    }
                    let guarded = (i..i + w).any(|k| protected[k]);
                    // Don't fuse across a sentence break or a comma.
                    if (i..i + w - 1).any(|k| !toks[k].post.is_empty()) {
                        continue;
                    }
                    let window: Vec<String> = (i..i + w).map(|k| toks[k].key()).collect();
                    let joined = window.concat();
                    if joined.is_empty() {
                        continue;
                    }
                    let exact = joined == target;
                    let all_common = (i..i + w).all(|k| is_common(&toks[k].core));
                    let already = (i..i + w)
                        .map(|k| toks[k].core.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                        == term;
                    if already {
                        hit = Some(0);
                        break;
                    }
                    // A protected word may only stay put inside a longer name, spelled as it sits
                    // there ("Claude" in "Claude Kode" → "Claude Code", "Baney" in "Baney space" →
                    // "BaneySpaces"), or take its own name's ending ("ghostty's" → "Ghostty's").
                    // It is never bent into a different spelling.
                    let protected_fits = {
                        let mut at = 0;
                        (i..i + w).all(|k| {
                            let key = toks[k].key();
                            let rest = target.get(at..).unwrap_or("");
                            at += key.len();
                            if !protected[k] {
                                return true;
                            }
                            let last = k == i + w - 1;
                            // The name goes on past this word ("Baney" + "space").
                            let inside = w >= 2 && rest.len() > key.len() && rest.starts_with(&key);
                            // The word ends a multi-word name.
                            let ends = w >= 2 && last && rest == key;
                            // Its own ending: "ghostty's" for "Ghostty".
                            let own_ending = last && key.strip_suffix('s') == Some(rest);
                            // A word that already completes the name never takes the next word
                            // with it ("Tailscale a" is not "Tailscale").
                            inside || ends || own_ending
                        })
                    };
                    if guarded && !exact && !protected_fits {
                        continue;
                    }
                    // A one-word term that is also an everyday word ("Actor", "Leading") only
                    // keeps its casing when spoken as such; never recase "the actor".
                    // A name known only from a correction ("Martyr") is a spelling to reach, not a
                    // casing to force: "a martyr died" stays as spoken.
                    let ordinary = if ti.is_some() { w == 1 && is_everyday(&toks[i].core) } else { all_common };
                    if exact && !ordinary {
                        hit = Some(w);
                        break;
                    }
                    if exact {
                        continue; // spoken as ordinary words: leave them
                    }
                    // Fuzzy only for words that are not ordinary English; that guard is what
                    // keeps "better" from becoming "Bezier" and "make sure" from "make Azure".
                    // Never bend an address or file name ("claude.ai", "notes/plan.md").
                    let addressy = (i..i + w).any(|k| toks[k].core.contains(['.', '@', '/', '\\']));
                    // Two ordinary words can still be the two halves of a one-word name, with at
                    // most one letter lost at the seam ("Bane spaces" → "BaneySpaces"): names of
                    // 8+ letters, never two everyday words ("what matters most" is not
                    // "Mattermost"), and never a word that only resembles a half ("the sun shone"
                    // is not "Sunshine").
                    let fused = all_common
                        && w == 2
                        && term_words == 1
                        && !addressy
                        && target.len() >= 8
                        && !(is_everyday(&toks[i].core) && is_everyday(&toks[i + 1].core))
                        && halves_of(&toks[i].key(), &toks[i + 1].key(), &target);
                    if fused {
                        hit = Some(w);
                        break;
                    }
                    // "martyr's" is still the word martyr, not the name Martyr.
                    let common_stem = w == 1
                        && ["'s", "\u{2019}s", "s"].iter().any(|e| {
                            toks[i].core.to_lowercase().strip_suffix(e).is_some_and(is_common)
                        });
                    if ti.is_none() && common_stem {
                        continue;
                    }
                    if all_common || addressy || target.len() < 5 || joined.len() < 4 {
                        continue;
                    }
                    // Score "Ghosty's" by its stem, so an ending doesn't block a match. Spelling
                    // only: sound codes drop the ending's vowel, so "Taurus" would sound like "Tauri".
                    // Only an ending on the term's own word counts: in "Hildegard is" the s
                    // belongs to "is".
                    let stem = joined
                        .strip_suffix('s')
                        .filter(|s| w == term_words && !target.ends_with('s') && !is_common(s));
                    // Numbers name different machines: "abby2" is abbey2, never abbey1.
                    let digits = |s: &str| s.chars().filter(char::is_ascii_digit).collect::<String>();
                    let (heard_digits, term_digits) = (digits(&joined), digits(&target));
                    if !heard_digits.is_empty() && !term_digits.is_empty() && heard_digits != term_digits {
                        continue;
                    }
                    let score = sounds_like_term(&joined, &target)
                        .max(stem.map_or(0.0, |s| similarity(s, &target)));
                    // A shared sound code alone is not enough: "Obsidian" and "Waypost" both
                    // encode as APST. Half the letters must agree too.
                    // Full-length codes that match exactly are trustworthy on their own ("Towery"
                    // and "Tauri" are both TR); they now tell Obsidian (APSTN) from Waypost (APST).
                    let spelled = similarity(stem.unwrap_or(&joined), &target);
                    let spelled_close = spelled >= 0.5 || (spelled >= 0.3 && same_sound(&joined, &target));
                    if spelled_close && score >= 0.82 && (w == term_words || score >= 0.88) {
                        hit = Some(w);
                        fuzzy = true;
                        break;
                    }
                }
                match hit {
                    Some(0) => {
                        if let Some(ti) = ti {
                            self.terms[ti].uses += 1;
                        }
                        // Already right: no shorter term may bend it later.
                        let n = term_words.max(1).min(toks.len() - i);
                        locked[i..i + n].fill(true);
                        i += n;
                    }
                    Some(w) => {
                        let sentence_start = i == 0 || ends_sentence(&toks[i - 1]);
                        let suffix = if fuzzy {
                            plural_suffix(&toks[i + w - 1].core, &term)
                        } else {
                            String::new()
                        };
                        let with = term.clone() + &suffix;
                        let n = replace_span(toks, locked, i, w, &with, sentence_start);
                        protected.splice(i..i + w, std::iter::repeat_n(false, n));
                        if let Some(ti) = ti {
                            self.terms[ti].uses += 1;
                        }
                        i += n.max(1);
                    }
                    None => i += 1,
                }
            }
        }
    }

    // ------------------------------------------------------------ editing

    pub fn add_term(&mut self, text: &str, source: &str) -> bool {
        let text = text.trim();
        if text.is_empty() || self.terms.iter().any(|t| t.text.eq_ignore_ascii_case(text)) {
            return false;
        }
        self.terms.push(Term {
            text: text.into(),
            source: source.into(),
            added: now(),
            uses: 0,
        });
        true
    }

    pub fn remove_term(&mut self, text: &str) {
        self.terms
            .retain(|t| !t.text.eq_ignore_ascii_case(text.trim()));
    }

    pub fn add_correction(&mut self, from: &str, to: &str, source: &str) -> Option<Correction> {
        let (from, to) = (from.trim(), to.trim());
        if from.is_empty() || norm_phrase(from) == norm_phrase(to) && from == to {
            return None;
        }
        self.pending.retain(|p| !p.from.eq_ignore_ascii_case(from));
        if let Some(c) = self
            .corrections
            .iter_mut()
            .find(|c| c.from.eq_ignore_ascii_case(from))
        {
            c.to = to.into();
            return Some(c.clone());
        }
        let c = Correction {
            from: from.into(),
            to: to.into(),
            count: 0,
            source: source.into(),
            added: now(),
        };
        self.corrections.push(c.clone());
        Some(c)
    }

    pub fn remove_correction(&mut self, from: &str) {
        self.corrections
            .retain(|c| !c.from.eq_ignore_ascii_case(from.trim()));
    }

    pub fn accept_pending(&mut self, from: &str) {
        if let Some(p) = self
            .pending
            .iter()
            .find(|p| p.from.eq_ignore_ascii_case(from))
            .cloned()
        {
            self.add_correction(&p.from, &p.to, "learned");
        }
    }

    pub fn dismiss_pending(&mut self, from: &str) {
        self.pending.retain(|p| !p.from.eq_ignore_ascii_case(from));
    }

    // ------------------------------------------------------------ learning

    /// Learn from the user turning `typed` into `fixed`. `explicit` means they fixed it on
    /// purpose in Silas's History, so every plausible fix is trusted at once.
    pub fn learn(&mut self, typed: &str, fixed: &str, explicit: bool) -> LearnResult {
        let mut result = LearnResult::default();
        for (from, to) in diff_fixes(typed, fixed) {
            if to.is_empty() {
                // A deleted word. Only ever a suggestion; never automatic.
                if norm_phrase(&from).len() == 1
                    && !self.fillers.iter().any(|f| norm(f) == norm(&from))
                {
                    self.observe_pending(&from, "", &mut result);
                }
                continue;
            }
            let score = sound_alike(&from, &to);
            let case_only = norm_phrase(&from) == norm_phrase(&to);
            let needed = if explicit { 0.3 } else { 0.45 };
            if !case_only && score < needed {
                continue; // a rewrite, not a mishearing
            }
            // A name (capitalized or with digits) that is not ordinary English is worth keeping as a word.
            let new_words: Vec<String> =
                tokenize(&to).into_iter().map(|t| t.core).filter(|w| is_name(w)).collect();
            let is_name = !new_words.is_empty();
            // Ordinary words ("office", "apple", "noon") are only ever rewritten into a name.
            // An edit that turns one ordinary word into another is a change of mind or a
            // typo, and learning it would break every later sentence that uses the word.
            let everyday_from = tokenize(&from).iter().all(|t| is_everyday(&t.core));
            if everyday_from && !is_name && !explicit {
                continue;
            }
            for w in &new_words {
                if self.add_term(w, "learned") {
                    result.terms.push(w.clone());
                }
            }
            let trusted = (explicit && (is_name || !everyday_from)) || (is_name && score >= 0.6);
            if trusted {
                if let Some(c) = self.add_correction(&from, &to, "learned") {
                    result.learned.push(c);
                }
            } else {
                self.observe_pending(&from, &to, &mut result);
            }
        }
        result.learned.dedup_by(|a, b| a.from == b.from);
        result
    }

    fn observe_pending(&mut self, from: &str, to: &str, result: &mut LearnResult) {
        let existing = self
            .pending
            .iter_mut()
            .find(|p| p.from.eq_ignore_ascii_case(from) && p.to == to);
        let count = match existing {
            Some(p) => {
                p.count += 1;
                p.last_seen = now();
                p.count
            }
            None => {
                self.pending.push(Pending {
                    from: from.into(),
                    to: to.into(),
                    count: 1,
                    last_seen: now(),
                });
                1
            }
        };
        if count >= PROMOTE_AFTER && !to.is_empty() {
            if let Some(c) = self.add_correction(from, to, "learned") {
                result.learned.push(c);
            }
        } else if let Some(p) = self
            .pending
            .iter()
            .find(|p| p.from.eq_ignore_ascii_case(from) && p.to == to)
        {
            result.pending.push(p.clone());
        }
    }

    /// Merge another dictionary: a Silas export or a TypeWhisper export
    /// (`[{type: "term"|"correction", original, replacement}]`).
    pub fn import(&mut self, json: &str) -> anyhow::Result<usize> {
        let before = self.terms.len() + self.corrections.len();
        if let Ok(other) = serde_json::from_str::<Dictionary>(json) {
            for t in other.terms {
                self.add_term(&t.text, "import");
            }
            for c in other.corrections {
                self.add_correction(&c.from, &c.to, "import");
            }
        } else {
            #[derive(Deserialize)]
            struct TwEntry {
                #[serde(rename = "type")]
                kind: String,
                original: String,
                replacement: Option<String>,
            }
            let entries: Vec<TwEntry> = serde_json::from_str(json)?;
            for e in entries {
                match (e.kind.as_str(), e.replacement.as_deref()) {
                    ("correction", Some(to)) if !to.is_empty() => {
                        self.add_correction(&e.original, to, "import");
                    }
                    _ => {
                        self.add_term(&e.original, "import");
                    }
                }
            }
        }
        Ok(self.terms.len() + self.corrections.len() - before)
    }
}

/// "the the the idea" → "the idea". Two in a row is kept unless it's a short function
/// word, since "had had" and "that that" are real English.
fn drop_stutters(mut toks: Vec<Tok>) -> Vec<Tok> {
    const STUTTER: &[&str] = &["the", "a", "an", "i", "to", "and", "we", "it", "is", "of", "in", "my", "you", "so", "but", "for"];
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    while i < toks.len() {
        let mut j = i + 1;
        while j < toks.len() && toks[j].key() == toks[i].key() && toks[j - 1].post.is_empty() && !toks[i].key().is_empty() {
            j += 1;
        }
        let run = j - i;
        let key = toks[i].key();
        // "b b b both" → "both": a run of one letter that starts the next word is a false start.
        let false_start = run >= 2
            && key.len() == 1
            && key != "a"
            && key != "i"
            && toks[j - 1].post.is_empty()
            && toks.get(j).is_some_and(|n| n.key().len() > 1 && n.key().starts_with(&key));
        if false_start {
            toks[j].pre = toks[i].pre.clone() + &toks[j].pre;
            if starts_upper(&toks[i].core) {
                toks[j].core = capitalize(&toks[j].core);
            }
            i = j; // the real word may start its own stutter: "t t the the idea"
            continue;
        }
        if run >= 3 || (run == 2 && STUTTER.contains(&toks[i].key().as_str())) {
            let mut kept = toks[i].clone();
            kept.post = toks[j - 1].post.clone();
            out.push(kept);
        } else {
            out.extend_from_slice(&toks[i..j]);
        }
        i = j;
    }
    out
}

/// Replace `toks[i..i+n]` with `with`, keeping the outer punctuation. Returns the new length.
fn replace_span(
    toks: &mut Vec<Tok>,
    locked: &mut Vec<bool>,
    i: usize,
    n: usize,
    with: &str,
    sentence_start: bool,
) -> usize {
    let pre = toks[i].pre.clone();
    let post = toks[i + n - 1].post.clone();
    let mut new: Vec<Tok> = tokenize(with);
    if new.is_empty() {
        toks.drain(i..i + n);
        locked.drain(i..i + n);
        return 0;
    }
    if sentence_start {
        new[0].core = capitalize(&new[0].core);
    }
    new[0].pre = pre + &new[0].pre;
    let last = new.len() - 1;
    new[last].post = new[last].post.clone() + &post;
    let len = new.len();
    toks.splice(i..i + n, new);
    locked.splice(i..i + n, std::iter::repeat_n(true, len));
    len
}

/// Word-level diff of what Silas typed against what the user left. Returns small
/// replacement hunks (1–3 words each side) as plain phrases.
pub fn diff_fixes(typed: &str, fixed: &str) -> Vec<(String, String)> {
    let a = tokenize(typed);
    let b = tokenize(fixed);
    let (n, m) = (a.len(), b.len());
    // dp[i][j] = edit distance between a[i..] and b[j..]
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..=n).rev() {
        for j in (0..=m).rev() {
            dp[i][j] = if i == n {
                m - j
            } else if j == m {
                n - i
            } else if a[i].core == b[j].core {
                dp[i + 1][j + 1]
            } else {
                1 + dp[i + 1][j + 1].min(dp[i + 1][j]).min(dp[i][j + 1])
            };
        }
    }
    let mut hunks = vec![];
    let (mut i, mut j) = (0, 0);
    let (mut ha, mut hb): (Vec<String>, Vec<String>) = (vec![], vec![]);
    let flush =
        |ha: &mut Vec<String>, hb: &mut Vec<String>, hunks: &mut Vec<(String, String)>| {
            if !ha.is_empty() && ha.len() <= 3 && hb.len() <= 3 {
                hunks.push((ha.join(" "), hb.join(" ")));
            }
            ha.clear();
            hb.clear();
        };
    while i < n || j < m {
        if i < n && j < m && a[i].core == b[j].core {
            flush(&mut ha, &mut hb, &mut hunks);
            i += 1;
            j += 1;
        } else if i < n && j < m && dp[i][j] == 1 + dp[i + 1][j + 1] {
            ha.push(a[i].core.clone());
            hb.push(b[j].core.clone());
            i += 1;
            j += 1;
        } else if i < n && (j == m || dp[i][j] == 1 + dp[i + 1][j]) {
            ha.push(a[i].core.clone());
            i += 1;
        } else {
            hb.push(b[j].core.clone());
            j += 1;
        }
    }
    flush(&mut ha, &mut hb, &mut hunks);
    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        let mut d = Dictionary::default();
        for t in [
            "Claude Code",
            "Claude",
            "Parish1",
            "Chapel1",
            "Ghostty",
            "TypeWhisper",
            "GitHub",
            "Tailscale",
            "Omarchy",
            "BaneyVault",
        ] {
            d.add_term(t, "user");
        }
        d.add_correction("Paris one", "Parish1", "user");
        d.add_correction("make Azure", "make sure", "user");
        d
    }

    #[test]
    fn fillers_go_and_sentence_case_stays() {
        let mut d = dict();
        assert_eq!(
            d.apply("Um, so we start. Uh it works, um, fine.", true),
            "So we start. It works, fine."
        );
        assert_eq!(
            d.apply("And, uh, then we ship. I think, um, we should.", true),
            "And then we ship. I think we should."
        );
        assert_eq!(d.apply("This is done um.", true), "This is done.");
        assert_eq!(d.apply("Um", true), "");
    }

    #[test]
    fn corrections_apply_with_punctuation() {
        let mut d = dict();
        assert_eq!(
            d.apply("ssh into Paris one, then check.", true),
            "ssh into Parish1, then check."
        );
        assert_eq!(d.apply("Paris one is up.", true), "Parish1 is up.");
        assert_eq!(
            d.apply("Please make Azure it runs.", true),
            "Please make sure it runs."
        );
    }

    #[test]
    fn terms_snap_exact_joins_and_case() {
        let mut d = dict();
        assert_eq!(
            d.apply("open ghostty and type whisper", true),
            "open Ghostty and TypeWhisper"
        );
        assert_eq!(d.apply("push it to github.", true), "push it to GitHub.");
        assert_eq!(
            d.apply("restart chapel one now", true),
            "restart Chapel1 now"
        );
        assert_eq!(
            d.apply("I use clot code daily", true),
            "I use clot code daily"
        ); // two common-ish words: no guess
    }

    #[test]
    fn terms_snap_fuzzy_rare_words() {
        let mut d = dict();
        assert_eq!(d.apply("open Ghosty please", true), "open Ghostty please");
        assert_eq!(
            d.apply("install Omarki on it", true),
            "install Omarchy on it"
        );
        assert_eq!(d.apply("check Tail Scale", true), "check Tailscale");
    }

    #[test]
    fn common_words_are_never_bent_into_jargon() {
        let mut d = dict();
        for t in ["Bezier", "Azure", "Codex", "Faber"] {
            d.add_term(t, "user");
        }
        let s = "make sure the model is faster and better, and the code is clean.";
        assert_eq!(d.apply(s, true), s);
        // Endings: a stem match must not bend real words ("Taurus" is not "Tauri" + s).
        for t in ["Tauri", "Ghostty", "Macy's", "Trader Joe's"] {
            d.add_term(t, "user");
        }
        let s = "the Taurus constellation, the ghosts and the hosts.";
        assert_eq!(d.apply(s, true), s);
        assert_eq!(d.apply("shop at macy's and trader joe's", true), "shop at Macy's and Trader Joe's");
        // Fusing two words into a name, and names known only from corrections.
        for t in ["BaneySpaces", "Mattermost", "Sunshine"] {
            d.add_term(t, "user");
        }
        d.add_correction("martry", "Martyr", "import");
        d.add_correction("eesa", "Isa", "import");
        d.add_correction("eena", "Ina", "import");
        d.add_correction("bookshelve", "Bookshelf", "import");
        for s in [
            "she found the bane of spaces everywhere.",
            "a martyr died for the faith.",
            "the fish has tail scales.",
            "what matters most is speed.",
            "this is a test, back in a minute.",
            "we visited the martyr's grave and the martyrs.",
            "put it on the book shelf.",
            "the sun shone all day.",
        ] {
            assert_eq!(d.apply(s, true), s);
        }
    }

    #[test]
    fn stutters_collapse_but_real_doubles_stay() {
        let mut d = dict();
        assert_eq!(d.apply("I don't like the the the idea", true), "I don't like the idea");
        assert_eq!(d.apply("we we had had enough", true), "we had had enough");
        assert_eq!(d.apply("no, no, no.", true), "no, no, no.");
        assert_eq!(
            d.apply("has both i but b b b b b b b b b both mobile", true),
            "has both i but both mobile"
        );
        assert_eq!(d.apply("B b both work.", true), "Both work.");
        assert_eq!(d.apply("plan a a apple", true), "plan a apple");
        assert_eq!(d.apply("x y z", true), "x y z");
        assert_eq!(d.apply("t t the the idea", true), "the idea");
        assert_eq!(d.apply("b b both both both", true), "both");
    }

    #[test]
    fn filler_between_commas_keeps_one() {
        let mut d = dict();
        assert_eq!(
            d.apply("Regarding the old lessons, um, remember that.", true),
            "Regarding the old lessons, remember that."
        );
    }

    #[test]
    fn addresses_and_plurals_survive_snapping() {
        let mut d = dict();
        assert_eq!(
            d.apply("please integrate better with claude.ai", true),
            "please integrate better with claude.ai"
        );
        assert_eq!(d.apply("open notes/omarki.md", true), "open notes/omarki.md");
        assert_eq!(
            d.apply("the file system, the GitHubs, and all the repos", true),
            "the file system, the GitHubs, and all the repos"
        );
        assert_eq!(d.apply("check the Ghostys now", true), "check the Ghosttys now");
        assert_eq!(d.apply("Ghosty's config", true), "Ghostty's config");
        assert_eq!(d.apply("open Ghosty please", true), "open Ghostty please");
    }

    #[test]
    fn everyday_words_keep_their_case() {
        let mut d = dict();
        d.add_term("Actor", "user");
        assert_eq!(d.apply("the actor left", true), "the actor left");
        assert_eq!(d.apply("ask claude", true), "ask Claude");
    }

    #[test]
    fn misses_from_real_history() {
        let mut d = dict();
        for t in ["BaneySpaces", "Runpod", "abbey1"] {
            d.add_term(t, "user");
        }
        d.add_correction("martry", "Martyr", "import");
        // A correction's target is a name Silas knows, so a new mishearing of it snaps too.
        assert_eq!(
            d.apply("Make sure the theme is that same Marter color scheme", true),
            "Make sure the theme is that same Martyr color scheme"
        );
        // Two ordinary words that spell a one-word name almost exactly fuse into it.
        assert_eq!(
            d.apply("For Bane spaces, work out why this blue dot's there.", true),
            "For BaneySpaces, work out why this blue dot's there."
        );
        assert_eq!(d.apply("link their run pod to it", true), "link their Runpod to it");
        assert_eq!(
            d.apply("Maybe not on Abbey One because it needs storage.", true),
            "Maybe not on abbey1 because it needs storage."
        );
    }

    #[test]
    fn near_twin_names_keep_their_own_spelling() {
        let mut d = dict();
        for t in ["Parish2", "abbey1", "abbey2"] {
            d.add_term(t, "user");
        }
        let s = "ssh into Parish2 now, then Parish1, abbey1 and abbey2.";
        assert_eq!(d.apply(s, true), s);
        assert_eq!(d.apply("ssh into parish2 now", true), "ssh into Parish2 now");
        assert_eq!(d.apply("Parish2's disk is full", true), "Parish2's disk is full");
        // A protected name can still be part of a longer name spoken exactly.
        d.add_term("BaneySpaces", "user");
        d.add_correction("Bainey", "Baney", "import");
        assert_eq!(d.apply("the dock of the Baney Spaces app", true), "the dock of the BaneySpaces app");
        assert_eq!(d.apply("open Baney space now", true), "open BaneySpaces now");
        // A name's own plural or possessive is still recased.
        assert_eq!(d.apply("check ghostty's config", true), "check Ghostty's config");
        assert_eq!(d.apply("push to github's api", true), "push to GitHub's api");
        // A protected name never swallows the next word, or grows into a longer twin.
        assert_eq!(d.apply("check Tailscale a lot", true), "check Tailscale a lot");
        assert_eq!(d.apply("restart Tailscale two times", true), "restart Tailscale two times");
        d.add_term("Parish10", "user");
        assert_eq!(d.apply("ssh into Parish1 now", true), "ssh into Parish1 now");
        // The digit picks the twin.
        assert_eq!(d.apply("reboot abby2", true), "reboot abbey2");
        assert_eq!(d.apply("we cloud code it later and make sure it works.", true), "we cloud code it later and make sure it works.");
    }

    #[test]
    fn sound_alike_names_still_snap() {
        let mut d = dict();
        d.add_term("Tauri", "user");
        // Spelled far apart, but the full sound codes match exactly (TR).
        assert_eq!(d.apply("build it with Towery", true), "build it with Tauri");
    }

    #[test]
    fn a_bigger_dictionary_does_not_bend_right_words() {
        let mut d = dict();
        for t in ["Obsidian", "Waypost", "Hildegard", "Proton Pass", "Proton Drive"] {
            d.add_term(t, "user");
        }
        for s in [
            // Same sound code (APST), almost no letters in common.
            "Obsidian needs more help, and Obsidian opens on my Mac.",
            // "is" is a word, not Hildegard's plural.
            "Hildegard is set up for generative requests.",
            "You should have gone to Proton Pass first, not Proton Drive.",
        ] {
            assert_eq!(d.apply(s, true), s);
        }
    }

    #[test]
    fn sound_codes_are_full_length() {
        // DoubleMetaphone::new(None) has no length cap; the default stops at 4.
        assert_eq!(phonetic("obsidian")[0], "APSTN");
        assert_ne!(phonetic("obsidian")[0], phonetic("waypost")[0]);
        assert_ne!(phonetic("protonpass")[0], phonetic("protondrive")[0]);
    }

    #[test]
    fn misheard_names_go_to_the_right_twin() {
        let mut d = dict();
        for t in ["Obsidian", "Waypost", "Hildegard", "Proton Pass", "Proton Drive"] {
            d.add_term(t, "user");
        }
        // Misspelled, so nothing is protected: the matcher alone has to get these right.
        assert_eq!(d.apply("open Obsidan now", true), "open Obsidian now");
        assert_eq!(d.apply("check Proton Pas first", true), "check Proton Pass first");
        assert_eq!(d.apply("Hildegarde is set up.", true), "Hildegard is set up.");
        // A longer name whose first word is itself a term still snaps.
        assert_eq!(d.apply("ask Claude Kode to do it", true), "ask Claude Code to do it");
    }

    #[test]
    fn diff_finds_small_fixes() {
        let f = diff_fixes(
            "ssh into Paris one and run it",
            "ssh into Parish1 and run it",
        );
        assert_eq!(f, vec![("Paris one".to_string(), "Parish1".to_string())]);
        let f = diff_fixes("I love clot code a lot", "I love Claude Code a lot");
        assert_eq!(
            f,
            vec![("clot code".to_string(), "Claude Code".to_string())]
        );
        assert!(diff_fixes("same text", "same text").is_empty());
    }

    #[test]
    fn learning_names_is_immediate_and_words_need_two() {
        let mut d = Dictionary::default();
        let r = d.learn("ask Magnet to run it", "ask Magnus to run it", false);
        // "Magnus" is not common English and is capitalized: trusted at once.
        assert_eq!(r.learned.len(), 1, "{r:?}");
        assert_eq!(d.apply("ask Magnet now", true), "ask Magnus now");

        let r = d.learn("the bezier works", "the better works", false);
        assert!(r.learned.is_empty() && r.pending.len() == 1, "{r:?}");
        let r = d.learn("so bezier now", "so better now", false);
        assert_eq!(r.learned.len(), 1, "{r:?}");
        assert_eq!(d.apply("it is bezier", true), "it is better");
    }

    #[test]
    fn everyday_words_are_never_learned_from_edits() {
        let mut d = Dictionary::default();
        d.learn("an apple a day", "an Apple a day", false);
        assert_eq!(d.apply("I ate an apple today", true), "I ate an apple today");
        for _ in 0..3 {
            d.learn("back at the office", "back at the officer", false);
            d.learn("lunch at noon", "lunch at noonn", false);
        }
        assert_eq!(d.apply("back at the office at noon", true), "back at the office at noon");
        // Taught on purpose in History, an ordinary word still needs a second sighting.
        let r = d.learn("call me at noon", "call me at nine", true);
        assert!(r.learned.is_empty(), "{r:?}");
        // A name is still learned at once.
        let r = d.learn("ssh into Paris one", "ssh into Parish1", false);
        assert_eq!(r.learned.len(), 1, "{r:?}");
    }

    #[test]
    fn rewrites_are_not_learned() {
        let mut d = Dictionary::default();
        let r = d.learn("send the report today", "send the invoice today", false);
        assert!(r.learned.is_empty() && r.pending.is_empty(), "{r:?}");
    }

    #[test]
    fn explicit_fixes_are_trusted() {
        let mut d = Dictionary::default();
        let r = d.learn("open the gh repo", "open the GitHub repo", true);
        assert!(r.learned.iter().any(|c| c.to == "GitHub"), "{r:?}");
    }

    #[test]
    fn imports_typewhisper_export() {
        let mut d = Dictionary::default();
        let json = r#"[{"type":"term","original":"Ghostty","replacement":""},{"type":"correction","original":"Paris one","replacement":"Parish1"}]"#;
        assert_eq!(d.import(json).unwrap(), 2);
        assert_eq!(d.apply("Paris one", true), "Parish1");
    }
}
