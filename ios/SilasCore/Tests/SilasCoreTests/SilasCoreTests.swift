import XCTest
@testable import SilasCore

/// Cases copied from the desktop's `dictionary.rs` tests so both apps clean text the same way.
final class CleanupTests: XCTestCase {
    func testFillersGoAndSentenceCaseStays() {
        XCTAssertEqual(Cleanup.apply("Um, so we start. Uh it works, um, fine."), "So we start. It works, fine.")
        XCTAssertEqual(
            Cleanup.apply("And, uh, then we ship. I think, um, we should."),
            "And then we ship. I think we should."
        )
        XCTAssertEqual(Cleanup.apply("This is done um."), "This is done.")
        XCTAssertEqual(Cleanup.apply("Um"), "")
    }

    func testStuttersCollapseButRealDoublesStay() {
        XCTAssertEqual(Cleanup.apply("I don't like the the the idea"), "I don't like the idea")
        XCTAssertEqual(Cleanup.apply("we we had had enough"), "we had had enough")
        XCTAssertEqual(Cleanup.apply("no, no, no."), "no, no, no.")
        XCTAssertEqual(
            Cleanup.apply("has both i but b b b b b b b b b both mobile"),
            "has both i but both mobile"
        )
        XCTAssertEqual(Cleanup.apply("B b both work."), "Both work.")
        XCTAssertEqual(Cleanup.apply("plan a a apple"), "plan a apple")
        XCTAssertEqual(Cleanup.apply("x y z"), "x y z")
        XCTAssertEqual(Cleanup.apply("t t the the idea"), "the idea")
        XCTAssertEqual(Cleanup.apply("b b both both both"), "both")
    }

    func testFillerBetweenCommasKeepsOne() {
        XCTAssertEqual(
            Cleanup.apply("Regarding the old lessons, um, remember that."),
            "Regarding the old lessons, remember that."
        )
    }

    func testPlainTextIsUntouched() {
        let s = "make sure the model is faster and better, and the code is clean."
        XCTAssertEqual(Cleanup.apply(s), s)
        XCTAssertEqual(Cleanup.apply(""), "")
    }
}

final class HoldMachineTests: XCTestCase {
    let t0 = Date(timeIntervalSinceReferenceDate: 0)

    func testHoldSpeaksAndReleaseFinishes() {
        var m = HoldMachine()
        XCTAssertEqual(m.press(at: t0), .start)
        XCTAssertTrue(m.isActive)
        XCTAssertEqual(m.release(at: t0 + 1.0), .finish)
        XCTAssertFalse(m.isActive)
    }

    func testTapTogglesHandsFreeUntilNextPress() {
        var m = HoldMachine()
        XCTAssertEqual(m.press(at: t0), .start)
        XCTAssertEqual(m.release(at: t0 + 0.1), .nothing)
        XCTAssertTrue(m.isHandsFree)
        XCTAssertEqual(m.press(at: t0 + 5), .finish)
        XCTAssertEqual(m.release(at: t0 + 5.1), .nothing)
        XCTAssertFalse(m.isActive)
        XCTAssertEqual(m.press(at: t0 + 6), .start)
    }

    func testToggleBehavesLikeTap() {
        var m = HoldMachine()
        XCTAssertEqual(m.toggle(), .start)
        XCTAssertTrue(m.isHandsFree)
        XCTAssertEqual(m.toggle(), .finish)
        XCTAssertFalse(m.isActive)
    }
}

final class ObsidianTests: XCTestCase {
    func testNewNoteURL() {
        let utc = TimeZone(identifier: "UTC")!
        let date = Date(timeIntervalSince1970: 1_792_234_800)  // 2026-10-17 11:00 UTC
        let url = Obsidian.newNoteURL(text: "Hello, world & more?\nNext line", date: date, timeZone: utc)
        XCTAssertEqual(
            url?.absoluteString,
            "obsidian://new?vault=BaneyVault&file=0_Inbox/Dictation%202026-10-17%2011.00"
                + "&content=Hello%2C%20world%20%26%20more%3F%0ANext%20line"
        )
    }
}

final class FillerSafetyTests: XCTestCase {
    func testAcronymsAndUnitsSurvive() {
        XCTAssertEqual(Cleanup.apply("Take him to the ER."), "Take him to the ER.")
        XCTAssertEqual(Cleanup.apply("Use a 5 mm bolt"), "Use a 5 mm bolt")
        XCTAssertEqual(Cleanup.apply("Um, take him to the ER."), "Take him to the ER.")
    }
}

final class RoundTwoCleanupTests: XCTestCase {
    func testDigitRunsNeverCollapse() {
        XCTAssertEqual(Cleanup.apply("call five five five one two"), "call five five five one two")
        XCTAssertEqual(Cleanup.apply("code 5 5 5 1"), "code 5 5 5 1")
        XCTAssertEqual(Cleanup.apply("5 5 50"), "5 5 50")
    }

