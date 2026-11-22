//! Snippets: say a trigger phrase, and Silas types the text saved for it.
//!
//! A snippet fires when the whole dictation is its trigger ("my address"), or when "insert"
//! marks it inside other speech ("send it to insert my address"). A sentence that only
//! contains the trigger's words ("I changed my address last week") is typed as spoken.
//! Matching ignores case and punctuation. Snippets live in the dictionary JSON so sync
//! (docs/sync.md, keys `s:` + norm(trigger)) carries them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub trigger: String,
    pub text: String,
    pub added: String,
}

/// The form triggers are compared in: lower case, letters and digits only, one space between words.
/// "My Address." and "my address" match; so do "e-mail" and "e mail".
pub fn match_key(s: &str) -> String {
    let mapped: String = s
        .chars()
        .filter(|c| *c != '\'' && *c != '\u{2019}')
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A piece of the text to type: speech (shaped by the app's style) or a snippet (typed as saved).
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Speech(String),
    Snippet(String),
}

/// The word that marks a snippet inside other speech.
const MARK: &str = "insert";

/// Split `text` into speech and the snippets it calls for.
pub fn expand(text: &str, snippets: &[Snippet]) -> Vec<Seg> {
    let usable: Vec<(String, &Snippet)> = snippets
        .iter()
        .map(|s| (match_key(&s.trigger), s))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    if usable.is_empty() {
        return vec![Seg::Speech(text.to_string())];
    }
    let whole = match_key(text);
    // The whole dictation is a trigger, with or without "insert" before it.
    let bare = whole.strip_prefix("insert ").unwrap_or(&whole);
    if let Some((_, s)) = usable.iter().find(|(k, _)| *k == whole || *k == bare) {
        return vec![Seg::Snippet(s.text.clone())];
    }
    // Tokens with their byte spans, and each one's match form.
    let toks: Vec<(usize, usize, String)> = spans(text)
        .into_iter()
        .map(|(a, b)| (a, b, match_key(&text[a..b])))
        .filter(|(_, _, k)| !k.is_empty())
        .collect();
    let mut segs = Vec::new();
    let mut from = 0; // byte offset of text not yet placed
    let mut t = 0;
    while t < toks.len() {
        if toks[t].2 != MARK {
            t += 1;
            continue;
        }
        // The longest trigger that the words after "insert" spell out.
        let mut best: Option<(usize, &Snippet)> = None;
        for (key, s) in &usable {
            let mut joined = String::new();
            for (m, tok) in toks[t + 1..].iter().enumerate() {
                if !joined.is_empty() {
                    joined.push(' ');
                }
                joined.push_str(&tok.2);
                if joined == *key {
                    if best.is_none_or(|(n, _)| m + 1 > n) {
                        best = Some((m + 1, s));
                    }
                    break;
                }
                if joined.len() >= key.len() {
                    break;
                }
            }
        }
        let Some((n, s)) = best else {
            t += 1;
            continue;
        };
        let start = toks[t].0;
        let last = &toks[t + n];
        // Keep the punctuation after the trigger ("insert my address, then") unless the
        // snippet ends a sentence of its own.
        let word = &text[last.0..last.1];
        let tail_at = word
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_alphanumeric())
            .map_or(word.len(), |(i, c)| i + c.len_utf8());
        let tail = &word[tail_at..];
        let before = &text[from..start];
        if !before.is_empty() {
            segs.push(Seg::Speech(before.to_string()));
        }
        segs.push(Seg::Snippet(s.text.clone()));
        let ends_itself = s.text.trim_end().ends_with(['.', '!', '?']);
        if ends_itself && tail.starts_with(['.', '!', '?']) {
            from = last.0 + tail_at + tail.len();
        } else {
            from = last.0 + tail_at;
        }
        t += n + 1;
    }
    if from < text.len() {
        segs.push(Seg::Speech(text[from..].to_string()));
    }
    if segs.is_empty() {
        segs.push(Seg::Speech(String::new()));
    }
    segs
}

/// Byte spans of the whitespace-separated words.
fn spans(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(trigger: &str, text: &str) -> Snippet {
        Snippet { trigger: trigger.into(), text: text.into(), added: String::new() }
    }

    fn typed(text: &str, snippets: &[Snippet]) -> String {
        expand(text, snippets)
            .into_iter()
            .map(|s| match s {
                Seg::Speech(t) | Seg::Snippet(t) => t,
            })
            .collect()
    }

    fn mine() -> Vec<Snippet> {
        vec![
            snip("my address", "12 Benson Rd, Springfield"),
            snip("my email", "me@example.com"),
            snip("sign off", "Thanks,\nDonovan"),
            snip("my address at work", "1 Office Park."),
        ]
    }

    #[test]
    fn a_whole_dictation_that_is_a_trigger_types_the_snippet() {
        assert_eq!(typed("My address.", &mine()), "12 Benson Rd, Springfield");
        assert_eq!(typed("my address", &mine()), "12 Benson Rd, Springfield");
        assert_eq!(typed("Sign-off!", &mine()), "Thanks,\nDonovan");
        assert_eq!(typed("Insert my email.", &mine()), "me@example.com");
        assert_eq!(expand("My email", &mine()), vec![Seg::Snippet("me@example.com".into())]);
    }

    #[test]
    fn insert_marks_a_snippet_inside_other_speech() {
        assert_eq!(typed("Send it to insert my address, then call me.", &mine()), "Send it to 12 Benson Rd, Springfield, then call me.");
        assert_eq!(typed("Reach me at insert my email.", &mine()), "Reach me at me@example.com.");
        // The longest trigger wins, and a snippet that ends its own sentence takes no second period.
        assert_eq!(typed("Mail it to insert my address at work.", &mine()), "Mail it to 1 Office Park.");
        assert_eq!(
            expand("Hi insert my email bye", &mine()),
            vec![Seg::Speech("Hi ".into()), Seg::Snippet("me@example.com".into()), Seg::Speech(" bye".into())]
        );
    }

    #[test]
    fn trigger_words_inside_other_speech_are_not_replaced() {
        for s in [
            "I changed my address last week.",
            "What is my email again?",
            "Please sign off on the plan.",
            "Insert the card, then my address shows.",
            "insert",
            "",
        ] {
            assert_eq!(typed(s, &mine()), s, "{s}");
        }
    }

    #[test]
    fn no_snippets_leaves_the_text_alone() {
        assert_eq!(expand("insert my address", &[]), vec![Seg::Speech("insert my address".into())]);
        assert_eq!(match_key("  My  Address! "), "my address");
        assert_eq!(match_key("Don't"), "dont");
    }
}
