// Double Metaphone, ported line for line from rphonetic 4.0.0 (`src/double_metaphone.rs`), which
// the desktop's `dictionary.rs` uses. rphonetic is itself a port of Apache Commons Codec, so this
// file carries the Apache License 2.0 (see the NOTICE below). Quirks are kept on purpose (for
// example the "VAN"/"VON" check with length 4 that never matches): the codes must equal the
// desktop's byte for byte, and `DoubleMetaphoneTests` checks them against codes made by rphonetic.
//
// Licensed to the Apache Software Foundation (ASF) under one or more contributor license
// agreements. The ASF licenses this file to You under the Apache License, Version 2.0; you may
// not use this file except in compliance with the License. You may obtain a copy of the License
// at http://www.apache.org/licenses/LICENSE-2.0. Unless required by applicable law or agreed to in
// writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.

import Foundation

public struct DoubleMetaphone {
    /// `nil` means no cap; the desktop's snapping uses full-length codes, learning uses 4.
    public let maxCodeLength: Int?

    public init(maxCodeLength: Int? = 4) {
        self.maxCodeLength = maxCodeLength
    }

    public func encode(_ value: String) -> String { run(value).primary }
    public func encodeAlternate(_ value: String) -> String { run(value).alternate }

    /// Both codes at once.
    public func codes(_ value: String) -> (primary: String, alternate: String) {
        let r = run(value)
        return (r.primary, r.alternate)
    }

    private func run(_ input: String) -> Result {
        var result = Result(max: maxCodeLength)
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty { return result }
        let v = Word(Array(trimmed.uppercased().utf8))
        let slavo = v.isSlavoGermanic
        var i = 0
        if v.startsWithAny(["GN", "KN", "PN", "WR", "PS"]) { i = 1 }
        while !result.isComplete && i < v.count {
            let ch = v.bytes[i]
            var skip = 0
            switch ch {
            case A, E, I, O, U, Y:
                if i == 0 { result.add("A") }
            case B:
                result.add("P")
                skip = v.char(i + 1) == B ? 1 : 0
            case C: skip = handleC(v, &result, i)
            case D: skip = handleD(v, &result, i)
            case F:
                result.add("F")
                skip = v.char(i + 1) == F ? 1 : 0
            case G: skip = handleG(v, &result, i, slavo)
            case H: skip = handleH(v, &result, i)
            case J: skip = handleJ(v, &result, i, slavo)
            case K:
                result.add("K")
                skip = v.char(i + 1) == K ? 1 : 0
            case L: skip = handleL(v, &result, i)
            case M:
                result.add("M")
                skip = conditionM0(v, i) ? 1 : 0
            case N:
                result.add("N")
                skip = v.char(i + 1) == N ? 1 : 0
            case P: skip = handleP(v, &result, i)
            case Q:
                result.add("K")
                skip = v.char(i + 1) == Q ? 1 : 0
            case R: skip = handleR(v, &result, i, slavo)
            case S: skip = handleS(v, &result, i, slavo)
            case T: skip = handleT(v, &result, i)
            case V:
                result.add("F")
                skip = v.char(i + 1) == V ? 1 : 0
            case W: skip = handleW(v, &result, i)
            case X: skip = handleX(v, &result, i)
            case Z: skip = handleZ(v, &result, i, slavo)
            default: break
            }
            i += skip + 1
        }
        return result
    }

    // MARK: - Letters

    private func handleC(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if conditionC0(v, i) {
            r.add("K")
            return 1
        } else if i == 0 && v.has(i, 6, "CAESAR") {
            r.add("S")
            return 1
        } else if v.has(i, 2, "CH") {
            return handleCH(v, &r, i)
        } else if v.has(i, 2, "CZ") && (i < 2 || !v.has(i - 2, 4, "WICZ")) {
            r.add("S", "X")
            return 1
        } else if v.has(i + 1, 3, "CIA") {
            r.add("X")
            return 2
        } else if v.has(i, 2, "CC") && !(i == 1 && v.char(0) == M) {
            return handleCC(v, &r, i)
        } else if v.has(i, 2, "CK", "CG", "CQ") {
            r.add("K")
            return 1
        } else if v.has(i, 2, "CI", "CE", "CY") {
            if v.has(i, 3, "CIO", "CIE", "CIA") {
                r.add("S", "X")
            } else {
                r.add("S")
            }
            return 1
        } else {
            r.add("K")
            if v.has(i + 1, 2, " C", " Q", " G") {
                return 2
            } else if v.has(i + 1, 1, "C", "K", "Q") && !v.has(i + 1, 2, "CE", "CI") {
                return 1
            }
            return 0
        }
    }

