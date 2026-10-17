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
