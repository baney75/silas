// The personal dictionary, ported from the desktop's `core/voice/src/dictionary.rs`: words Silas
// must spell right, corrections it applies after transcription, and the rules that turn a user's
// edits into new corrections. Pure and deterministic, so it is tested without audio
// (`DictionaryTests` holds the desktop's own cases).
//
// Rust measures `len()` in UTF-8 bytes and sorts stably; both are kept here, because they decide
// which name wins.

import Foundation

public struct Term: Codable, Equatable, Hashable {
    public var text: String
    public var source: String
    public var added: String
    public var uses: UInt32

    public init(text: String, source: String, added: String, uses: UInt32 = 0) {
        self.text = text
        self.source = source
        self.added = added
        self.uses = uses
    }

    enum CodingKeys: String, CodingKey { case text, source, added, uses }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decode(String.self, forKey: .text)
        source = try c.decodeIfPresent(String.self, forKey: .source) ?? "user"
        added = try c.decodeIfPresent(String.self, forKey: .added) ?? ""
        uses = try c.decodeIfPresent(UInt32.self, forKey: .uses) ?? 0
    }
}

public struct Correction: Codable, Equatable, Hashable {
    public var from: String
    public var to: String
    public var count: UInt32
    public var source: String
    public var added: String

    public init(from: String, to: String, count: UInt32 = 0, source: String, added: String) {
        self.from = from
        self.to = to
        self.count = count
        self.source = source
        self.added = added
    }

    enum CodingKeys: String, CodingKey { case from, to, count, source, added }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        from = try c.decode(String.self, forKey: .from)
        to = try c.decode(String.self, forKey: .to)
        count = try c.decodeIfPresent(UInt32.self, forKey: .count) ?? 0
        source = try c.decodeIfPresent(String.self, forKey: .source) ?? "user"
        added = try c.decodeIfPresent(String.self, forKey: .added) ?? ""
    }
}

public struct Pending: Codable, Equatable {
    public var from: String
    public var to: String
    public var count: UInt32
    public var lastSeen: String
}

/// A spoken trigger and the text it stands for. Stored and synced (docs/sync.md); expansion is
/// the desktop's to define first, so `PersonalDictionary.expandSnippets` is a hook that changes nothing yet.
public struct Snippet: Codable, Equatable, Hashable {
    public var trigger: String
    public var text: String
    public var added: String

    public init(trigger: String, text: String, added: String) {
        self.trigger = trigger
        self.text = text
        self.added = added
    }
}

public struct LearnResult: Equatable {
    public var learned: [Correction] = []
    public var pending: [Pending] = []
    public var terms: [String] = []

    public var isEmpty: Bool { learned.isEmpty && pending.isEmpty && terms.isEmpty }
}

public struct PersonalDictionary: Codable, Equatable {
    public var terms: [Term] = []
    public var corrections: [Correction] = []
    public var pending: [Pending] = []
    public var fillers: [String] = Cleanup.defaultFillers
    public var snippets: [Snippet] = []

    public init() {}

