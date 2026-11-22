// Silas dark palette, from brand/README.md and the desktop's dark-mode tokens in src/styles.css.

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
    static let bg = Color(hex: 0x14161A)
    static let side = Color(hex: 0x101215)
    static let surface = Color(hex: 0x1B1E23)
    static let surface2 = Color(hex: 0x23272D)
    static let line = Color(hex: 0x2B2F36)
    static let lineStrong = Color(hex: 0x3A3F47)
    static let text = Color(hex: 0xEDE6D8)
    static let text2 = Color(hex: 0xBCB4A5)
    static let text3 = Color(hex: 0x918A7D)
    static let gold = Color(hex: 0xD4AF6A)
    static let goldSoft = Color(hex: 0xD4AF6A, opacity: 0.13)
    static let goldLine = Color(hex: 0xD4AF6A, opacity: 0.42)
    static let green = Color(hex: 0x86B892)
    static let red = Color(hex: 0xE58A73)
    static let redSoft = Color(hex: 0xE58A73, opacity: 0.12)
    static let primaryFg = Color(hex: 0x14161A)

    static func serif(_ size: CGFloat, weight: Font.Weight = .semibold) -> Font {
        .system(size: size, weight: weight, design: .serif)
    }
}
