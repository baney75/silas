import SwiftUI
import SilasCore

struct ContentView: View {
    @Bindable var model: DictationModel

    var body: some View {
        GeometryReader { geo in
            let wide = geo.size.width > geo.size.height
            ZStack {
                Theme.bg.ignoresSafeArea()
                if wide {
                    HStack(spacing: 0) {
                        DictatePane(model: model, wide: true)
                        Rectangle().fill(Theme.line).frame(width: 1).ignoresSafeArea()
                        HistoryPane(model: model)
                            .frame(width: min(400, geo.size.width * 0.32))
                            .background(Theme.side.ignoresSafeArea())
                    }
                } else {
                    VStack(spacing: 0) {
                        DictatePane(model: model, wide: false)
                        Rectangle().fill(Theme.line).frame(height: 1)
                        HistoryPane(model: model)
                            .frame(height: geo.size.height * 0.3)
                            .background(Theme.side.ignoresSafeArea())
                    }
                }
            }
        }
        .foregroundStyle(Theme.text)
    }
}

// MARK: - Dictate

struct DictatePane: View {
    @Bindable var model: DictationModel
    let wide: Bool

    var body: some View {
        VStack(spacing: 24) {
            header
            VStack(spacing: 18) {
                HoldButton(model: model)
                hint
                StatusLine(model: model)
            }
            .frame(maxWidth: .infinity)
            TranscriptCard(model: model)
        }
        .padding(.horizontal, 40)
        .padding(.top, 24)
        .padding(.bottom, 28)
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image("Mark")
                .resizable()
                .frame(width: 36, height: 36)
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                .accessibilityHidden(true)
            Text("Silas").font(Theme.serif(28))
            Spacer()
            Text("Speak. Silas writes.")
                .font(Theme.serif(17, weight: .regular).italic())
                .foregroundStyle(Theme.text3)
        }
    }

    private var hint: some View {
        HStack(spacing: 8) {
            if model.status == .listening && model.handsFree {
                Text("Hands-free. Tap again to write.")
            } else {
                Text("Hold to speak, release to write")
                Text("·").foregroundStyle(Theme.text3).accessibilityHidden(true)
                Text("Tap for hands-free")
            }
        }
        .font(.system(size: 17))
        .foregroundStyle(Theme.text2)
    }
}

// MARK: - Hold button

struct HoldButton: View {
    @Bindable var model: DictationModel
    /// Resets by itself when the system cancels the touch (permission alert, call, Control Center).
    @GestureState private var pressed = false

    private let size: CGFloat = 200

    var body: some View {
        let listening = model.status == .listening
        let writing = model.status == .writing
        ZStack {
            // Glow that follows the voice.
            Circle()
                .fill(Theme.gold.opacity(0.18))
                .frame(width: size, height: size)
                .scaleEffect(listening ? 1.08 + CGFloat(model.level) * 0.35 : 1)
                .opacity(listening ? 1 : 0)
                .animation(.easeOut(duration: 0.12), value: model.level)
            Circle()
                .fill(listening ? Theme.gold : Theme.surface2)
                .overlay(Circle().strokeBorder(listening ? Theme.gold : Theme.goldLine, lineWidth: 2))
                .frame(width: size, height: size)
                .shadow(color: .black.opacity(0.45), radius: 16, y: 8)
            if writing {
                ProgressView().controlSize(.large).tint(Theme.gold)
            } else {
                Image(systemName: listening ? "waveform" : "mic.fill")
                    .font(.system(size: 64, weight: .medium))
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
                        .overlay(Circle().strokeBorder(Theme.line))
                }
                .offset(x: 28, y: -8)
                .accessibilityLabel("Cancel")
                .keyboardShortcut(.escape, modifiers: [])
            }
        }
        .accessibilityElement()
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel(listening ? "Stop and write" : "Dictate")
        .accessibilityHint("Starts listening. Activate again to write what you said.")
        .accessibilityAction { model.toggle() }
        .accessibilityAction(named: "Cancel") { if listening { model.cancel() } }
    }
}

// MARK: - Status

struct StatusLine: View {
    @Bindable var model: DictationModel

