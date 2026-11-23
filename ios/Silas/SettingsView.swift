import SilasCore
import SwiftUI
import UniformTypeIdentifiers

struct SettingsView: View {
    @Bindable var model: DictationModel
    @Bindable var sync: SyncController
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL

    @State private var shownCode: String?
    @State private var typedCode = ""
    @State private var newAddress = ""
    @State private var scanning = false
    @State private var pickingFolder = false
    @State private var confirmLeave = false
    @State private var confirmNewKey = false
    @State private var error: String?

    var body: some View {
        Form {
            Section("This device") {
                TextField("Name", text: $sync.deviceName)
                    .accessibilityIdentifier("silas.devicename")
            }

            syncSection

            keyboardSection

            Section {
                Text("Say “Dictate with Silas” to Siri, or put it on the Action button: Settings › Action Button › Shortcut. “Last Silas dictation” hands your last text to any shortcut.")
                    .foregroundStyle(Theme.text2)
            } header: {
                Text("Shortcuts")
            }

            Section("About") {
                LabeledContent("Speech model", value: "Parakeet TDT v3, on this device")
                LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "")
                Text("“By Silvanus, a faithful brother… I have written briefly.” 1 Peter 5:12")
                    .font(Theme.serif(15, weight: .regular).italic())
                    .foregroundStyle(Theme.text3)
            }
        }
        .scrollContentBackground(.hidden)
        .background(Theme.bg)
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        .sheet(isPresented: $scanning) {
            QRScannerSheet { code in
                scanning = false
                Task { await join(code) }
            }
        }
        .fileImporter(isPresented: $pickingFolder, allowedContentTypes: [.folder]) { result in
            if case .success(let url) = result {
                Task { await sync.setFolder(url) }
            }
        }
        .confirmationDialog("Leave sync?", isPresented: $confirmLeave, titleVisibility: .visible) {
            Button("Leave sync", role: .destructive) {
                shownCode = nil
                Task { await sync.leave() }
            }
        } message: {
            Text("This device keeps its dictionary and stops syncing. Pair again with a code to rejoin.")
        }
        .confirmationDialog("Make a new sync key?", isPresented: $confirmNewKey, titleVisibility: .visible) {
            Button("New sync key", role: .destructive) {
                Task {
                    do { shownCode = try await sync.newKey() } catch { self.error = error.localizedDescription }
                }
            }
        } message: {
            Text("Your other devices stop syncing with this one until you pair them again with the new code.")
        }
    }

    // MARK: Sync

    @ViewBuilder private var syncSection: some View {
        if let code = model.joinCode {
            Section {
                Text("Join sync with the code you scanned?")
                Text(code).font(.system(size: 13, design: .monospaced)).foregroundStyle(Theme.text2)
                Button("Join") {
                    model.joinCode = nil
                    Task { await join(code) }
                }
                Button("Not now", role: .cancel) { model.joinCode = nil }
            } header: {
                Text("Pair")
            }
        }

        Section {
            if sync.isOn {
                if let status = sync.statusText() {
                    Text(status).foregroundStyle(Theme.ok)
                } else {
                    Text("On. Waiting for your other devices.").foregroundStyle(Theme.text2)
                }
                if let problem = sync.problem {
                    Text(problem).foregroundStyle(Theme.red)
                }
                Button(sync.busy ? "Syncing…" : "Sync now") { Task { await sync.syncEverything() } }
                    .disabled(sync.busy)
                if let code = shownCode {
                    PairingCodeView(code: code, url: sync.pairURL)
                    Button("Hide code") { shownCode = nil }
                } else {
                    Button("Pair a device") { Task { await showCode() } }
                }
            } else {
                Text("Share your words, corrections and fillers with your other Silas devices. No server, no account: devices talk on your network or through a folder you already sync.")
                    .foregroundStyle(Theme.text2)
                Button("Pair a device") { Task { await showCode() } }
                    .accessibilityIdentifier("silas.sync.pair")
                if let code = shownCode {
                    PairingCodeView(code: code, url: sync.pairURL)
                }
            }
            if let error {
                Text(error).foregroundStyle(Theme.red)
            }
        } header: {
            Text("Sync")
        }

        Section {
            TextField("Pairing code", text: $typedCode)
                .textInputAutocapitalization(.characters)
                .autocorrectionDisabled()
                .font(.system(size: 15, design: .monospaced))
            Button("Join") { Task { await join(typedCode) } }
                .disabled(typedCode.trimmingCharacters(in: .whitespaces).isEmpty)
            Button {
                scanning = true
            } label: {
                Label("Scan the code on your computer", systemImage: "qrcode.viewfinder")
            }
        } header: {
            Text("Join with a code")
        } footer: {
            Text("On your computer: Silas › Settings › Sync › Pair a device.")
        }

        if sync.isOn {
            Section("Devices") {
                if sync.peers.isEmpty {
                    Text("None yet").foregroundStyle(Theme.text3)
                }
                ForEach(sync.peers) { p in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(p.name)
                            Text(platformName(p.platform)).font(.system(size: 13)).foregroundStyle(Theme.text3)
                        }
                        Spacer()
                        Text(p.lastSynced, format: .relative(presentation: .named))
                            .font(.system(size: 13)).foregroundStyle(Theme.text3)
                    }
                }
            }

            Section {
                ForEach(sync.addresses, id: \.self) { a in
                    Text(a)
                        .swipeActions {
                            Button("Remove", role: .destructive) { Task { await sync.removeAddress(a) } }
                        }
                }
                HStack {
                    TextField("host:port", text: $newAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    Button("Add") {
                        let a = newAddress
                        newAddress = ""
                        Task { await sync.addAddress(a) }
                    }
                    .disabled(newAddress.isEmpty)
                }
            } header: {
                Text("Peers by address")
            } footer: {
                Text("For networks where devices can't find each other, such as a tailnet: rectory1:47823.")
            }

            Section {
                if let name = sync.folderName {
                    LabeledContent("Folder", value: name)
                    Button("Choose another folder") { pickingFolder = true }
                    Button("Stop using this folder", role: .destructive) { sync.clearFolder() }
                } else {
                    Button("Choose a folder") { pickingFolder = true }
                }
            } header: {
                Text("Sync folder")
            } footer: {
                Text("For devices that are never on the same network: pick a folder in iCloud Drive or another synced drive. Silas keeps an encrypted file in “Silas Sync” there.")
            }

            Section {
                Button("New sync key") { confirmNewKey = true }
                Button("Leave sync", role: .destructive) { confirmLeave = true }
            }
        }
    }

    // MARK: Keyboard

    @ViewBuilder private var keyboardSection: some View {
        Section {
            if model.keyboardAvailable {
                Text("Settings › General › Keyboard › Keyboards › Add New Keyboard › Silas. Then tap Silas and turn on Allow Full Access, which lets the keyboard read the text Silas wrote for it. Nothing is sent anywhere.")
                    .foregroundStyle(Theme.text2)
                Text("In any app, switch to the Silas keyboard and tap the mic. Silas opens and listens; go back and the text is typed for you. For five minutes after, the mic key works without leaving the app.")
                    .foregroundStyle(Theme.text2)
                Button("Open Silas in Settings") {
                    if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
                }
                .accessibilityIdentifier("silas.opensettings")
            } else {
                Text("This build has no App Group, so the Silas Keyboard can't get text from the app. Reinstall with ios/scripts/install.sh.")
                    .foregroundStyle(Theme.text2)
            }
        } header: {
            Text("Silas Keyboard")
        }
    }

    // MARK: Actions

    private func showCode() async {
        do {
            shownCode = try await sync.pairingCode()
            error = nil
        } catch {
            self.error = "Couldn't save the sync key: \(error.localizedDescription)"
        }
    }

    private func join(_ code: String) async {
        do {
            try await sync.join(code: code)
            typedCode = ""
            error = nil
        } catch {
            self.error = "That code isn't valid. Check it on the other device and try again."
        }
    }

    private func platformName(_ p: String) -> String {
        switch p {
        case "macos": return "Mac"
        case "windows": return "Windows"
        case "linux": return "Linux"
        case "ios": return "iPhone"
        case "ipados": return "iPad"
        default: return p
        }
    }
}

/// The pairing code in groups, with a QR code of `silas://pair?k=` for another phone or iPad.
struct PairingCodeView: View {
    let code: String
    let url: URL?

    var body: some View {
        VStack(spacing: 12) {
            if let url, let image = QRCode.image(for: url.absoluteString) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: 200, height: 200)
                    .padding(10)
                    .background(Color.white)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .accessibilityLabel("Pairing QR code")
            }
            Text(code)
                .font(.system(size: 15, weight: .medium, design: .monospaced))
                .foregroundStyle(Theme.goldInk)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
            Text("Enter or scan this on your other device.")
                .font(.system(size: 13))
                .foregroundStyle(Theme.text3)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 8)
    }
}
