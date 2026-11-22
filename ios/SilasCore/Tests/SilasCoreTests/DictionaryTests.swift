import XCTest
@testable import SilasCore

/// The desktop's own tests from `core/voice/src/dictionary.rs`, case for case, so the iOS app
/// spells, corrects and learns exactly like the desktop. Keep the order and names in step with
/// the Rust file when cases are added there.
final class DictionaryTests: XCTestCase {
    func dict() -> PersonalDictionary {
        var d = PersonalDictionary()
        for t in ["Claude Code", "Claude", "Argon1", "Krypton1", "Ghostty", "TypeWhisper", "GitHub", "Tailscale", "Omarchy", "BaneyVault"] {
            d.addTerm(t, source: "user")
        }
        d.addCorrection("Argon one", "Argon1", source: "user")
        d.addCorrection("make Azure", "make sure", source: "user")
        return d
    }

    func testCommonWordListLoaded() {
        XCTAssertEqual(CommonWords.count, 20000)
        XCTAssertTrue(CommonWords.isCommon("better"))
        XCTAssertTrue(CommonWords.isEveryday("Actor"))
    }

    /// The package's copy must stay identical to the desktop's list.
    func testCommonWordListMatchesDesktop() throws {
        let here = URL(fileURLWithPath: #filePath)
        let core = here.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let repo = core.deletingLastPathComponent().deletingLastPathComponent()
        let desktop = try Data(contentsOf: repo.appendingPathComponent("core/voice/data/common-en.txt"))
        let copy = try Data(contentsOf: core.appendingPathComponent("Sources/SilasCore/Resources/common-en.txt"))
        XCTAssertEqual(copy, desktop, "copy core/voice/data/common-en.txt into ios/SilasCore/Sources/SilasCore/Resources/")
    }

    func testFillersGoAndSentenceCaseStays() {
        var d = dict()
        XCTAssertEqual(d.apply("Um, so we start. Uh it works, um, fine."), "So we start. It works, fine.")
        XCTAssertEqual(d.apply("And, uh, then we ship. I think, um, we should."), "And then we ship. I think we should.")
        XCTAssertEqual(d.apply("This is done um."), "This is done.")
        XCTAssertEqual(d.apply("Um"), "")
    }

    func testCorrectionsApplyWithPunctuation() {
        var d = dict()
        XCTAssertEqual(d.apply("ssh into Argon one, then check."), "ssh into Argon1, then check.")
        XCTAssertEqual(d.apply("Argon one is up."), "Argon1 is up.")
        XCTAssertEqual(d.apply("Please make Azure it runs."), "Please make sure it runs.")
    }

    func testTermsSnapExactJoinsAndCase() {
        var d = dict()
        XCTAssertEqual(d.apply("open ghostty and type whisper"), "open Ghostty and TypeWhisper")
        XCTAssertEqual(d.apply("push it to github."), "push it to GitHub.")
        XCTAssertEqual(d.apply("restart krypton one now"), "restart Krypton1 now")
        XCTAssertEqual(d.apply("I use clot code daily"), "I use clot code daily")
    }

    func testTermsSnapFuzzyRareWords() {
        var d = dict()
        XCTAssertEqual(d.apply("open Ghosty please"), "open Ghostty please")
        XCTAssertEqual(d.apply("install Omarki on it"), "install Omarchy on it")
        XCTAssertEqual(d.apply("check Tail Scale"), "check Tailscale")
    }

    func testCommonWordsAreNeverBentIntoJargon() {
        var d = dict()
        for t in ["Bezier", "Azure", "Codex", "Faber"] { d.addTerm(t, source: "user") }
        var s = "make sure the model is faster and better, and the code is clean."
        XCTAssertEqual(d.apply(s), s)
        for t in ["Tauri", "Ghostty", "Macy's", "Trader Joe's"] { d.addTerm(t, source: "user") }
        s = "the Taurus constellation, the ghosts and the hosts."
        XCTAssertEqual(d.apply(s), s)
        XCTAssertEqual(d.apply("shop at macy's and trader joe's"), "shop at Macy's and Trader Joe's")
        for t in ["BaneySpaces", "Mattermost", "Sunshine"] { d.addTerm(t, source: "user") }
        d.addCorrection("martry", "Martyr", source: "import")
        d.addCorrection("eesa", "Isa", source: "import")
        d.addCorrection("eena", "Ina", source: "import")
        d.addCorrection("bookshelve", "Bookshelf", source: "import")
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
            XCTAssertEqual(d.apply(s), s)
        }
    }

