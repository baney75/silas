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
            if phase == .background {
                model.finishIfRecording()
                model.saveNow()
            }
        }
    }
}
