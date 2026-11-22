// UI checks run by `xcodebuild test` on the iPhone 17 simulator (see ios/README.md). Each test
// keeps a screenshot; ios/scripts/screenshots.sh exports them to ios/docs/.

import XCTest

final class SilasUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    override func tearDown() {
        XCUIApplication(bundleIdentifier: "com.apple.Preferences").terminate()
    }

    /// The app opens to the dictation screen.
    func testLaunch() {
        let app = XCUIApplication()
        app.launchArguments = ["-uitest"]
        app.launch()
        XCTAssertTrue(app.buttons["silas.hold"].waitForExistence(timeout: 30))
        XCTAssertTrue(element(app, "silas.status").exists)
        keep("launch")
    }

    /// The speech model loads (downloading it on a fresh simulator) and the bundled clip,
    /// made with `say "Silas writes what you say."`, comes back as text.
    func testModelReadyAndSelfTest() {
        let app = XCUIApplication()
        app.launchArguments = ["-uitest", "-selftest-bundled"]
        app.launch()
        let result = element(app, "silas.selftest")
        // A fresh simulator downloads about 480 MB first.
        XCTAssertTrue(result.waitForExistence(timeout: 1800), "the self-test never finished")
        XCTAssertTrue(element(app, "silas.status").label.contains("Ready"), element(app, "silas.status").label)
        let text = result.label.lowercased()
        XCTAssertTrue(text.contains("writes what you say"), "self-test heard: \(result.label)")
        keep("ready")
    }

    /// Turn on the Silas Keyboard with Full Access, then show it on a text field: the mic starts
    /// a dictation, its text is typed there, and "type last again" types it once more.
    func testKeyboardTypesHandedOffText() {
        let sentence = "Hello from Silas."
        let app = launchWithKeyboard(answering: sentence)
        let field = element(app, "silas.testfield")
        let mic = showSilasKeyboard(on: field, in: app)
        mic.tap()
        let typed = waitFor(field, toRead: sentence)
        keep("keyboard")
        XCTAssertTrue(typed, "field reads: \(field.value as? String ?? ""):\n\(app.debugDescription)")

        app.buttons["silas.kb.last"].tap()
        XCTAssertTrue(waitFor(field, toRead: sentence + " " + sentence), "field reads: \(field.value as? String ?? "")")
    }

    /// A dictation started in one field and finished after the person moved to another is not
    /// typed into the new field; "type last again" types it there when asked.
    func testKeyboardKeepsTextForItsOwnField() {
        let sentence = "Meant for the first field."
        let app = launchWithKeyboard(answering: sentence)
        let first = element(app, "silas.testfield")
        let second = element(app, "silas.testfield2")
        let mic = showSilasKeyboard(on: first, in: app)
        mic.tap()
        second.tap()
        // The answer comes 4 s after the start; give it time to arrive and be (wrongly) typed.
        sleep(8)
        XCTAssertFalse(value(of: second).contains(sentence), "typed into the field it wasn't for: \(value(of: second))")
        XCTAssertFalse(value(of: first).contains(sentence), "typed into a field not in use: \(value(of: first))")

        app.buttons["silas.kb.last"].tap()
        XCTAssertTrue(waitFor(second, toRead: sentence), "field reads: \(value(of: second))")
        // A dictation started here is typed here.
        mic.tap()
        XCTAssertTrue(waitFor(second, toRead: sentence + " " + sentence), "field reads: \(value(of: second))")
        keep("keyboard-own-field")
    }

    // MARK: Helpers

    /// Turn the keyboard on, then launch the keyboard test screen with a session whose every
    /// dictation comes back as `sentence`.
    private func launchWithKeyboard(answering sentence: String) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-uitest", "-keyboardtest"]
        app.launch()
        XCTAssertEqual(element(app, "silas.appgroup").label, "App Group: on")
        enableKeyboard(from: app)
        app.terminate()
        app.launchArguments = ["-uitest", "-keyboardtest", "-handoff", sentence]
        app.launch()
        return app
    }

    /// Focus `field`, pick the Silas Keyboard from the globe key's list, and return its mic key
    /// once a session is armed.
    private func showSilasKeyboard(on field: XCUIElement, in app: XCUIApplication) -> XCUIElement {
        XCTAssertTrue(field.waitForExistence(timeout: 15))
        field.tap()
        let globe = app.keys["Next keyboard"].exists ? app.keys["Next keyboard"] : app.buttons["Next keyboard"]
        XCTAssertTrue(globe.waitForExistence(timeout: 10), "no globe key:\n\(app.debugDescription)")
        globe.press(forDuration: 1.2)
        let choice = app.descendants(matching: .any).matching(NSPredicate(format: "label == 'Silas'")).firstMatch
        if choice.waitForExistence(timeout: 5) {
            choice.tap()
        } else {
            keep("keyboard-list")
            XCTFail("no Silas in the keyboard list:\n\(app.debugDescription)")
        }
        let mic = app.buttons["silas.kb.mic"]
        XCTAssertTrue(mic.waitForExistence(timeout: 10), "the Silas Keyboard's keys aren't visible:\n\(app.debugDescription)")
        let armed = NSPredicate(format: "value == 'armed'")
        XCTAssertEqual(
            XCTWaiter().wait(for: [XCTNSPredicateExpectation(predicate: armed, object: mic)], timeout: 15), .completed,
            "the keyboard never saw the session: \(mic.value as? String ?? "")")
        return mic
    }

    private func value(of field: XCUIElement) -> String { field.value as? String ?? "" }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    private func waitFor(_ field: XCUIElement, toRead text: String, timeout: TimeInterval = 15) -> Bool {
        let p = NSPredicate(format: "value == %@", text)
        return XCTWaiter().wait(for: [XCTNSPredicateExpectation(predicate: p, object: field)], timeout: timeout) == .completed
    }

    /// Settings › General › Keyboard › Keyboards: add Silas, then turn on Allow Full Access.
    private func enableKeyboard(from app: XCUIApplication) {
        let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        settings.terminate()
        settings.launch()
        // Settings reopens where it was left: go back to the top first.
        let general = settings.buttons["com.apple.settings.general"]
        for _ in 0..<6 where !(general.waitForExistence(timeout: 2) && general.isHittable) {
            let back = settings.navigationBars.buttons.firstMatch
            guard back.exists else { break }
            back.tap()
        }
        tap("com.apple.settings.general", in: settings)
        tap("com.apple.settings.general.keyboard", in: settings)
        tap("Keyboards", in: settings, prefix: true)
        // The Silas row is identified by the keyboard's bundle id once it has been added.
        let row = "com.baney.silas.ios.keyboard"
        if !settings.cells[row].waitForExistence(timeout: 3) {
            tap("AddNewKeyboard", in: settings)
            tap("com.baney.silas.ios", in: settings)
        }
        tap(row, in: settings)
        turnOn(settings.switches["Allow Full Access"], in: settings)
        let allow = settings.alerts.buttons["Allow"]
        if allow.waitForExistence(timeout: 5) { allow.tap() }
        let full = settings.switches["Allow Full Access"]
        XCTAssertTrue(full.waitForExistence(timeout: 5))
        XCTAssertEqual(full.value as? String, "1", "Full Access is off:\n\(settings.debugDescription)")
        keep("keyboard-settings")
        app.activate()
    }

    /// Tap the first hittable button, cell or text labelled `label` (or starting with it),
    /// scrolling down to find it.
    private func tap(_ label: String, in app: XCUIApplication, prefix: Bool = false) {
        let p = prefix
            ? NSPredicate(format: "label BEGINSWITH %@ OR identifier == %@", label, label)
            : NSPredicate(format: "label == %@ OR identifier == %@", label, label)
        func find() -> XCUIElement? {
            for query in [app.buttons, app.cells, app.staticTexts] {
                if let hit = query.matching(p).allElementsBoundByIndex.first(where: { $0.exists && $0.frame.width > 0 && $0.isHittable }) {
                    return hit
                }
            }
            return nil
        }
        let deadline = Date().addingTimeInterval(8)
        var target = find()
        while target == nil && Date() < deadline {
            usleep(300_000)
            target = find()
        }
        var swipes = 0
        while target == nil && swipes < 5 {
            app.swipeUp()
            swipes += 1
            target = find()
        }
        guard let target else {
            XCTFail("no \(label) in Settings:\n\(app.debugDescription)")
            return
        }
        target.tap()
    }

    private func turnOn(_ toggle: XCUIElement, in settings: XCUIApplication) {
        XCTAssertTrue(toggle.waitForExistence(timeout: 10), "no switch \(toggle):\n\(settings.debugDescription)")
        if toggle.value as? String != "1" {
            // The switch's frame covers the whole row; tap the knob end.
            toggle.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)).tap()
        }
    }

    private func keep(_ name: String) {
        let a = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }
}