    func testFillersAfterNumbersGoExceptUnits() {
        XCTAssertEqual(Cleanup.apply("I have 3 uh questions"), "I have 3 questions")
        XCTAssertEqual(Cleanup.apply("about 20 um people"), "about 20 people")
        XCTAssertEqual(Cleanup.apply("We need 5, um, chairs"), "We need 5, chairs")
        XCTAssertEqual(Cleanup.apply("Use a 5 mm bolt"), "Use a 5 mm bolt")
    }

    func testGermanErAndUmAreWords() {
        XCTAssertEqual(Cleanup.apply("Er ist hier."), "Er ist hier.")
        XCTAssertEqual(Cleanup.apply("Um zu gehen, brauche ich Zeit."), "Um zu gehen, brauche ich Zeit.")
        // Unambiguous English fillers still go in German text, and English "um" still goes.
        XCTAssertEqual(Cleanup.apply("Er ist, uh, hier."), "Er ist, hier.")
        XCTAssertEqual(Cleanup.apply("Um, so we start."), "So we start.")
        XCTAssertEqual(Cleanup.apply("Er, I think so."), "I think so.")
    }
}

final class HistoryFileTests: XCTestCase {
    private func tmp() -> URL {
        let d = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }

    func testMissingRoundTripAndCorruptIsMovedAside() throws {
        let dir = tmp()
        let url = dir.appendingPathComponent("history.json")
        XCTAssertEqual(HistoryFile.load(from: url), .missing)
        let item = Dictation(raw: "a", text: "A", seconds: 1)
        try HistoryFile.save([item], to: url)
        guard case .ok(let items) = HistoryFile.load(from: url) else { return XCTFail("not ok") }
        XCTAssertEqual(items.map(\.id), [item.id])

        try Data("{truncated".utf8).write(to: url)
        guard case .corrupt(let aside?) = HistoryFile.load(from: url) else { return XCTFail("not corrupt") }
        XCTAssertTrue(FileManager.default.fileExists(atPath: aside.path))
        // The next save cannot destroy the corrupt copy.
        XCTAssertEqual(HistoryFile.load(from: url), .missing)
        XCTAssertEqual(try String(contentsOf: aside, encoding: .utf8), "{truncated")
    }

    func testExistingButUnreadableIsNeverMissing() throws {
        let dir = tmp()
        let url = dir.appendingPathComponent("history.json")
        try HistoryFile.save([], to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o000], ofItemAtPath: url.path)
        defer { try? FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path) }
        if getuid() == 0 { throw XCTSkip("root can read anything") }
        XCTAssertEqual(HistoryFile.load(from: url), .unreadable)
    }

    func testPendingAudioSurvivesAndIsRemoved() throws {
        let dir = tmp()
        let id = UUID()
        try PendingAudio.write([0.5, -0.25, 1], id: id, in: dir)
        let found = PendingAudio.list(in: dir)
        XCTAssertEqual(found.count, 1)
        XCTAssertEqual(found[0].id, id)
        XCTAssertEqual(found[0].samples, [0.5, -0.25, 1])
        PendingAudio.remove(id: id, in: dir)
        XCTAssertTrue(PendingAudio.list(in: dir).isEmpty)
    }

    func testPendingAudioIsExcludedFromBackup() throws {
        let dir = tmp()
        let id = UUID()
        try PendingAudio.write([0.1, 0.2], id: id, in: dir)
        let folder = PendingAudio.folder(in: dir)
        XCTAssertEqual(try folder.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
        let file = try XCTUnwrap(FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil).first)
        XCTAssertEqual(try file.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    }

    func testPendingAudioKeepsItsRecordedDate() throws {
        let dir = tmp()
        let id = UUID()
        let monday = Date(timeIntervalSince1970: 1_792_000_000)
        try PendingAudio.write([0.3], id: id, date: monday, in: dir)
        XCTAssertEqual(PendingAudio.list(in: dir).first?.date, monday)
        PendingAudio.remove(id: id, in: dir)
        XCTAssertTrue(PendingAudio.list(in: dir).isEmpty)
    }

    func testShortClipsArePaddedToFluidAudiosMinimum() {
        let short = [Float](repeating: 0.5, count: 5_600)
        let out = AudioPadding.padded(short)
        XCTAssertEqual(out.count, 16_000)
        XCTAssertEqual(Array(out.prefix(5_600)), short)
        XCTAssertTrue(out.dropFirst(5_600).allSatisfy { $0 == 0 })
        XCTAssertEqual(AudioPadding.padded([]).count, 16_000)
    }

    func testLongClipsAreNotTouched() {
        let long = [Float](repeating: 0.1, count: 16_000)
        XCTAssertEqual(AudioPadding.padded(long), long)
        let longer = [Float](repeating: 0.1, count: 40_000)
        XCTAssertEqual(AudioPadding.padded(longer).count, 40_000)
    }
}