    enum CodingKeys: String, CodingKey { case terms, corrections, pending, fillers, snippets }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        terms = try c.decodeIfPresent([Term].self, forKey: .terms) ?? []
        corrections = try c.decodeIfPresent([Correction].self, forKey: .corrections) ?? []
        pending = try c.decodeIfPresent([Pending].self, forKey: .pending) ?? []
        fillers = try c.decodeIfPresent([String].self, forKey: .fillers) ?? Cleanup.defaultFillers
        snippets = try c.decodeIfPresent([Snippet].self, forKey: .snippets) ?? []
    }

    /// A fix seen this many times becomes automatic.
    static let promoteAfter: UInt32 = 2

    // MARK: Applying

    /// Turn the engine's text into what Silas writes.
    public mutating func apply(_ text: String, removeFillers: Bool = true) -> String {
        var toks = tokenize(text)
        if removeFillers {
            toks = dropFillers(toks, fillers: Cleanup.fillerSet(fillers, for: toks))
            toks = dropStutters(toks)
        }
        var locked = [Bool](repeating: false, count: toks.count)
        applyCorrections(&toks, &locked)
        snapTerms(&toks, &locked)
        toks = expandSnippets(toks, locked: &locked)
        toks = spokenCode(toks, locked: &locked)
        return render(toks)
    }

    /// Hook for snippets ("my address" → the address). The desktop defines the rule first; until
    /// it lands this returns the tokens unchanged, and `DictionaryTests` pins that.
    func expandSnippets(_ toks: [Tok], locked: inout [Bool]) -> [Tok] { toks }

    /// Hook for spoken code ("open paren", "snake case"), also defined on the desktop first.
    func spokenCode(_ toks: [Tok], locked: inout [Bool]) -> [Tok] { toks }

    private mutating func applyCorrections(_ toks: inout [Tok], _ locked: inout [Bool]) {
        let order = corrections.indices
            .map { ($0, normPhrase(corrections[$0].from).count) }
            .sorted { $0.1 != $1.1 ? $0.1 > $1.1 : $0.0 < $1.0 }
            .map(\.0)
        for ci in order {
            let from = normPhrase(corrections[ci].from)
            if from.isEmpty { continue }
            let to = corrections[ci].to
            var i = 0
            while i + from.count <= toks.count {
                let matches = (0..<from.count).allSatisfy { k in !locked[i + k] && toks[i + k].key == from[k] }
                if !matches {
                    i += 1
                    continue
                }
                let sentenceStart = i == 0 || endsSentence(toks[i - 1])
                let replaced = replaceSpan(&toks, &locked, i, from.count, to, sentenceStart)
                corrections[ci].count += 1
                i += max(replaced, 1)
            }
        }
    }

    /// Every word Silas snaps heard text to: the terms, plus the names that corrections produce.
    /// The index is the term's, for counting uses; nil for a name that only lives in a correction.
    func snapTargets() -> [(String, Int?)] {
        var out: [(String, Int?)] = terms.enumerated().map { ($0.element.text, $0.offset) }
        for c in corrections {
            for w in tokenize(c.to).map(\.core) where isName(w) {
                if !out.contains(where: { eqIgnoreASCIICase($0.0, w) }) {
                    out.append((w, nil))
                }
            }
        }
        return out
    }

    private mutating func snapTerms(_ toks: inout [Tok], _ locked: inout [Bool]) {
        let targets = snapTargets().enumerated()
            .sorted { a, b in
                let (la, lb) = (a.element.0.utf8.count, b.element.0.utf8.count)
                return la != lb ? la > lb : a.offset < b.offset
            }
            .map(\.element)
        // Protect every name already spelled right before any matching, so a near twin can never
        // bend it. Only an exact longer name may still claim it ("Baney Spaces" → "BaneySpaces").
        var protected = [Bool](repeating: false, count: toks.count)
        for (term, _) in targets {
            let keys = normPhrase(term)
            let n = keys.count
            if n == 0 { continue }
            let limit = max(toks.count - (n - 1), 0)
            for i in 0..<limit {
                let fits = (0..<n).allSatisfy { k in
                    let key = toks[i + k].key
                    return key == keys[k] || (k == n - 1 && stripSuffix(key, "s") == keys[k])
                }
                if fits {
                    for k in i..<i + n { protected[k] = true }
                }
            }
        }
        for (term, ti) in targets {
            let target = normPhrase(term).joined()
            let targetLen = target.utf8.count
            // Names known only from a correction need some length to be safe to listen for.
            if targetLen < 3 || (ti == nil && targetLen < 5) { continue }
            let termKeys = normPhrase(term)
            let termWords = termKeys.count
            var i = 0
            while i < toks.count {
                var hit: Int?
                var fuzzy = false
                for w in stride(from: min(termWords + 1, 3), through: 1, by: -1) {
                    if i + w > toks.count || (i..<i + w).contains(where: { locked[$0] }) { continue }
                    let guarded = (i..<i + w).contains { protected[$0] }
                    // Don't fuse across a sentence break or a comma.
                    if (i..<i + w - 1).contains(where: { !toks[$0].post.isEmpty }) { continue }
                    let window = (i..<i + w).map { toks[$0].key }
                    let joined = window.joined()
                    if joined.isEmpty { continue }
                    let exact = joined == target
                    let allCommon = (i..<i + w).allSatisfy { CommonWords.isCommon(toks[$0].core) }
                    let already = (i..<i + w).map { toks[$0].core }.joined(separator: " ") == term
                    if already {
                        hit = 0
                        break
                    }
                    let byWord = termWords > 1 && w == termWords
                    let protectedFits: Bool
                    if byWord {
                        protectedFits = (i..<i + w).allSatisfy { k in
                            let key = toks[k].key
                            let want = termKeys[k - i]
                            return !protected[k] || key == want || (k == i + w - 1 && stripSuffix(key, "s") == want)
                        }
                    } else {
                        let targetBytes = Array(target.utf8)
                        var at = 0
                        protectedFits = (i..<i + w).allSatisfy { k in
                            let key = toks[k].key
                            let keyBytes = Array(key.utf8)
                            let rest = byteTail(targetBytes, from: at)
                            at += keyBytes.count
                            if !protected[k] { return true }
                            let last = k == i + w - 1
                            let inside = w >= 2 && rest.count > keyBytes.count && rest.starts(with: keyBytes)
                            let ends = w >= 2 && last && rest == keyBytes
                            let ownEnding = last && stripSuffix(key, "s").map { Array($0.utf8) == rest } == true
                            return inside || ends || ownEnding
                        }
                    }
                    if guarded && !exact && !protectedFits { continue }
                    let ordinary = ti != nil ? (w == 1 && CommonWords.isEveryday(toks[i].core)) : allCommon
                    if exact && !ordinary {
                        hit = w
                        break
                    }
                    if exact { continue }
                    // Numbers name different machines: "xeno2" is xenon2, never xenon1.
                    let termDigits = asciiDigits(target)
                    if !termDigits.isEmpty && asciiDigits(joined) != termDigits { continue }
                    let addressy = (i..<i + w).contains { k in
                        toks[k].core.contains(where: { ".@/\\".contains($0) })
                    }
                    let fused = allCommon
                        && w == 2
                        && termWords == 1
                        && !addressy
                        && targetLen >= 8
                        && !(CommonWords.isEveryday(toks[i].core) && CommonWords.isEveryday(toks[i + 1].core))
                        && halvesOf(toks[i].key, toks[i + 1].key, target)
                    if fused {
                        hit = w
                        break
                    }
                    // "martyr's" is still the word martyr, not the name Martyr.
                    let commonStem = w == 1 && ["'s", "\u{2019}s", "s"].contains { e in
                        stripSuffix(toks[i].core.lowercased(), e).map(CommonWords.isCommon) == true
                    }
                    if ti == nil && commonStem { continue }
                    if allCommon || addressy || targetLen < 5 || joined.utf8.count < 4 { continue }
                    let stem = stripSuffix(joined, "s").flatMap { s in
                        (w == termWords && !target.hasSuffix("s") && !CommonWords.isCommon(s)) ? s : nil
                    }
                    let score = max(soundsLikeTerm(joined, target), stem.map { similarity($0, target) } ?? 0)
                    let spelled = similarity(stem ?? joined, target)
                    let spelledClose = spelled >= 0.5 || (spelled >= 0.3 && sameSound(joined, target))
                    let shorterHits = w > termWords && {
                        let shorter = (i..<i + w - 1).map { toks[$0].key }.joined()
                        return max(soundsLikeTerm(shorter, target), similarity(shorter, target)) >= 0.82
                    }()
                    if spelledClose && !shorterHits && score >= 0.82 && (w == termWords || score >= 0.88) {
                        hit = w
                        fuzzy = true
                        break
                    }
                }
                switch hit {
                case 0?:
                    if let ti { terms[ti].uses += 1 }
                    // Already right: no shorter term may bend it later.
                    let n = min(max(termWords, 1), toks.count - i)
                    for k in i..<i + n { locked[k] = true }
                    i += n
                case let w?:
                    let sentenceStart = i == 0 || endsSentence(toks[i - 1])
                    let suffix = fuzzy ? pluralSuffix(toks[i + w - 1].core, term) : ""
                    let n = replaceSpan(&toks, &locked, i, w, term + suffix, sentenceStart)
                    protected.replaceSubrange(i..<i + w, with: [Bool](repeating: false, count: n))
                    if let ti { terms[ti].uses += 1 }
                    i += max(n, 1)
                case nil:
                    i += 1
                }
            }
        }
    }

    // MARK: Editing

    @discardableResult
    public mutating func addTerm(_ text: String, source: String) -> Bool {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty || terms.contains(where: { eqIgnoreASCIICase($0.text, text) }) { return false }
        terms.append(Term(text: text, source: source, added: rfc3339Now()))
        return true
    }

    public mutating func removeTerm(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        terms.removeAll { eqIgnoreASCIICase($0.text, t) }
    }

    @discardableResult
    public mutating func addCorrection(_ from: String, _ to: String, source: String) -> Correction? {
        let from = from.trimmingCharacters(in: .whitespacesAndNewlines)
        let to = to.trimmingCharacters(in: .whitespacesAndNewlines)
        if from.isEmpty || (normPhrase(from) == normPhrase(to) && from == to) { return nil }
        pending.removeAll { eqIgnoreASCIICase($0.from, from) }
        if let i = corrections.firstIndex(where: { eqIgnoreASCIICase($0.from, from) }) {
            corrections[i].to = to
            return corrections[i]
        }
        let c = Correction(from: from, to: to, source: source, added: rfc3339Now())
        corrections.append(c)
        return c
    }

    public mutating func removeCorrection(_ from: String) {
        let f = from.trimmingCharacters(in: .whitespacesAndNewlines)
        corrections.removeAll { eqIgnoreASCIICase($0.from, f) }
    }

    public mutating func acceptPending(_ from: String) {
        if let p = pending.first(where: { eqIgnoreASCIICase($0.from, from) }) {
            addCorrection(p.from, p.to, source: "learned")
        }
    }

    public mutating func dismissPending(_ from: String) {
        pending.removeAll { eqIgnoreASCIICase($0.from, from) }
    }

    // MARK: Learning

    /// Learn from the user turning `typed` into `fixed`. `explicit` means they fixed it on purpose
    /// in History, so every plausible fix is trusted at once.
    @discardableResult
    public mutating func learn(typed: String, fixed: String, explicit: Bool) -> LearnResult {
        var result = LearnResult()
        for (from, to) in diffFixes(typed, fixed) {
            if to.isEmpty {
                // A deleted word. Only ever a suggestion; never automatic.
                if normPhrase(from).count == 1 && !fillers.contains(where: { norm($0) == norm(from) }) {
                    observePending(from, "", &result)
                }
                continue
            }
            let score = soundAlike(from, to)
            let caseOnly = normPhrase(from) == normPhrase(to)
            let needed: Float = explicit ? 0.3 : 0.45
            if !caseOnly && score < needed { continue }
            let newWords = tokenize(to).map(\.core).filter(isName)
            let named = !newWords.isEmpty
            let everydayFrom = tokenize(from).allSatisfy { CommonWords.isEveryday($0.core) }
            if everydayFrom && !named && !explicit { continue }
            for w in newWords where addTerm(w, source: "learned") {
                result.terms.append(w)
            }
            let trusted = (explicit && (named || !everydayFrom)) || (named && score >= 0.6)
            if trusted {
                if let c = addCorrection(from, to, source: "learned") { result.learned.append(c) }
            } else {
                observePending(from, to, &result)
            }
        }
        // Rust's dedup_by: drop consecutive repeats of the same `from`.
        var deduped: [Correction] = []
        for c in result.learned where deduped.last?.from != c.from { deduped.append(c) }
        result.learned = deduped
        return result
    }

    private mutating func observePending(_ from: String, _ to: String, _ result: inout LearnResult) {
        let count: UInt32
        if let i = pending.firstIndex(where: { eqIgnoreASCIICase($0.from, from) && $0.to == to }) {
            pending[i].count += 1
            pending[i].lastSeen = rfc3339Now()
            count = pending[i].count
        } else {
            pending.append(Pending(from: from, to: to, count: 1, lastSeen: rfc3339Now()))
            count = 1
        }
        if count >= Self.promoteAfter && !to.isEmpty {
            if let c = addCorrection(from, to, source: "learned") { result.learned.append(c) }
        } else if let p = pending.first(where: { eqIgnoreASCIICase($0.from, from) && $0.to == to }) {
            result.pending.append(p)
        }
    }

    /// Merge another dictionary: a Silas export or a TypeWhisper export
    /// (`[{type: "term"|"correction", original, replacement}]`). Returns how many items were added.
    public mutating func importJSON(_ data: Data) throws -> Int {
        let before = terms.count + corrections.count
        if let other = try? JSONDecoder().decode(PersonalDictionary.self, from: data),
            (try? JSONSerialization.jsonObject(with: data)) is [String: Any]
        {
            for t in other.terms { addTerm(t.text, source: "import") }
            for c in other.corrections { addCorrection(c.from, c.to, source: "import") }
        } else {
            struct TwEntry: Decodable {
                let type: String
                let original: String
                let replacement: String?
            }
            for e in try JSONDecoder().decode([TwEntry].self, from: data) {
                if e.type == "correction", let to = e.replacement, !to.isEmpty {
                    addCorrection(e.original, to, source: "import")
                } else {
                    addTerm(e.original, source: "import")
                }
            }
        }
        return terms.count + corrections.count - before
    }
}

