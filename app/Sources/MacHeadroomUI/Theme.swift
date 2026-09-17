import SwiftUI

/// Every colour the menu uses, in one place, so a look can be swapped whole.
public struct Theme: Identifiable {
    public var id: String { name }
    public let name: String
    public let note: String
    public let scheme: ColorScheme
    let background: Color
    let card: Color
    let cardStroke: Color?
    let text: Color
    let secondary: Color
    let tertiary: Color
    let accent: Color
    let accentText: Color
    let iconTile: Color
    let icon: Color
    let track: Color
    let warn: Color
    let danger: Color
    let good: Color
    let divider: Color
    let badge: Color
    let quietButton: Color
    let hover: Color
}

public extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(.sRGB,
                  red: Double((hex >> 16) & 0xFF) / 255,
                  green: Double((hex >> 8) & 0xFF) / 255,
                  blue: Double(hex & 0xFF) / 255,
                  opacity: opacity)
    }
}

public extension Theme {
    /// The first version: system greys over the menu's translucent material.
    static let system = Theme(
        name: "System", note: "Today's version", scheme: .light,
        background: .clear, card: Color.primary.opacity(0.045), cardStroke: nil,
        text: .primary, secondary: .secondary, tertiary: Color.secondary.opacity(0.7),
        accent: .accentColor, accentText: .white,
        iconTile: Color.accentColor.opacity(0.13), icon: .accentColor,
        track: Color.primary.opacity(0.08), warn: .orange, danger: .red, good: .green,
        divider: Color.primary.opacity(0.1), badge: Color.primary.opacity(0.06),
        quietButton: Color.primary.opacity(0.08), hover: Color.primary.opacity(0.06))

    static let clean = Theme(
        name: "Clean", note: "Bright white, Apple blue", scheme: .light,
        background: Color(hex: 0xFFFFFF), card: Color(hex: 0xF4F5F7), cardStroke: nil,
        text: Color(hex: 0x1D1D1F), secondary: Color(hex: 0x6E6E73), tertiary: Color(hex: 0xA1A1A6),
        accent: Color(hex: 0x0071E3), accentText: .white,
        iconTile: Color(hex: 0xE8F1FD), icon: Color(hex: 0x0071E3),
        track: Color(hex: 0xE8E8ED), warn: Color(hex: 0xFF9500), danger: Color(hex: 0xFF3B30), good: Color(hex: 0x34C759),
        divider: Color(hex: 0xE5E5EA), badge: Color(hex: 0xF2F2F7),
        quietButton: Color(hex: 0xF2F2F7), hover: Color(hex: 0xEBEBF0))

    static let paper = Theme(
        name: "Paper", note: "Warm cream, terracotta", scheme: .light,
        background: Color(hex: 0xFBF8F3), card: Color(hex: 0xF2ECE2), cardStroke: nil,
        text: Color(hex: 0x2B2621), secondary: Color(hex: 0x7A7066), tertiary: Color(hex: 0xA89E93),
        accent: Color(hex: 0xC4552E), accentText: .white,
        iconTile: Color(hex: 0xF4DFD2), icon: Color(hex: 0xB44C27),
        track: Color(hex: 0xE9E1D5), warn: Color(hex: 0xDD8A35), danger: Color(hex: 0xC8412B), good: Color(hex: 0x5E9B6A),
        divider: Color(hex: 0xE7DFD3), badge: Color(hex: 0xF2ECE2),
        quietButton: Color(hex: 0xEDE6DA), hover: Color(hex: 0xEAE3D7))

    static let mint = Theme(
        name: "Mint", note: "Fresh white, green for good", scheme: .light,
        background: Color(hex: 0xFFFFFF), card: Color(hex: 0xF1F8F4), cardStroke: nil,
        text: Color(hex: 0x16241D), secondary: Color(hex: 0x5D6F66), tertiary: Color(hex: 0x9AA8A1),
        accent: Color(hex: 0x12875A), accentText: .white,
        iconTile: Color(hex: 0xDCF0E5), icon: Color(hex: 0x12875A),
        track: Color(hex: 0xE3EDE8), warn: Color(hex: 0xE38B2C), danger: Color(hex: 0xD9453B), good: Color(hex: 0x16A06B),
        divider: Color(hex: 0xE2EBE6), badge: Color(hex: 0xEDF6F1),
        quietButton: Color(hex: 0xE9F2ED), hover: Color(hex: 0xE4EEE9))

    static let platinum = Theme(
        name: "Platinum", note: "A quiet nod to the classic Mac", scheme: .light,
        background: Color(hex: 0xEDEDED), card: Color(hex: 0xFBFBFB), cardStroke: Color(hex: 0xCFCFCF),
        text: Color(hex: 0x1A1A1A), secondary: Color(hex: 0x5F5F5F), tertiary: Color(hex: 0x999999),
        accent: Color(hex: 0x5856D6), accentText: .white,
        iconTile: Color(hex: 0xEAE9FB), icon: Color(hex: 0x4F4DC8),
        track: Color(hex: 0xD6D6D6), warn: Color(hex: 0xDB8527), danger: Color(hex: 0xD6453D), good: Color(hex: 0x3E9D5B),
        divider: Color(hex: 0xD2D2D2), badge: Color(hex: 0xFBFBFB),
        quietButton: Color(hex: 0xE2E2E2), hover: Color(hex: 0xF0F0F0))

    static let midnight = Theme(
        name: "Midnight", note: "Deep navy rather than grey", scheme: .dark,
        background: Color(hex: 0x15171C), card: Color(hex: 0x1F232B), cardStroke: nil,
        text: Color(hex: 0xF2F4F8), secondary: Color(hex: 0x9AA3B2), tertiary: Color(hex: 0x6B7384),
        accent: Color(hex: 0x6EA8FE), accentText: Color(hex: 0x0B1220),
        iconTile: Color(hex: 0x243044), icon: Color(hex: 0x8DB8FF),
        track: Color(hex: 0x2A2F39), warn: Color(hex: 0xFFB45C), danger: Color(hex: 0xFF7A70), good: Color(hex: 0x5BD69A),
        divider: Color(hex: 0x2A2F39), badge: Color(hex: 0x262B34),
        quietButton: Color(hex: 0x2A2F39), hover: Color(hex: 0x262B34))

    static let candidates: [Theme] = [.clean, .paper, .mint, .platinum, .midnight]

    static func named(_ name: String) -> Theme? {
        candidates.first { $0.name.caseInsensitiveCompare(name) == .orderedSame }
    }
}

private struct ThemeKey: EnvironmentKey {
    static let defaultValue = Theme.system
}

public extension EnvironmentValues {
    var theme: Theme {
        get { self[ThemeKey.self] }
        set { self[ThemeKey.self] = newValue }
    }
}

extension Level {
    func dot(_ t: Theme) -> Color {
        switch self {
        case .full: return t.danger
        case .low, .tight: return t.warn
        case .fine, .plenty: return t.good
        }
    }

    func bar(_ t: Theme) -> Color {
        switch self {
        case .full: return t.danger
        case .low, .tight: return t.warn
        case .fine, .plenty: return t.accent
        }
    }
}