    var body: some View {
        VStack(spacing: 10) {
            HStack(spacing: 10) {
                Circle().fill(dotColor).frame(width: 10, height: 10)
                Text(model.status.label)
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(model.status == .listening ? Theme.gold : Theme.text)
                    .contentTransition(.numericText())
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            .background(Capsule().fill(Theme.surface))
            .overlay(Capsule().strokeBorder(Theme.line))
            .accessibilityElement(children: .combine)

            switch model.status {
            case .downloading(let p):
                VStack(spacing: 6) {
                    ProgressView(value: p).tint(Theme.gold).frame(width: 320)
                    Text("One-time download, about 480 MB. After this, Silas works offline.")
                        .font(.system(size: 14))
                        .foregroundStyle(Theme.text3)
                }
            case .error(let message):
                HStack(spacing: 12) {
                    Text(message).font(.system(size: 15)).foregroundStyle(Theme.red)
                    Button("Try again") { model.dismissError() }
                        .buttonStyle(.bordered)
                }
            default:
                Text(model.notice ?? " ")
                    .font(.system(size: 15))
                    .foregroundStyle(Theme.text3)
                    .animation(.default, value: model.notice)
            }
        }
    }

    private var dotColor: Color {
        switch model.status {
        case .ready: return Theme.green
        case .listening, .writing, .downloading, .loading: return Theme.gold
        case .error: return Theme.red
        }
    }
}

// MARK: - Transcript

struct TranscriptCard: View {
    @Bindable var model: DictationModel
    @Environment(\.openURL) private var openURL
    @FocusState private var editing: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("TRANSCRIPT")
                    .font(.system(size: 13, weight: .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Theme.text3)
                Spacer()
                if editing {
                    Button("Done") { editing = false }
                        .font(.system(size: 15, weight: .semibold))
                }
            }
            ZStack(alignment: .topLeading) {
                if model.text.isEmpty {
                    Text("What you say shows up here. You can edit it before you copy or save it.")
                        .font(.system(size: 22))
                        .foregroundStyle(Theme.text3)
                        .padding(.top, 8)
                        .padding(.leading, 5)
                        .allowsHitTesting(false)
                }
                TextEditor(text: $model.text)
                    .font(.system(size: 22))
                    .lineSpacing(5)
                    .scrollContentBackground(.hidden)
                    .focused($editing)
            }
            .frame(maxHeight: .infinity)

            HStack(spacing: 12) {
                Button {
                    model.copy()
                } label: {
                    Label("Copy", systemImage: "doc.on.doc")
                }
                .buttonStyle(SecondaryButton())

                ShareLink(item: model.text) {
                    Label("Share", systemImage: "square.and.arrow.up")
                }
                .buttonStyle(SecondaryButton())

                Spacer()

                Button {
                    guard let url = model.obsidianURL else { return }
                    openURL(url) { accepted in
                        if !accepted { model.show("Obsidian didn't open. Is it installed?") }
                    }
                } label: {
                    Label("Save to Obsidian", systemImage: "square.and.arrow.down")
                }
                .buttonStyle(PrimaryButton())
            }
            .disabled(model.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .padding(22)
        .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Theme.surface))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Theme.line))
    }
}

struct PrimaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(Theme.primaryFg)
            .padding(.horizontal, 20)
            .frame(height: 48)
            .background(Capsule().fill(Theme.gold.opacity(configuration.isPressed ? 0.8 : 1)))
            .opacity(enabled ? 1 : 0.4)
    }
}

struct SecondaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 17, weight: .medium))
            .foregroundStyle(Theme.text)
            .padding(.horizontal, 18)
            .frame(height: 48)
            .background(Capsule().fill(configuration.isPressed ? Theme.line : Theme.surface2))
            .overlay(Capsule().strokeBorder(Theme.lineStrong))
            .opacity(enabled ? 1 : 0.4)
    }
}

// MARK: - History

struct HistoryPane: View {
    @Bindable var model: DictationModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("HISTORY")
                .font(.system(size: 13, weight: .semibold))
                .tracking(0.8)
                .foregroundStyle(Theme.text3)
                .padding(.horizontal, 24)
                .padding(.top, 24)
                .padding(.bottom, 10)
            if model.history.isEmpty {
                Text("Your dictations will show up here.")
                    .font(.system(size: 16))
                    .foregroundStyle(Theme.text3)
                    .padding(.horizontal, 24)
                Spacer()
            } else {
                List {
                    ForEach(model.history) { item in
                        HistoryRow(item: item, selected: item.id == model.selectedID)
                            .contentShape(Rectangle())
                            .onTapGesture { model.select(item) }
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
    }
}

struct HistoryRow: View {
    let item: Dictation
    let selected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(item.date, format: .dateTime.weekday(.abbreviated).month(.abbreviated).day().hour().minute())
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(selected ? Theme.gold : Theme.text3)
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
