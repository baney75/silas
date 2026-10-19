import SwiftUI

@main
struct SilasApp: App {
    @State private var model = DictationModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
                .preferredColorScheme(.dark)
                .tint(Theme.gold)
                .task { await model.prepare() }
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