// MARK: - Common words

/// `core/voice/data/common-en.txt`, copied into this package (a test checks the copy is identical).
public enum CommonWords {
    static let all: [String] = {
        guard let url = Bundle.module.url(forResource: "common-en", withExtension: "txt"),
            let text = try? String(contentsOf: url, encoding: .utf8)
        else { return [] }
        // Rust's `lines()`: no extra empty line after the final newline, "\r\n" counts as one break.
        var lines = text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        return lines.map { $0.trimmingCharacters(in: .whitespaces) }
    }()

    static let common: Set<String> = Set(all)
    /// Top 5,000 only. The full list holds many names (it comes from subtitles), so a capitalized
    /// word outside this smaller set is treated as a name worth remembering.
    static let everyday: Set<String> = Set(all.prefix(5000))

    public static func isCommon(_ word: String) -> Bool { common.contains(word.lowercased()) }
    public static func isEveryday(_ word: String) -> Bool { everyday.contains(word.lowercased()) }

    /// How many words loaded; 0 means the resource is missing and the guard is off.
    public static var count: Int { all.count }
}

// MARK: - Matching helpers

func normPhrase(_ phrase: String) -> [String] {
    tokenize(phrase).map(\.key).filter { !$0.isEmpty }
}

func levenshtein(_ a: [Unicode.Scalar], _ b: [Unicode.Scalar]) -> Int {
    var prev = Array(0...b.count)
    var cur = [Int](repeating: 0, count: b.count + 1)
    if a.isEmpty { return b.count }
    for i in 1...a.count {
        cur[0] = i
        if !b.isEmpty {
            for j in 1...b.count {
                let cost = a[i - 1] != b[j - 1] ? 1 : 0
                cur[j] = min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost)
            }
        }
        swap(&prev, &cur)
    }
    return prev[b.count]
}

