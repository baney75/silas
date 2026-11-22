import SilasCore
import SwiftUI

struct ContentView: View {
    @Bindable var model: DictationModel
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        Group {
            if sizeClass == .regular {
                WideLayout(model: model)
            } else {
                PhoneLayout(model: model)
            }
        }
        .foregroundStyle(Theme.text)
        .sheet(item: $model.sheet) { sheet in
            NavigationStack {
                switch sheet {
                case .history: HistoryList(model: model, inSheet: true)
                case .dictionary: DictionaryView(model: model)
                case .settings: SettingsView(model: model, sync: model.sync)
                }
            }
            .preferredColorScheme(.dark)
            .tint(Theme.gold)
            .presentationBackground(Theme.bg)
        }
    }
}

// MARK: - Layouts

/// iPhone, and an iPad in a narrow window: transcript on top, the button at thumb height.
struct PhoneLayout: View {
    @Bindable var model: DictationModel

    var body: some View {
        ZStack {
            Theme.bg.ignoresSafeArea()
            VStack(spacing: 16) {
                Header(model: model, showHistory: true)
                TranscriptCard(model: model)
                StatusLine(model: model)
                HoldButton(model: model, size: 148)
                Hint(model: model)
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 12)
        }
    }
}

/// iPad: dictation on the left, history on the right (or below, in portrait).
struct WideLayout: View {
    @Bindable var model: DictationModel

    var body: some View {
        GeometryReader { geo in
            let wide = geo.size.width > geo.size.height
            ZStack {
                Theme.bg.ignoresSafeArea()
                // One layout container for both orientations keeps every child's identity, so a
                // hold in progress survives a rotation or a Stage Manager resize.
                let layout = wide
                    ? AnyLayout(HStackLayout(spacing: 0))
                    : AnyLayout(VStackLayout(spacing: 0))
                layout {
                    VStack(spacing: 24) {
                        Header(model: model, showHistory: false)
                        VStack(spacing: 18) {
                            HoldButton(model: model, size: 200)
                            Hint(model: model)
                            StatusLine(model: model)
                        }
                        .frame(maxWidth: .infinity)
                        TranscriptCard(model: model)
                    }
                    .padding(.horizontal, wide ? 40 : 24)
                    .padding(.top, 24)
                    .padding(.bottom, 28)
                    Rectangle().fill(Theme.hairline.opacity(0.5))
                        .frame(width: wide ? 1 : nil, height: wide ? nil : 1)
                        .ignoresSafeArea(edges: wide ? .vertical : [])
                    HistoryList(model: model, inSheet: false)
                        .frame(
                            width: wide ? min(400, geo.size.width * 0.32) : nil,
                            height: wide ? nil : geo.size.height * 0.3
                        )
                        .background(Theme.side.ignoresSafeArea())
                }
            }
        }
    }
}

// MARK: - Header

struct Header: View {
    @Bindable var model: DictationModel
    let showHistory: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image("Mark")
                .resizable()
                .frame(width: 34, height: 34)
                .accessibilityHidden(true)
            Text("Silas").font(Theme.serif(28))
                .accessibilityAddTraits(.isHeader)
            Spacer()
            if showHistory {
                iconButton("clock.arrow.circlepath", "History") { model.sheet = .history }
            }
            iconButton("character.book.closed", "Dictionary") { model.sheet = .dictionary }
            iconButton("gearshape", "Settings") { model.sheet = .settings }
        }
    }

    private func iconButton(_ symbol: String, _ label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 19, weight: .medium))
                .foregroundStyle(Theme.text2)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .accessibilityLabel(label)
        .accessibilityIdentifier("silas.\(label.lowercased())")
    }
}

struct Hint: View {
    @Bindable var model: DictationModel

    var body: some View {
        Group {
            if model.status == .listening && model.handsFree {
                Text("Hands-free. Tap again to write.")
            } else if model.status == .listening {
                Text("Release to write")
            } else {
                Text("Hold to speak · Tap for hands-free")
            }
        }
        .font(.system(size: 15))
        .foregroundStyle(Theme.text2)
        .multilineTextAlignment(.center)
    }
}

// MARK: - Hold button

struct HoldButton: View {
    @Bindable var model: DictationModel
    var size: CGFloat = 200
    /// Resets by itself when the system cancels the touch (permission alert, call, Control Center).
    @GestureState private var pressed = false

