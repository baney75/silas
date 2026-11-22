import SwiftUI

@main
struct SilasApp: App {
    @State private var model = DictationModel.shared
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .preferredColorScheme(.dark)
                .tint(Theme.gold)
                .task { await model.prepare() }
                .onOpenURL { model.open($0) }
        }
        .onChange(of: scenePhase) { _, phase in
            switch phase {
            case .background:
                model.leavingForeground()
            case .active:
                model.becameActive()
            default:
                break
            }
        }
    }
}

/// The app, or in debug builds the keyboard test screen the UI test asks for.
struct RootView: View {
    @Bindable var model: DictationModel

    var body: some View {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("-keyboardtest") {
            KeyboardTestView(model: model)
        } else {
            ContentView(model: model)
        }
        #else
        ContentView(model: model)
        #endif
    }
}