/// 1.0 = identical, 0.0 = nothing in common.
public func similarity(_ a: String, _ b: String) -> Float {
    let a = Array(a.unicodeScalars), b = Array(b.unicodeScalars)
    let m = max(a.count, b.count)
    if m == 0 { return 1 }
    return 1 - Float(levenshtein(a, b)) / Float(m)
}

/// Primary and alternate Double Metaphone codes of the ASCII letters in `s`.
func phonetic(_ s: String, max: Int? = nil) -> [String] {
    let letters = String(s.unicodeScalars.filter { $0.isASCII && $0.properties.isAlphabetic }.map(Character.init))
    if letters.isEmpty { return ["", ""] }
    let c = DoubleMetaphone(maxCodeLength: max).codes(letters)
    return [c.primary, c.alternate]
}

/// How alike two spellings sound and look, 0...1, for deciding whether an edit was a real
/// correction. Learning keeps the 4-character codes it was tuned with.
public func soundAlike(_ heard: String, _ intended: String) -> Float {
    soundAlikeCodes(heard, intended, max: 4)
}

func soundsLikeTerm(_ heard: String, _ term: String) -> Float {
    soundAlikeCodes(heard, term, max: nil)
}

private func soundAlikeCodes(_ heard: String, _ intended: String, max: Int?) -> Float {
    let a = normPhrase(heard).joined(), b = normPhrase(intended).joined()
    if a.isEmpty || b.isEmpty { return 0 }
    var score = similarity(a, b)
    let pa = phonetic(a, max: max), pb = phonetic(b, max: max)
    for x in pa where !x.isEmpty {
        for y in pb where !y.isEmpty {
            score = x == y ? Swift.max(score, 0.85) : Swift.max(score, similarity(x, y) * 0.8)
        }
    }
    return score
}

