// Shortcuts, Siri and the Action button. "Dictate with Silas" opens Silas already listening
// hands-free; "Last Silas dictation" hands the last text to a shortcut without opening the app.

import AppIntents
import SilasCore

struct DictateIntent: AppIntent {
    static let title: LocalizedStringResource = "Dictate with Silas"
    static let description = IntentDescription("Opens Silas and starts listening hands-free. Tap the button to write.")
    static let openAppWhenRun = true

    @MainActor
    func perform() async throws -> some IntentResult {
        DictationModel.shared.startHandsFree(toKeyboard: false)
        return .result()
    }
}

struct LastDictationIntent: AppIntent {
    static let title: LocalizedStringResource = "Last Silas dictation"
    static let description = IntentDescription("The text of your most recent Silas dictation.")

    @MainActor
    func perform() async throws -> some IntentResult & ReturnsValue<String> & ProvidesDialog {
        let text = DictationModel.shared.history.first?.text ?? ""
        return .result(value: text, dialog: text.isEmpty ? "No dictations yet." : "\(text)")
    }
}

struct SilasShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: DictateIntent(),
            phrases: ["Dictate with \(.applicationName)", "Start \(.applicationName)"],
            shortTitle: "Dictate",
            systemImageName: "mic.fill"
        )
        AppShortcut(
            intent: LastDictationIntent(),
            phrases: ["Last \(.applicationName) dictation", "Get my last \(.applicationName) dictation"],
            shortTitle: "Last dictation",
            systemImageName: "text.quote"
        )
    }
}
