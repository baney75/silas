// The Silas Keyboard. iOS keyboards can't use the microphone, so the mic key asks the Silas app to
// listen: the first tap opens Silas by URL, which starts a session that keeps the microphone armed
// while the person is back in their app; later taps send start and stop through the App Group.
// The finished text comes back through the App Group and is typed with `textDocumentProxy`.
// The pasteboard is never touched. Needs Allow Full Access to share the App Group container.

import SilasCore
import UIKit

private enum Palette {
    static let bg = UIColor(rgb: 0x0A0A0B)
    static let key = UIColor(rgb: 0x18181B)
    static let keyDown = UIColor(rgb: 0x1F1F23)
    static let line = UIColor(rgb: 0x34333A)
    static let text = UIColor(rgb: 0xF3EEE4)
    static let text2 = UIColor(rgb: 0xBDB6A8)
    static let text3 = UIColor(rgb: 0x8E877A)
    static let gold = UIColor(rgb: 0xD6AE45)
    static let goldHi = UIColor(rgb: 0xF2D58A)
    static let red = UIColor(rgb: 0xEC7D65)
}

private extension UIColor {
    convenience init(rgb: UInt32) {
        self.init(
            red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }
}

final class KeyboardViewController: UIInputViewController {
    /// Text older than this is not typed: it was meant for a field the person has since left.
    private static let pendingMaxAge: TimeInterval = 600

    private lazy var handoff = Handoff()
    private var phase: Handoff.Phase = .off
    private var observer: DarwinNotify.Observer?
    private var refreshTimer: Timer?
    private var deleteTimer: Timer?
    /// The last character this keyboard typed, while the cursor hasn't moved since.
    private var typedTail: Character?