/// `target` is `first`, at most one more letter, then `second`.
func halvesOf(_ first: String, _ second: String, _ target: String) -> Bool {
    let f = Array(first.utf8), s = Array(second.utf8), t = Array(target.utf8)
    return t.count <= f.count + s.count + 1 && t.starts(with: f) && t[f.count...].reversed().starts(with: s.reversed())
}

/// Some full-length sound code of `a` equals one of `b`.
func sameSound(_ a: String, _ b: String) -> Bool {
    let pa = phonetic(a), pb = phonetic(b)
    return pa.contains { !$0.isEmpty && pb.contains($0) }
}

/// The "s" or "'s" a heard word adds to a term ("GitHubs" → "GitHub" + "s").
func pluralSuffix(_ heard: String, _ term: String) -> String {
    let t = norm(term)
    for suffix in ["'s", "\u{2019}s", "s"] {
        guard let stem = stripSuffix(heard, suffix) else { continue }
        if term.hasSuffix(suffix) { return "" }
        let possessive = suffix != "s"
        if possessive || (!t.hasSuffix("s") && similarity(norm(stem), t) >= 0.82) {
            return suffix
        }
    }
    return ""
}

/// A name worth listening for: capitalized or with a digit, and not ordinary English.
func isName(_ w: String) -> Bool {
    (w.first?.isUppercase == true || w.unicodeScalars.contains { $0.isASCII && $0.properties.numericType != nil })
        && !CommonWords.isEveryday(w) && w.utf8.count > 2
}