    private func conditionC0(_ v: Word, _ i: Int) -> Bool {
        if v.has(i, 4, "CHIA") { return true }
        if i < 1 { return false }
        if i < 2 || v.isVowel(i - 2) { return false }
        if i > 0 && !v.has(i - 1, 3, "ACH") { return false }
        let ch = v.char(i + 2)
        if i < 2 { return false }
        return (ch == nil || (ch != I && ch != E)) || v.has(i - 2, 6, "BACHER", "MACHER")
    }

    private func handleCH(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if i > 0 && v.has(i, 4, "CHAE") {
            r.add("K", "X")
        } else if conditionCH0(v, i) || conditionCH1(v, i) {
            r.add("K")
        } else if i > 0 {
            if v.has(0, 2, "MC") {
                r.add("K")
            } else {
                r.add("X", "K")
            }
        } else {
            r.add("X")
        }
        return 1
    }

    private func conditionCH0(_ v: Word, _ i: Int) -> Bool {
        if i != 0 { return false }
        if !v.has(i + 1, 5, "HARAC", "HARIS") && !v.has(i + 1, 3, "HOR", "HYM", "HIA", "HEM") {
            return false
        }
        return !v.has(0, 5, "CHORE")
    }

    private func conditionCH1(_ v: Word, _ i: Int) -> Bool {
        // `has(0, 4, "VAN", "VON")` can never match (length 4, 3-letter options); kept as rphonetic has it.
        (v.has(0, 4, "VAN", "VON") || v.has(0, 3, "SCH"))
            || (i > 1 && v.has(i - 2, 6, "ORCHES", "ARCHIT", "ORCHID"))
            || (i > 1 && v.has(i + 2, 1, "T", "S"))
            || ((i == 0 || v.has(i - 1, 1, "A", "O", "U", "E"))
                && (v.has(i + 2, 1, "L", "R", "N", "M", "B", "H", "F", "V", "W", " ")
                    || i + 1 == v.count - 1))
    }

    private func handleCC(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.has(i + 2, 1, "I", "E", "H") && !v.has(i + 2, 2, "HU") {
            if (i == 1 && v.char(i - 1) == A) || v.has(i - 1, 5, "UCCEE", "UCCES") {
                r.add("KS")
            } else {
                r.add("X")
            }
            return 2
        }
        r.add("K")
        return 1
    }

    private func handleD(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.has(i, 2, "DG") {
            if v.has(i + 2, 1, "I", "E", "Y") {
                r.add("J")
                return 2
            }
            r.add("TK")
            return 1
        } else if v.has(i, 2, "DT", "DD") {
            r.add("T")
            return 1
        }
        r.add("T")
        return 0
    }

