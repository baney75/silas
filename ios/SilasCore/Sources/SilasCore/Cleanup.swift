// Filler and stutter cleanup, ported from the desktop's `dictionary.rs`
// (`drop_fillers`, `drop_stutters` and the tokenizer they share). Pure and deterministic.

import Foundation

public enum Cleanup {
    public static let defaultFillers = [
        "um", "umm", "uh", "uhh", "uhm", "erm", "er", "ah", "hmm", "mm", "mhm",
    ]

    /// Turn the engine's text into what Silas writes: fillers and stutters removed,
    /// sentence case kept.
    public static func apply(_ text: String, fillers: [String] = defaultFillers) -> String {
        var toks = tokenize(text)
        toks = dropFillers(toks, fillers: Set(fillers.map(norm)))
        toks = dropStutters(toks)
        return render(toks)
    }
}

// MARK: - Tokens

/// One word with the punctuation that hugs it, e.g. `"(Parish,"` → pre `(`, core `Parish`, post `,`.
struct Tok: Equatable {
    var pre: String
    var core: String
    var post: String

    var key: String { norm(core) }
    var rendered: String { pre + core + post }
}

private func isAlnum(_ c: Character) -> Bool { c.isLetter || c.isNumber }

func splitTok(_ raw: Substring) -> Tok {
    let chars = Array(raw)
    var start = 0
    while start < chars.count && !isAlnum(chars[start]) { start += 1 }
    var end = chars.count
    while end > start && !isAlnum(chars[end - 1]) { end -= 1 }
    // Keep inner punctuation (local.ai, E-R-O, don't) but not outer.
    return Tok(
        pre: String(chars[..<start]),
        core: String(chars[start..<end]),
        post: String(chars[end...])
    )
}

func tokenize(_ text: String) -> [Tok] {
    text.split(whereSeparator: { $0.isWhitespace }).map(splitTok)
}

func render(_ toks: [Tok]) -> String {
    toks.map(\.rendered).filter { !$0.isEmpty }.joined(separator: " ")
}

/// Lowercase letters and digits only, with spoken digits folded ("one" → "1").
func norm(_ word: String) -> String {
    let lower = String(word.lowercased().filter(isAlnum))
    let digits = [
        "zero": "0", "one": "1", "two": "2", "three": "3", "four": "4",
        "five": "5", "six": "6", "seven": "7", "eight": "8", "nine": "9",
    ]
    return digits[lower] ?? lower
}

private func startsUpper(_ s: String) -> Bool { s.first?.isUppercase ?? false }

private func endsSentence(_ t: Tok) -> Bool { t.post.contains(where: { ".!?".contains($0) }) }

private func capitalize(_ s: String) -> String {
    guard let f = s.first else { return "" }
    return f.uppercased() + s.dropFirst()
}

// MARK: - Fillers

private let bridge: Set<String> = [
    "and", "but", "or", "so", "then", "because", "think", "like", "well",
    "if", "that", "the", "a", "to", "of", "i", "we", "you",
]

func dropFillers(_ toks: [Tok], fillers: Set<String>) -> [Tok] {
    var out: [Tok] = []
    out.reserveCapacity(toks.count)
    var capitalizeNext = false
    for (i, t) in toks.enumerated() {
        let key = t.key
        if !key.isEmpty && fillers.contains(key) && !looksLikeRealWord(t, after: out.last) {
            // "Um, so" → "So"; "and, um, then" → "and then"; "done um." → "done."
            let sentenceStart = i == 0 || (out.last.map(endsSentence) ?? false)
            if sentenceStart && startsUpper(t.core) {
                capitalizeNext = true
            }
            if endsSentence(t) {
                if var prev = out.last, !endsSentence(prev) {
                    prev.post = trimTrailingCommas(prev.post)
                        + trimLeadingCommas(t.post).trimmingCharacters(in: .whitespaces)
                    out[out.count - 1] = prev
                }
            } else if var prev = out.last {
                // "lessons, um, remember" keeps its comma: the pause was a real break. After
                // "and" or "I think" it was only the filler's pause.
                if prev.post == "," && t.post == "," && bridge.contains(prev.key) {
                    prev.post = ""
                    out[out.count - 1] = prev
                }
            }
            continue
        }
        var t = t
        if capitalizeNext {
            t.core = capitalize(t.core)
            capitalizeNext = false
        }
        out.append(t)
    }
    return out
}

/// "ER" (emergency room) and "5 mm" are words, not hesitations.
private func looksLikeRealWord(_ t: Tok, after prev: Tok?) -> Bool {
    if t.core.count > 1 && t.core.allSatisfy({ $0.isUppercase }) && t.core.contains(where: { $0.isLetter }) {
        // "UM" typed in caps is rare; "ER", "AH" in caps are acronyms. Keep them.
        return true
    }
    if let prev, prev.post.isEmpty || prev.post == ",", !prev.key.isEmpty, prev.key.allSatisfy({ $0.isNumber }) {
        return true
    }
    return false
}

private func trimTrailingCommas(_ s: String) -> String {
    var s = s
    while s.hasSuffix(",") { s.removeLast() }
    return s
}

private func trimLeadingCommas(_ s: String) -> String {
    String(s.drop(while: { $0 == "," }))
}

// MARK: - Stutters

private let stutterWords: Set<String> = [
    "the", "a", "an", "i", "to", "and", "we", "it", "is", "of", "in", "my", "you", "so", "but", "for",
]

/// "the the the idea" → "the idea". Two in a row is kept unless it's a short function
/// word, since "had had" and "that that" are real English.
func dropStutters(_ input: [Tok]) -> [Tok] {
    var toks = input
    var out: [Tok] = []
    out.reserveCapacity(toks.count)
    var i = 0
    while i < toks.count {
        let key = toks[i].key
        var j = i + 1
        while j < toks.count && toks[j].key == key && toks[j - 1].post.isEmpty && !key.isEmpty {
            j += 1
        }
        let run = j - i
        // "b b b both" → "both": a run of one letter that starts the next word is a false start.
        let falseStart = run >= 2
            && key.count == 1
            && key != "a"
            && key != "i"
            && toks[j - 1].post.isEmpty
            && j < toks.count
            && toks[j].key.count > 1
            && toks[j].key.hasPrefix(key)
        if falseStart {
            toks[j].pre = toks[i].pre + toks[j].pre
            if startsUpper(toks[i].core) {
                toks[j].core = capitalize(toks[j].core)
            }
            i = j  // the real word may start its own stutter: "t t the the idea"
            continue
        }
        if run >= 3 || (run == 2 && stutterWords.contains(key)) {
            var kept = toks[i]
            kept.post = toks[j - 1].post
            out.append(kept)
        } else {
            out.append(contentsOf: toks[i..<j])
        }
        i = j
    }
    return out
}