private func capitalized(_ s: String) -> String {
    guard let f = s.first else { return "" }
    return f.uppercased() + s.dropFirst()
}

/// Rust's `strip_suffix`, byte-exact.
func stripSuffix(_ s: String, _ suffix: String) -> String? {
    let b = Array(s.utf8), x = Array(suffix.utf8)
    guard b.count >= x.count, b.suffix(x.count).elementsEqual(x) else { return nil }
    return String(decoding: b.dropLast(x.count), as: UTF8.self)
}

/// Rust's `target.get(at..).unwrap_or("")` as bytes.
private func byteTail(_ bytes: [UInt8], from at: Int) -> [UInt8] {
    guard at <= bytes.count else { return [] }
    if at < bytes.count && bytes[at] & 0xC0 == 0x80 { return [] }
    return Array(bytes[at...])
}

private func asciiDigits(_ s: String) -> String {
    String(s.filter { $0.isASCII && $0.isNumber })
}

func eqIgnoreASCIICase(_ a: String, _ b: String) -> Bool {
    let x = Array(a.utf8), y = Array(b.utf8)
    guard x.count == y.count else { return false }
    for (p, q) in zip(x, y) where asciiLower(p) != asciiLower(q) { return false }
    return true
}

private func asciiLower(_ c: UInt8) -> UInt8 { (65...90).contains(c) ? c + 32 : c }