    private func handleG(_ v: Word, _ r: inout Result, _ i: Int, _ slavo: Bool) -> Int {
        if v.char(i + 1) == H {
            return handleGH(v, &r, i)
        } else if v.char(i + 1) == N {
            if i == 1 && v.isVowel(0) && !slavo {
                r.add("KN", "N")
            } else if !v.has(i + 2, 2, "EY") && v.char(i + 1) != Y && !slavo {
                r.add("N", "KN")
            } else {
                r.add("KN")
            }
            return 1
        } else if v.has(i + 1, 2, "LI") && !slavo {
            r.add("KL", "L")
            return 1
        } else if (i == 0
            && (v.char(i + 1) == Y
                || v.has(i + 1, 2, "ES", "EP", "EB", "EL", "EY", "IB", "IL", "IN", "IE", "EI", "ER")))
            || ((v.has(i + 1, 2, "ER") || v.char(i + 1) == Y)
                && !v.has(0, 6, "DANGER", "RANGER", "MANGER")
                && (i == 0 || !v.has(i - 1, 1, "E", "I"))
                && (i == 0 || !v.has(i - 1, 3, "RGY", "OGY")))
        {
            r.add("K", "J")
            return 1
        } else if v.has(i + 1, 1, "E", "I", "Y") || (i > 0 && v.has(i - 1, 4, "AGGI", "OGGI")) {
            if v.has(0, 4, "VAN ", "VON ") || v.has(0, 3, "SCH") || v.has(i + 1, 2, "ET") {
                r.add("K")
            } else if v.has(i + 1, 3, "IER") {
                r.add("J")
            } else {
                r.add("J", "K")
            }
            return 1
        } else if v.char(i + 1) == G {
            r.add("K")
            return 1
        }
        r.add("K")
        return 0
    }

    private func handleGH(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if i > 0 && !v.isVowel(i - 1) {
            r.add("K")
            return 1
        } else if i == 0 {
            r.add(v.char(i + 2) == I ? "J" : "K")
            return 1
        } else if (i > 1 && v.has(i - 2, 1, "B", "H", "D"))
            || (i > 2 && v.has(i - 3, 1, "B", "H", "D"))
            || (i > 3 && v.has(i - 4, 1, "B", "H"))
        {
            return 1
        }
        if i > 2 && v.char(i - 1) == U && v.has(i - 3, 1, "C", "G", "L", "R", "T") {
            r.add("F")
        } else if i > 0 && v.char(i - 1) != I {
            r.add("K")
        }
        return 1
    }

    private func handleH(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if (i == 0 || v.isVowel(i - 1)) && v.isVowel(i + 1) {
            r.add("H")
            return 1
        }
        return 0
    }

    private func handleJ(_ v: Word, _ r: inout Result, _ i: Int, _ slavo: Bool) -> Int {
        if v.has(i, 4, "JOSE") || v.has(0, 4, "SAN ") {
            if (i == 0 && v.char(i + 4) == SPACE) || v.count == 4 || v.has(0, 4, "SAN ") {
                r.add("H")
            } else {
                r.add("J", "H")
            }
            return 0
        }
        if i == 0 && !v.has(i, 4, "JOSE") {
            r.add("J", "A")
        } else if i > 0 && v.isVowel(i - 1) && !slavo && (v.char(i + 1) == A || v.char(i + 1) == O) {
            r.add("J", "H")
        } else if i == v.count - 1 {
            r.add("J", " ")
        } else if !v.has(i + 1, 1, "L", "T", "K", "S", "N", "M", "B", "Z")
            && (i == 0 || !v.has(i - 1, 1, "S", "K", "L"))
        {
            r.add("J")
        }
        return v.char(i + 1) == J ? 1 : 0
    }

    private func handleL(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.char(i + 1) == L {
            if conditionL0(v, i) {
                r.addPrimary("L")
            } else {
                r.add("L")
            }
            return 1
        }
        r.add("L")
        return 0
    }

    private func conditionL0(_ v: Word, _ i: Int) -> Bool {
        let n = v.count
        if n >= 3 && i == n - 3 && i > 0 && v.has(i - 1, 4, "ILLO", "ILLA", "ALLE") {
            return true
        }
        return ((n > 1 && v.has(n - 2, 2, "AS", "OS")) || (n > 0 && v.has(n - 1, 1, "A", "O")))
            && n > 0 && v.has(i - 1, 4, "ALLE")
    }

    private func conditionM0(_ v: Word, _ i: Int) -> Bool {
        if v.char(i + 1) == M { return true }
        return i > 0 && v.has(i - 1, 3, "UMB") && (i + 1 == v.count - 1 || v.has(i + 2, 2, "ER"))
    }