    var body: some View {
        let listening = model.status == .listening
        let writing = model.status == .writing
        ZStack {
            // Glow that follows the voice.
            Circle()
                .fill(Theme.gold.opacity(0.16))
                .frame(width: size, height: size)
                .scaleEffect(listening ? 1.08 + CGFloat(model.level) * 0.35 : 1)
                .opacity(listening ? 1 : 0)
                .animation(.easeOut(duration: 0.12), value: model.level)
            Circle()
                .fill(listening ? Theme.gold : Theme.surface2)
                .overlay(Circle().strokeBorder(listening ? Theme.goldHi : Theme.goldLine, lineWidth: 2))
                .frame(width: size, height: size)
                .shadow(color: .black.opacity(0.5), radius: 16, y: 8)
            if writing {
                ProgressView().controlSize(.large).tint(Theme.gold)
            } else {
                Image(systemName: listening ? "waveform" : "mic.fill")
                    .font(.system(size: size * 0.32, weight: .medium))
                    .foregroundStyle(listening ? Theme.primaryFg : Theme.gold)
                    .symbolEffect(.variableColor.iterative, isActive: listening)
            }
        }
        .scaleEffect(pressed ? 0.96 : 1)
        .animation(.spring(response: 0.25, dampingFraction: 0.7), value: pressed)
        .animation(.easeInOut(duration: 0.2), value: model.status)
        .opacity(model.canRecord || writing ? 1 : 0.45)
        .contentShape(Circle())
        .gesture(
            DragGesture(minimumDistance: 0)
                .updating($pressed) { _, state, _ in state = true }
        )
        .onChange(of: pressed) { _, isDown in
            if isDown { model.press() } else { model.release() }
        }
        .onDisappear { if pressed { model.release() } }
        .overlay(alignment: .topTrailing) {
            if listening {
                Button {
                    model.cancel()
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 15, weight: .bold))
                        .foregroundStyle(Theme.text2)
                        .frame(width: 40, height: 40)
                        .background(Circle().fill(Theme.surface2))
                        .overlay(Circle().strokeBorder(Theme.lineStrong))
                }
                .offset(x: 24, y: -8)
                .accessibilityLabel("Cancel")
                .keyboardShortcut(.escape, modifiers: [])
            }
        }
        .accessibilityElement()
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(listening ? "Stop and write" : "Dictate")
        .accessibilityHint("Starts listening. Activate again to write what you said.")
        .accessibilityIdentifier("silas.hold")
        .accessibilityAction { model.toggle() }
        .accessibilityAction(named: "Cancel") { if listening { model.cancel() } }
    }
}

// MARK: - Status

struct StatusLine: View {
    @Bindable var model: DictationModel

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 10) {
                Circle().fill(dotColor).frame(width: 9, height: 9)
                Text(model.status.label)
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(model.status == .ready ? Theme.ok : model.status == .listening ? Theme.gold : Theme.text)
                    .contentTransition(.numericText())
                if model.keyboardSession {
                    Text("· Keyboard")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundStyle(Theme.text3)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
            .background(Capsule().fill(Theme.surface))
            .overlay(Capsule().strokeBorder(Theme.line))
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("silas.status")

            switch model.status {
            case .downloading(let p):
                VStack(spacing: 6) {
                    ProgressView(value: p).tint(Theme.gold).frame(maxWidth: 320)
                    Text("One-time download, about 480 MB. After this, Silas works offline.")
                        .font(.system(size: 13))
                        .foregroundStyle(Theme.text3)
                        .multilineTextAlignment(.center)
                }
            case .error(let message):
                VStack(spacing: 8) {
                    Text(message).font(.system(size: 14)).foregroundStyle(Theme.red)
                        .multilineTextAlignment(.center)
                    Button("Try again") { model.dismissError() }
                        .buttonStyle(SecondaryButton())
                }
            default:
                if model.hasFailedClip {
                    VStack(spacing: 8) {
                        Text(model.failedClipCount > 1 ? "\(model.failedClipCount) recordings couldn't be transcribed." : "A recording couldn't be transcribed.")
                            .font(.system(size: 14)).foregroundStyle(Theme.red)
                        HStack(spacing: 10) {
                            Button("Try again") { model.retryFailedClip() }
                                .buttonStyle(SecondaryButton())
                            Button("Discard", role: .destructive) { model.discardFailedClip() }
                                .buttonStyle(SecondaryButton())
                        }
                    }
                } else {
                    Text(model.notice ?? " ")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.text3)
                        .lineLimit(2)
                        .multilineTextAlignment(.center)
                        .animation(.default, value: model.notice)
                }
            }
            #if DEBUG
            if let result = model.selfTestResult {
                Text(result)
                    .font(.system(size: 12))
                    .foregroundStyle(Theme.text3)
                    .accessibilityIdentifier("silas.selftest")
            }
            #endif
        }
    }

    private var dotColor: Color {
        switch model.status {
        case .ready: return Theme.ok
        case .listening: return Theme.red
        case .writing, .downloading, .loading: return Theme.gold
        case .error: return Theme.red
        }
    }
}

