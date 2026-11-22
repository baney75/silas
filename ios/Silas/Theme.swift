// Silas is diamond black and gold, dark only (brand/README.md, src/styles.css). Gold is the one
// accent; red is for errors and the recording dot only; there is no green anywhere.

import SwiftUI

extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}

enum Theme {
    static let bg = Color(hex: 0x0A0A0B)
    static let side = Color(hex: 0x050506)
    static let surface = Color(hex: 0x101012)
    static let surface2 = Color(hex: 0x18181B)
    static let line = Color(hex: 0x1F1F23)
    static let lineStrong = Color(hex: 0x34333A)
    static let text = Color(hex: 0xF3EEE4)
    static let text2 = Color(hex: 0xBDB6A8)
    static let text3 = Color(hex: 0x8E877A)
    static let gold = Color(hex: 0xD6AE45)
    /// Gold text and links on black.
    static let goldInk = Color(hex: 0xE8C66C)
    /// Focus ring and the top edge of a pressed key.
    static let goldHi = Color(hex: 0xF2D58A)
    static let goldSoft = Color(hex: 0xD6AE45, opacity: 0.12)
    static let goldLine = Color(hex: 0xD6AE45, opacity: 0.42)
    static let hairline = Color(hex: 0xD6AE45, opacity: 0.30)
    /// Ready, saved and allowed: gold, never green.
    static let ok = Color(hex: 0xE8C66C)
    static let red = Color(hex: 0xEC7D65)
    static let primaryFg = Color(hex: 0x0A0A0B)
    /// The 1 px light line along the top edge of every card.
    static let facet = Color.white.opacity(0.06)

    static func serif(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .serif)
    }
}

/// A card: surface fill, hairline border and the facet line on top.
struct Card: ViewModifier {
    var radius: CGFloat = 16

    func body(content: Content) -> some View {
        content
            .background(RoundedRectangle(cornerRadius: radius, style: .continuous).fill(Theme.surface))
            .overlay(RoundedRectangle(cornerRadius: radius, style: .continuous).strokeBorder(Theme.line))
            .overlay(alignment: .top) {
                Rectangle().fill(Theme.facet).frame(height: 1).padding(.horizontal, radius)
            }
    }
}

extension View {
    func card(radius: CGFloat = 16) -> some View { modifier(Card(radius: radius)) }

    /// Small caps label above a section, like the desktop's.
    func sectionLabel() -> some View {
        self.font(.system(size: 13, weight: .semibold))
            .tracking(0.8)
            .foregroundStyle(Theme.text3)
    }
}

struct PrimaryButton: ButtonStyle {
    @Environment(\.isEnabled) private var enabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(Theme.primaryFg)
            .padding(.horizontal, 20)
            .frame(minHeight: 48)
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
            .frame(minHeight: 48)
            .background(Capsule().fill(configuration.isPressed ? Theme.line : Theme.surface2))
            .overlay(Capsule().strokeBorder(Theme.lineStrong))
            .opacity(enabled ? 1 : 0.4)
    }
}