    func testStuttersCollapseButRealDoublesStay() {
        var d = dict()
        XCTAssertEqual(d.apply("I don't like the the the idea"), "I don't like the idea")
        XCTAssertEqual(d.apply("we we had had enough"), "we had had enough")
        XCTAssertEqual(d.apply("no, no, no."), "no, no, no.")
        XCTAssertEqual(d.apply("has both i but b b b b b b b b b both mobile"), "has both i but both mobile")
        XCTAssertEqual(d.apply("B b both work."), "Both work.")
        XCTAssertEqual(d.apply("plan a a apple"), "plan a apple")
        XCTAssertEqual(d.apply("x y z"), "x y z")
        XCTAssertEqual(d.apply("t t the the idea"), "the idea")
        XCTAssertEqual(d.apply("b b both both both"), "both")
    }

    func testFillerBetweenCommasKeepsOne() {
        var d = dict()
        XCTAssertEqual(d.apply("Regarding the old lessons, um, remember that."), "Regarding the old lessons, remember that.")
    }

    func testAddressesAndPluralsSurviveSnapping() {
        var d = dict()
        XCTAssertEqual(d.apply("please integrate better with claude.ai"), "please integrate better with claude.ai")
        XCTAssertEqual(d.apply("open notes/omarki.md"), "open notes/omarki.md")
        XCTAssertEqual(
            d.apply("the file system, the GitHubs, and all the repos"),
            "the file system, the GitHubs, and all the repos")
        XCTAssertEqual(d.apply("check the Ghostys now"), "check the Ghosttys now")
        XCTAssertEqual(d.apply("Ghosty's config"), "Ghostty's config")
        XCTAssertEqual(d.apply("open Ghosty please"), "open Ghostty please")
    }

    func testEverydayWordsKeepTheirCase() {
        var d = dict()
        d.addTerm("Actor", source: "user")
        XCTAssertEqual(d.apply("the actor left"), "the actor left")
        XCTAssertEqual(d.apply("ask claude"), "ask Claude")
    }

    func testMissesFromRealHistory() {
        var d = dict()
        for t in ["BaneySpaces", "Runpod", "xenon1"] { d.addTerm(t, source: "user") }
        d.addCorrection("martry", "Martyr", source: "import")
        XCTAssertEqual(
            d.apply("Make sure the theme is that same Marter color scheme"),
            "Make sure the theme is that same Martyr color scheme")
        XCTAssertEqual(
            d.apply("For Bane spaces, work out why this blue dot's there."),
            "For BaneySpaces, work out why this blue dot's there.")
        XCTAssertEqual(d.apply("link their run pod to it"), "link their Runpod to it")
        XCTAssertEqual(
            d.apply("Maybe not on Xenon One because it needs storage."),
            "Maybe not on xenon1 because it needs storage.")
    }

    func testNearTwinNamesKeepTheirOwnSpelling() {
        var d = dict()
        for t in ["Argon2", "xenon1", "xenon2"] { d.addTerm(t, source: "user") }
        let s = "ssh into Argon2 now, then Argon1, xenon1 and xenon2."
        XCTAssertEqual(d.apply(s), s)
        XCTAssertEqual(d.apply("ssh into argon2 now"), "ssh into Argon2 now")
        XCTAssertEqual(d.apply("Argon2's disk is full"), "Argon2's disk is full")
        d.addTerm("BaneySpaces", source: "user")
        d.addCorrection("Bainey", "Baney", source: "import")
        XCTAssertEqual(d.apply("the dock of the Baney Spaces app"), "the dock of the BaneySpaces app")
        XCTAssertEqual(d.apply("open Baney space now"), "open BaneySpaces now")
        XCTAssertEqual(d.apply("check ghostty's config"), "check Ghostty's config")
        XCTAssertEqual(d.apply("push to github's api"), "push to GitHub's api")
        XCTAssertEqual(d.apply("check Tailscale a lot"), "check Tailscale a lot")
        XCTAssertEqual(d.apply("restart Tailscale two times"), "restart Tailscale two times")
        XCTAssertEqual(d.apply("check Tailscal a lot"), "check Tailscale a lot")
        XCTAssertEqual(d.apply("check Tail Scale"), "check Tailscale")
        d.addTerm("Argon10", source: "user")
        XCTAssertEqual(d.apply("ssh into Argon1 now"), "ssh into Argon1 now")
        XCTAssertEqual(d.apply("reboot xeno2"), "reboot xenon2")
        d.addTerm("Argon12", source: "user")
        XCTAssertEqual(d.apply("ssh into Argon two now"), "ssh into Argon2 now")
        d.addTerm("Code", source: "user")
        XCTAssertEqual(d.apply("ask Claud Code to do it"), "ask Claude Code to do it")
        XCTAssertEqual(
            d.apply("we cloud code it later and make sure it works."),
            "we cloud code it later and make sure it works.")
    }