    private func handleP(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.char(i + 1) == H {
            r.add("F")
            return 1
        }
        r.add("P")
        return v.has(i + 1, 1, "P", "B") ? 1 : 0
    }

    private func handleR(_ v: Word, _ r: inout Result, _ i: Int, _ slavo: Bool) -> Int {
        if i > 3 && i == v.count - 1 && !slavo && v.has(i - 2, 2, "IE") && !v.has(i - 4, 2, "ME", "MA") {
            r.addAlternate("R")
        } else {
            r.add("R")
        }
        return v.char(i + 1) == R ? 1 : 0
    }

    private func handleS(_ v: Word, _ r: inout Result, _ i: Int, _ slavo: Bool) -> Int {
        if i > 0 && v.has(i - 1, 3, "ISL", "YSL") {
            return 0
        } else if i == 0 && v.has(i, 5, "SUGAR") {
            r.add("X", "S")
            return 0
        } else if v.has(i, 2, "SH") {
            if v.has(i + 1, 4, "HEIM", "HOEK", "HOLM", "HOLZ") {
                r.add("S")
            } else {
                r.add("X")
            }
            return 1
        } else if v.has(i, 3, "SIO", "SIA") || v.has(i, 4, "SIAN") {
            if slavo {
                r.add("S")
            } else {
                r.add("S", "X")
            }
            return 2
        } else if (i == 0 && v.has(i + 1, 1, "M", "N", "L", "W")) || v.has(i + 1, 1, "Z") {
            r.add("S", "X")
            return v.has(i + 1, 1, "Z") ? 1 : 0
        } else if v.has(i, 2, "SC") {
            return handleSC(v, &r, i)
        }
        if i > 1 && i == v.count - 1 && v.has(i - 2, 2, "AI", "OI") {
            r.addAlternate("S")
        } else {
            r.add("S")
        }
        return v.has(i + 1, 1, "S", "Z") ? 1 : 0
    }

    private func handleSC(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.char(i + 2) == H {
            if v.has(i + 3, 2, "OO", "ER", "EN", "UY", "ED", "EM") {
                if v.has(i + 3, 2, "ER", "EN") {
                    r.add("X", "SK")
                } else {
                    r.add("SK")
                }
            } else if i == 0 && !v.isVowel(3) && v.char(3) != W {
                r.add("X", "S")
            } else {
                r.add("X")
            }
        } else if v.has(i + 2, 1, "I", "E", "Y") {
            r.add("S")
        } else {
            r.add("SK")
        }
        return 2
    }

    private func handleT(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.has(i, 4, "TION") || v.has(i, 3, "TIA", "TCH") {
            r.add("X")
            return 2
        } else if v.has(i, 2, "TH") || v.has(i, 3, "TTH") {
            if v.has(i + 2, 2, "OM", "AM") || v.has(0, 4, "VAN ", "VON ") || v.has(0, 3, "SCH") {
                r.add("T")
            } else {
                r.add("0", "T")
            }
            return 1
        }
        r.add("T")
        return v.has(i + 1, 1, "T", "D") ? 1 : 0
    }

    private func handleW(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if v.has(i, 2, "WR") {
            r.add("R")
            return 1
        } else if i == 0 && (v.isVowel(i + 1) || v.has(i, 2, "WH")) {
            if v.isVowel(i + 1) {
                r.add("A", "F")
            } else {
                r.add("A")
            }
            return 0
        } else if (i > 0 && i == v.count - 1 && v.isVowel(i - 1))
            || (i > 0 && v.has(i - 1, 5, "EWSKI", "EWSKY", "OWSKI", "OWSKY"))
            || v.has(0, 3, "SCH")
        {
            r.addAlternate("F")
            return 0
        } else if v.has(i, 4, "WICZ", "WITZ") {
            r.add("TS", "FX")
            return 3
        }
        return 0
    }

    private func handleX(_ v: Word, _ r: inout Result, _ i: Int) -> Int {
        if i == 0 {
            r.add("S")
            return 0
        }
        if !(i == v.count - 1
            && ((i > 2 && v.has(i - 3, 3, "IAU", "EAU")) || (i > 1 && v.has(i - 2, 2, "AU", "OU"))))
        {
            r.add("KS")
        }
        return v.has(i + 1, 1, "C", "X") ? 1 : 0
    }