    private let status = UILabel()
    private let mic = UIButton(type: .custom)
    private let cancel = UIButton(type: .system)
    private let last = UIButton(type: .system)
    private let globe = UIButton(type: .system)
    private let space = UIButton(type: .system)
    private let delete = UIButton(type: .system)
    private let ret = UIButton(type: .system)

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.bg
        build()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        observer = DarwinNotify.Observer(Handoff.changed) { [weak self] in self?.refresh() }
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.refresh() }
        refresh()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        observer = nil
        refreshTimer?.invalidate()
        refreshTimer = nil
        stopDeleteRepeat()
    }

    override func viewWillLayoutSubviews() {
        super.viewWillLayoutSubviews()
        globe.isHidden = !needsInputModeSwitchKey
    }

    // MARK: State

    /// Read the session and type any text waiting for this field.
    private func refresh() {
        guard hasFullAccess, let handoff else {
            phase = .off
            render()
            return
        }
        if let pending = handoff.takePending(), Date().timeIntervalSince(pending.date) < Self.pendingMaxAge {
            type(pending.text)
        }
        phase = handoff.session().phase
        render(preview: handoff.session().preview)
    }

    private func render(preview: String? = nil) {
        let micSymbol: String
        switch phase {
        case .listening:
            status.text = preview ?? "Listening… tap to write"
            status.textColor = preview == nil ? Palette.text2 : Palette.text
            micSymbol = "stop.fill"
            mic.backgroundColor = Palette.gold
            mic.tintColor = Palette.bg
        case .writing:
            status.text = "Writing…"
            status.textColor = Palette.text2
            micSymbol = "ellipsis"
            mic.backgroundColor = Palette.key
            mic.tintColor = Palette.gold
        case .armed:
            status.text = "Tap to speak"
            status.textColor = Palette.text2
            micSymbol = "mic.fill"
            mic.backgroundColor = Palette.key
            mic.tintColor = Palette.gold
        case .off:
            status.text = hasFullAccess && handoff != nil
                ? "Tap the mic: Silas opens and listens. Come back and it types."
                : "Turn on Allow Full Access for Silas in Settings › General › Keyboard › Keyboards."
            status.textColor = Palette.text3
            micSymbol = "mic.fill"
            mic.backgroundColor = Palette.key
            mic.tintColor = Palette.gold
        }
        mic.setImage(UIImage(systemName: micSymbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 28, weight: .semibold)), for: .normal)
        mic.accessibilityLabel = phase == .listening ? "Stop and write" : "Dictate"
        mic.accessibilityValue = phase.rawValue
        cancel.isHidden = phase != .listening
        last.isEnabled = handoff?.last != nil
    }

    // MARK: Keys

    @objc private func micTapped() {
        guard hasFullAccess, let handoff else {
            render()
            return
        }
        switch handoff.session().phase {
        case .off:
            status.text = "Opening Silas…"
            if !openContainingApp(Handoff.dictateURL) {
                status.text = "Open Silas, then come back here."
            }
        case .armed:
            handoff.send(.start)
        case .listening:
            handoff.send(.stop)
        case .writing:
            break
        }
    }

    @objc private func cancelTapped() {
        handoff?.send(.cancel)
    }

    @objc private func lastTapped() {
        guard let text = handoff?.last?.text, !text.isEmpty else { return }
        type(text)
    }

    @objc private func spaceTapped() {
        textDocumentProxy.insertText(" ")
        typedTail = " "
    }

    @objc private func returnTapped() {
        textDocumentProxy.insertText("\n")
        typedTail = "\n"
    }

    @objc private func deleteDown() {
        deleteOne()
        deleteTimer = Timer.scheduledTimer(withTimeInterval: 0.45, repeats: false) { [weak self] _ in
            self?.deleteTimer = Timer.scheduledTimer(withTimeInterval: 0.08, repeats: true) { [weak self] _ in
                self?.deleteOne()
            }
        }
    }

    private func deleteOne() {
        textDocumentProxy.deleteBackward()
        typedTail = nil
    }

    @objc private func deleteUp() { stopDeleteRepeat() }

    private func stopDeleteRepeat() {
        deleteTimer?.invalidate()
        deleteTimer = nil
    }

    /// Type text, with a space before it when it follows a word.
    private func type(_ text: String) {
        guard !text.isEmpty else { return }
        var out = text
        if let before = characterBeforeCursor(), !before.isWhitespace,
            let first = out.first, !first.isPunctuation
        {
            out = " " + out
        }
        textDocumentProxy.insertText(out)
        typedTail = out.last
    }

    /// The host's context lags behind our own inserts (it can still read empty a second after),
    /// so fall back to the last character this keyboard typed until the cursor moves.
    private func characterBeforeCursor() -> Character? {
        if let context = textDocumentProxy.documentContextBeforeInput, let last = context.last {
            return last
        }
        return typedTail
    }

    override func selectionDidChange(_ textInput: UITextInput?) {
        super.selectionDidChange(textInput)
        typedTail = nil
    }

    /// Keyboards can't call `UIApplication.open`; the app object in the responder chain can.
    /// This is how keyboards that dictate through their app (Wispr Flow, Aqua) open it.
    private func openContainingApp(_ url: URL) -> Bool {
        let selector = sel_registerName("openURL:options:completionHandler:")
        var responder: UIResponder? = self
        while let r = responder {
            if r.responds(to: selector), let imp = r.method(for: selector) {
                typealias Open = @convention(c) (AnyObject, Selector, NSURL, NSDictionary, AnyObject?) -> Void
                unsafeBitCast(imp, to: Open.self)(r, selector, url as NSURL, NSDictionary(), nil)
                return true
            }
            responder = r.next
        }
        return false
    }

    // MARK: Layout

    private func build() {
        let height = view.heightAnchor.constraint(equalToConstant: 250)
        height.priority = .defaultHigh
        height.isActive = true

        status.font = .systemFont(ofSize: 15)
        status.numberOfLines = 2
        status.textAlignment = .center
        status.lineBreakMode = .byTruncatingHead
        status.accessibilityIdentifier = "silas.kb.status"

        mic.layer.cornerRadius = 38
        mic.layer.borderWidth = 2
        mic.layer.borderColor = Palette.gold.withAlphaComponent(0.42).cgColor
        mic.accessibilityIdentifier = "silas.kb.mic"
        mic.addTarget(self, action: #selector(micTapped), for: .touchUpInside)

        style(cancel, symbol: "xmark", label: "Cancel", id: "silas.kb.cancel")
        cancel.addTarget(self, action: #selector(cancelTapped), for: .touchUpInside)
        style(last, symbol: "arrow.uturn.backward", label: "Type last again", id: "silas.kb.last")
        last.addTarget(self, action: #selector(lastTapped), for: .touchUpInside)
        style(globe, symbol: "globe", label: "Next keyboard", id: "silas.kb.globe")
        globe.addTarget(self, action: #selector(handleInputModeList(from:with:)), for: .allTouchEvents)
        style(space, title: "space", label: "Space", id: "silas.kb.space")
        space.addTarget(self, action: #selector(spaceTapped), for: .touchUpInside)
        style(delete, symbol: "delete.left", label: "Delete", id: "silas.kb.delete")
        delete.addTarget(self, action: #selector(deleteDown), for: .touchDown)
        delete.addTarget(self, action: #selector(deleteUp), for: [.touchUpInside, .touchUpOutside, .touchCancel])
        style(ret, symbol: "return", label: "Return", id: "silas.kb.return")
        ret.addTarget(self, action: #selector(returnTapped), for: .touchUpInside)

        let middle = UIView()
        for v in [last, mic, cancel] {
            v.translatesAutoresizingMaskIntoConstraints = false
            middle.addSubview(v)
        }
        NSLayoutConstraint.activate([
            mic.centerXAnchor.constraint(equalTo: middle.centerXAnchor),
            mic.centerYAnchor.constraint(equalTo: middle.centerYAnchor),
            mic.widthAnchor.constraint(equalToConstant: 76),
            mic.heightAnchor.constraint(equalToConstant: 76),
            last.leadingAnchor.constraint(equalTo: middle.leadingAnchor),
            last.centerYAnchor.constraint(equalTo: middle.centerYAnchor),
            last.widthAnchor.constraint(equalToConstant: 64),
            last.heightAnchor.constraint(equalToConstant: 44),
            cancel.trailingAnchor.constraint(equalTo: middle.trailingAnchor),
            cancel.centerYAnchor.constraint(equalTo: middle.centerYAnchor),
            cancel.widthAnchor.constraint(equalToConstant: 64),
            cancel.heightAnchor.constraint(equalToConstant: 44),
            middle.heightAnchor.constraint(equalToConstant: 92),
        ])

        let bottom = UIStackView(arrangedSubviews: [globe, space, delete, ret])
        bottom.spacing = 6
        bottom.distribution = .fill
        for v in [globe, delete, ret] {
            v.widthAnchor.constraint(equalToConstant: 56).isActive = true
        }
        bottom.heightAnchor.constraint(equalToConstant: 46).isActive = true

        let column = UIStackView(arrangedSubviews: [status, middle, bottom])
        column.axis = .vertical
        column.spacing = 10
        column.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 8),
            column.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -8),
            column.topAnchor.constraint(equalTo: view.topAnchor, constant: 10),
            column.bottomAnchor.constraint(lessThanOrEqualTo: view.bottomAnchor, constant: -6),
            status.heightAnchor.constraint(greaterThanOrEqualToConstant: 40),
        ])
        render()
    }

    private func style(_ b: UIButton, symbol: String? = nil, title: String? = nil, label: String, id: String) {
        var config = UIButton.Configuration.filled()
        config.baseBackgroundColor = Palette.key
        config.baseForegroundColor = Palette.text
        config.cornerStyle = .medium
        if let symbol {
            config.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .medium))
        }
        if let title {
            var t = AttributedString(title)
            t.font = .systemFont(ofSize: 16)
            config.attributedTitle = t
        }
        b.configuration = config
        b.accessibilityLabel = label
        b.accessibilityIdentifier = id
    }
}