    func testSoundAlikeNamesStillSnap() {
        var d = dict()
        d.addTerm("Tauri", source: "user")
        XCTAssertEqual(d.apply("build it with Towery"), "build it with Tauri")
    }

    func testABiggerDictionaryDoesNotBendRightWords() {
        var d = dict()
        for t in ["Obsidian", "Waypost", "Hildegard", "Proton Pass", "Proton Drive"] { d.addTerm(t, source: "user") }
        for s in [
            "Obsidian needs more help, and Obsidian opens on my Mac.",
            "Hildegard is set up for generative requests.",
            "You should have gone to Proton Pass first, not Proton Drive.",
        ] {
            XCTAssertEqual(d.apply(s), s)
        }
    }

    func testSoundCodesAreFullLength() {
        XCTAssertEqual(phonetic("obsidian")[0], "APSTN")
        XCTAssertNotEqual(phonetic("obsidian")[0], phonetic("waypost")[0])
        XCTAssertNotEqual(phonetic("protonpass")[0], phonetic("protondrive")[0])
    }

    func testMisheardNamesGoToTheRightTwin() {
        var d = dict()
        for t in ["Obsidian", "Waypost", "Hildegard", "Proton Pass", "Proton Drive"] { d.addTerm(t, source: "user") }
        XCTAssertEqual(d.apply("open Obsidan now"), "open Obsidian now")
        XCTAssertEqual(d.apply("check Proton Pas first"), "check Proton Pass first")
        XCTAssertEqual(d.apply("Hildegarde is set up."), "Hildegard is set up.")
        XCTAssertEqual(d.apply("ask Claude Kode to do it"), "ask Claude Code to do it")
    }

    func testDiffFindsSmallFixes() {
        var f = diffFixes("ssh into Argon one and run it", "ssh into Argon1 and run it")
        XCTAssertEqual(f.map { [$0.0, $0.1] }, [["Argon one", "Argon1"]])
        f = diffFixes("I love clot code a lot", "I love Claude Code a lot")
        XCTAssertEqual(f.map { [$0.0, $0.1] }, [["clot code", "Claude Code"]])
        XCTAssertTrue(diffFixes("same text", "same text").isEmpty)
    }

    func testLearningNamesIsImmediateAndWordsNeedTwo() {
        var d = PersonalDictionary()
        var r = d.learn(typed: "ask Magnet to run it", fixed: "ask Magnus to run it", explicit: false)
        XCTAssertEqual(r.learned.count, 1, "\(r)")
        XCTAssertEqual(d.apply("ask Magnet now"), "ask Magnus now")
        r = d.learn(typed: "the bezier works", fixed: "the better works", explicit: false)
        XCTAssertTrue(r.learned.isEmpty && r.pending.count == 1, "\(r)")
        r = d.learn(typed: "so bezier now", fixed: "so better now", explicit: false)
        XCTAssertEqual(r.learned.count, 1, "\(r)")
        XCTAssertEqual(d.apply("it is bezier"), "it is better")
    }

    func testEverydayWordsAreNeverLearnedFromEdits() {
        var d = PersonalDictionary()
        d.learn(typed: "an apple a day", fixed: "an Apple a day", explicit: false)
        XCTAssertEqual(d.apply("I ate an apple today"), "I ate an apple today")
        for _ in 0..<3 {
            d.learn(typed: "back at the office", fixed: "back at the officer", explicit: false)
            d.learn(typed: "lunch at noon", fixed: "lunch at noonn", explicit: false)
        }
        XCTAssertEqual(d.apply("back at the office at noon"), "back at the office at noon")
        var r = d.learn(typed: "call me at noon", fixed: "call me at nine", explicit: true)
        XCTAssertTrue(r.learned.isEmpty, "\(r)")
        r = d.learn(typed: "ssh into Argon one", fixed: "ssh into Argon1", explicit: false)
        XCTAssertEqual(r.learned.count, 1, "\(r)")
    }