    private func handleZ(_ v: Word, _ r: inout Result, _ i: Int, _ slavo: Bool) -> Int {
        if v.char(i + 1) == H {
            r.add("J")
            return 1
        }
        if v.has(i + 1, 2, "ZO", "ZI", "ZA") || (slavo && (i > 0 && v.char(i - 1) != T)) {
            r.add("S", "TS")
        } else {
            r.add("S")
        }
        return v.char(i + 1) == Z ? 1 : 0
    }

    // MARK: - Helpers

    private struct Word {
        let bytes: [UInt8]
        var count: Int { bytes.count }

        init(_ bytes: [UInt8]) { self.bytes = bytes }

        func char(_ i: Int) -> UInt8? { i >= 0 && i < bytes.count ? bytes[i] : nil }

        /// rphonetic's `contains`: the `length` bytes at `start` equal one of `options`.
        func has(_ start: Int, _ length: Int, _ options: String...) -> Bool {
            guard start >= 0, start + length <= bytes.count else { return false }
            let slice = bytes[start..<start + length]
            return options.contains { $0.utf8.elementsEqual(slice) }
        }

        func startsWithAny(_ options: [String]) -> Bool {
            options.contains { bytes.starts(with: $0.utf8) }
        }

        /// rphonetic's `is_vowel(.., true)`: a, e, i, o, u or y.
        func isVowel(_ i: Int) -> Bool {
            guard let c = char(i) else { return false }
            return c == A || c == E || c == I || c == O || c == U || c == Y
        }

        var isSlavoGermanic: Bool {
            bytes.contains(W) || bytes.contains(K) || contains("CZ") || contains("WITZ")
        }

        private func contains(_ s: String) -> Bool {
            let needle = Array(s.utf8)
            guard needle.count <= bytes.count else { return false }
            for start in 0...(bytes.count - needle.count) where bytes[start..<start + needle.count].elementsEqual(needle) {
                return true
            }
            return false
        }
    }

    private struct Result {
        var primary = ""
        var alternate = ""
        let max: Int?

        var isComplete: Bool {
            guard let max else { return false }
            return primary.utf8.count >= max && alternate.utf8.count >= max
        }

        mutating func add(_ main: String, _ alt: String? = nil) {
            addPrimary(main)
            addAlternate(alt ?? main)
        }

        mutating func addPrimary(_ s: String) { Self.append(s, to: &primary, max: max) }
        mutating func addAlternate(_ s: String) { Self.append(s, to: &alternate, max: max) }

        private static func append(_ s: String, to code: inout String, max: Int?) {
            guard let max else {
                code += s
                return
            }
            let room = max - code.utf8.count
            guard room > 0 else { return }
            code += s.utf8.count <= room ? s : String(s.prefix(room))
        }
    }
}

private let A = UInt8(ascii: "A"), B = UInt8(ascii: "B"), C = UInt8(ascii: "C"), D = UInt8(ascii: "D")
private let E = UInt8(ascii: "E"), F = UInt8(ascii: "F"), G = UInt8(ascii: "G"), H = UInt8(ascii: "H")
private let I = UInt8(ascii: "I"), J = UInt8(ascii: "J"), K = UInt8(ascii: "K"), L = UInt8(ascii: "L")
private let M = UInt8(ascii: "M"), N = UInt8(ascii: "N"), O = UInt8(ascii: "O"), P = UInt8(ascii: "P")
private let Q = UInt8(ascii: "Q"), R = UInt8(ascii: "R"), S = UInt8(ascii: "S"), T = UInt8(ascii: "T")
private let U = UInt8(ascii: "U"), V = UInt8(ascii: "V"), W = UInt8(ascii: "W"), X = UInt8(ascii: "X")
private let Y = UInt8(ascii: "Y"), Z = UInt8(ascii: "Z"), SPACE = UInt8(ascii: " ")