// MARK: - Transcript

struct TranscriptCard: View {
    @Bindable var model: DictationModel
    @Environment(\.openURL) private var openURL
    @FocusState private var editing: Bool

    private var live: Bool { model.status == .listening || model.status == .writing }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(live ? "LIVE" : "TRANSCRIPT")
                    .sectionLabel()
                    .foregroundStyle(live ? Theme.goldInk : Theme.text3)
                Spacer()
                if editing {
                    Button("Done") { editing = false }
                        .font(.system(size: 15, weight: .semibold))
                }
            }
            ZStack(alignment: .topLeading) {
                if live {
                    ScrollView {
                        Text(model.preview.isEmpty ? (model.status == .writing ? "Writing…" : "Listening…") : model.preview)
                            .font(.system(size: 21))
                            .lineSpacing(5)
                            .foregroundStyle(model.preview.isEmpty ? Theme.text3 : Theme.text2)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.top, 8)
                            .padding(.leading, 5)
                            .accessibilityIdentifier("silas.preview")
                    }
                    .defaultScrollAnchor(.bottom)
                } else {
                    if model.text.isEmpty {
                        Text("What you say shows up here. Edit it, and Silas learns your spelling.")
                            .font(.system(size: 21))
                            .foregroundStyle(Theme.text3)
                            .padding(.top, 8)
                            .padding(.leading, 5)
                            .allowsHitTesting(false)
                    }
                    TextEditor(text: $model.text)
                        .font(.system(size: 21))
                        .lineSpacing(5)
                        .scrollContentBackground(.hidden)
                        .focused($editing)
                        .accessibilityIdentifier("silas.transcript")
                }
            }
            .frame(maxHeight: .infinity)

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    copyButton
                    shareButton
                    Spacer(minLength: 0)
                    obsidianButton
                }
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 10) {
                        copyButton
                        shareButton
                    }
                    obsidianButton
                }
            }
            .disabled(live || model.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .padding(16)
        .card()
        .onChange(of: editing) { _, now in
            if !now { model.commitEdit() }
        }
    }

    private var copyButton: some View {
        Button {
            model.copy()
        } label: {
            Label("Copy", systemImage: "doc.on.doc")
        }
        .buttonStyle(SecondaryButton())
    }

    private var shareButton: some View {
        ShareLink(item: model.text) {
            Label("Share", systemImage: "square.and.arrow.up")
        }
        .buttonStyle(SecondaryButton())
        .simultaneousGesture(TapGesture().onEnded { model.commitEdit() })
    }

    private var obsidianButton: some View {
        Button {
            model.commitEdit()
            guard let url = model.obsidianURL else { return }
            openURL(url) { accepted in
                if !accepted { model.show("Obsidian didn't open. Is it installed?") }
            }
        } label: {
            Label("Save to Obsidian", systemImage: "square.and.arrow.down")
        }
        .buttonStyle(PrimaryButton())
    }
}

// MARK: - History

struct HistoryList: View {
    @Bindable var model: DictationModel
    let inSheet: Bool
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !inSheet {
                Text("HISTORY")
                    .sectionLabel()
                    .padding(.horizontal, 24)
                    .padding(.top, 24)
                    .padding(.bottom, 10)
            }
            if model.history.isEmpty {
                Text("Your dictations will show up here.")
                    .font(.system(size: 16))
                    .foregroundStyle(Theme.text3)
                    .padding(.horizontal, 24)
                    .padding(.top, inSheet ? 16 : 0)
                Spacer()
            } else {
                List {
                    ForEach(model.history) { item in
                        HistoryRow(item: item, selected: item.id == model.selectedID)
                            .contentShape(Rectangle())
                            .onTapGesture {
                                model.select(item)
                                if inSheet { dismiss() }
                            }
                            .listRowBackground(Color.clear)
                            .listRowSeparatorTint(Theme.line)
                            .listRowInsets(EdgeInsets(top: 4, leading: 12, bottom: 4, trailing: 12))
                            .swipeActions {
                                Button(role: .destructive) {
                                    model.delete(item)
                                } label: {
                                    Label("Delete", systemImage: "trash")
                                }
                            }
                            .accessibilityAddTraits(.isButton)
                    }
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(inSheet ? Theme.bg : Color.clear)
        .navigationTitle(inSheet ? "History" : "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if inSheet {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

struct HistoryRow: View {
    let item: Dictation
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(item.date, format: .dateTime.weekday(.abbreviated).month(.abbreviated).day().hour().minute())
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(selected ? Theme.goldInk : Theme.text3)
            Text(item.text)
                .font(.system(size: 16))
                .foregroundStyle(Theme.text)
                .lineLimit(3)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(selected ? Theme.goldSoft : Color.clear)
        )
    }
}