/// Replace `toks[i..<i+n]` with `with`, keeping the outer punctuation. Returns the new length.
func replaceSpan(_ toks: inout [Tok], _ locked: inout [Bool], _ i: Int, _ n: Int, _ with: String, _ sentenceStart: Bool) -> Int {
    let pre = toks[i].pre
    let post = toks[i + n - 1].post
    var new = tokenize(with)
    if new.isEmpty {
        toks.removeSubrange(i..<i + n)
        locked.removeSubrange(i..<i + n)
        return 0
    }
    if sentenceStart { new[0].core = capitalized(new[0].core) }
    new[0].pre = pre + new[0].pre
    new[new.count - 1].post = new[new.count - 1].post + post
    toks.replaceSubrange(i..<i + n, with: new)
    locked.replaceSubrange(i..<i + n, with: [Bool](repeating: true, count: new.count))
    return new.count
}

/// Word-level diff of what Silas wrote against what the user left: small replacement hunks
/// (1 to 3 words each side) as plain phrases.
public func diffFixes(_ typed: String, _ fixed: String) -> [(String, String)] {
    let a = tokenize(typed), b = tokenize(fixed)
    let n = a.count, m = b.count
    var dp = [[Int]](repeating: [Int](repeating: 0, count: m + 1), count: n + 1)
    for i in stride(from: n, through: 0, by: -1) {
        for j in stride(from: m, through: 0, by: -1) {
            if i == n {
                dp[i][j] = m - j
            } else if j == m {
                dp[i][j] = n - i
            } else if a[i].core == b[j].core {
                dp[i][j] = dp[i + 1][j + 1]
            } else {
                dp[i][j] = 1 + min(dp[i + 1][j + 1], dp[i + 1][j], dp[i][j + 1])
            }
        }
    }
    var hunks: [(String, String)] = []
    var i = 0, j = 0
    var ha: [String] = [], hb: [String] = []
    func flush() {
        if !ha.isEmpty && ha.count <= 3 && hb.count <= 3 {
            hunks.append((ha.joined(separator: " "), hb.joined(separator: " ")))
        }
        ha = []
        hb = []
    }
    while i < n || j < m {
        if i < n && j < m && a[i].core == b[j].core {
            flush()
            i += 1
            j += 1
        } else if i < n && j < m && dp[i][j] == 1 + dp[i + 1][j + 1] {
            ha.append(a[i].core)
            hb.append(b[j].core)
            i += 1
            j += 1
        } else if i < n && (j == m || dp[i][j] == 1 + dp[i + 1][j]) {
            ha.append(a[i].core)
            i += 1
        } else {
            hb.append(b[j].core)
            j += 1
        }
    }
    flush()
    return hunks
}

func rfc3339Now() -> String {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return f.string(from: Date())
}