    func testRewritesAreNotLearned() {
        var d = PersonalDictionary()
        let r = d.learn(typed: "send the report today", fixed: "send the invoice today", explicit: false)
        XCTAssertTrue(r.learned.isEmpty && r.pending.isEmpty, "\(r)")
    }

    func testExplicitFixesAreTrusted() {
        var d = PersonalDictionary()
        let r = d.learn(typed: "open the gh repo", fixed: "open the GitHub repo", explicit: true)
        XCTAssertTrue(r.learned.contains { $0.to == "GitHub" }, "\(r)")
    }

    func testImportsTypewhisperExport() throws {
        var d = PersonalDictionary()
        let json = #"[{"type":"term","original":"Ghostty","replacement":""},{"type":"correction","original":"Argon one","replacement":"Argon1"}]"#
        XCTAssertEqual(try d.importJSON(Data(json.utf8)), 2)
        XCTAssertEqual(d.apply("Argon one"), "Argon1")
    }

    // MARK: iOS additions

    func testImportsASilasExport() throws {
        var src = dict()
        src.addTerm("Parish1", source: "user")
        let data = try JSONEncoder().encode(src)
        var d = PersonalDictionary()
        XCTAssertEqual(try d.importJSON(data), src.terms.count + src.corrections.count)
    }

    func testReadsTheDesktopFileShape() throws {
        let json = #"{"terms":[{"text":"Parish1","source":"user","added":"2026-11-22T00:00:00Z","uses":3}],"corrections":[{"from":"parish one","to":"Parish1","count":1,"source":"user","added":"x"}],"pending":[{"from":"a","to":"b","count":1,"lastSeen":"y"}]}"#
        var d = try JSONDecoder().decode(PersonalDictionary.self, from: Data(json.utf8))
        XCTAssertEqual(d.terms.first?.uses, 3)
        XCTAssertEqual(d.fillers, Cleanup.defaultFillers)
        XCTAssertEqual(d.apply("ssh into parish one"), "ssh into Parish1")
    }

    func testUsesAndCountsAreCounted() {
        var d = dict()
        _ = d.apply("open ghostty and Argon one")
        XCTAssertEqual(d.terms.first { $0.text == "Ghostty" }?.uses, 1)
        XCTAssertEqual(d.corrections.first { $0.from == "Argon one" }?.count, 1)
    }

    /// The snippet and spoken-code hooks change nothing until the desktop defines them.
    func testHooksAreInertForNow() {
        var d = dict()
        d.snippets = [Snippet(trigger: "my address", text: "12 Benson Rd", added: "")]
        XCTAssertEqual(d.apply("send it to my address"), "send it to my address")
    }
}

final class DoubleMetaphoneTests: XCTestCase {
    /// 5,067 words, coded by rphonetic 4.0.0 (the desktop's crate): full length and capped at 4.
    func testMatchesRphonetic() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .appendingPathComponent("Fixtures/double-metaphone.json")
        let rows = try JSONDecoder().decode([[String]].self, from: Data(contentsOf: url))
        XCTAssertGreaterThan(rows.count, 5000)
        let full = DoubleMetaphone(maxCodeLength: nil), four = DoubleMetaphone(maxCodeLength: 4)
        var misses: [String] = []
        for r in rows {
            let got = [full.encode(r[0]), full.encodeAlternate(r[0]), four.encode(r[0]), four.encodeAlternate(r[0])]
            if got != Array(r[1...4]) { misses.append("\(r[0]): want \(Array(r[1...4])) got \(got)") }
        }
        XCTAssertTrue(misses.isEmpty, misses.prefix(20).joined(separator: "\n"))
    }

    func testDocumentedExample() {
        XCTAssertEqual(DoubleMetaphone().encode("jumped"), "JMPT")
        XCTAssertEqual(DoubleMetaphone().encodeAlternate("jumped"), "AMPT")
    }
}
