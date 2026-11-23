#if DEBUG
import SwiftUI

/// Debug builds only, for the UI test (`-keyboardtest`): two plain text fields to type into with
/// the Silas Keyboard, a button that opens Silas's page in Settings to turn the keyboard on, and
/// `-handoff <text>`, which answers each dictation the keyboard starts with that text.
struct KeyboardTestView: View {
    @Bindable var model: DictationModel
    @State private var text = ""
    @State private var other = ""
    @Environment(\.openURL) private var openURL

    var body: some View {
        ZStack {
            Theme.bg.ignoresSafeArea()
            VStack(alignment: .leading, spacing: 16) {
                Text("Keyboard test").font(Theme.serif(24))
                TextField("Type here", text: $text, axis: .vertical)
                    .font(.system(size: 19))
                    .padding(12)
                    .card(radius: 12)
                    .accessibilityIdentifier("silas.testfield")
                TextField("Or here", text: $other, axis: .vertical)
                    .font(.system(size: 19))
                    .padding(12)
                    .card(radius: 12)
                    .accessibilityIdentifier("silas.testfield2")
                Button("Open Silas in Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
                .buttonStyle(SecondaryButton())
                .accessibilityIdentifier("silas.opensettings")
                Text(model.keyboardAvailable ? "App Group: on" : "App Group: off")
                    .foregroundStyle(Theme.text3)
                    .accessibilityIdentifier("silas.appgroup")
                Spacer()
            }
            .padding(20)
        }
        .foregroundStyle(Theme.text)
        .onAppear {
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-handoff"), i + 1 < args.count {
                model.debugHandoff(args[i + 1])
            }
        }
    }
}
#endif
