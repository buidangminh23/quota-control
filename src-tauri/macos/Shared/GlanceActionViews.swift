import SwiftUI

/// The popup's control and dialog colors (`src/styles/tokens.css`), for the buttons on the island and
/// the widgets and the confirmations they ask for.
struct GlanceControlPalette {
    let scheme: ColorScheme

    var control: Color { Color(glanceHex: scheme == .dark ? "#3a3a3c" : "#ffffff")! }
    var controlBorder: Color { scheme == .dark ? Color.white.opacity(0.12) : Color.black.opacity(0.14) }
    var menu: Color { Color(glanceHex: scheme == .dark ? "#2c2c2e" : "#f7f7f8")! }
    var menuBorder: Color { scheme == .dark ? Color.white.opacity(0.14) : Color.black.opacity(0.14) }
    var label: Color { scheme == .dark ? Color.white.opacity(0.9) : Color.black.opacity(0.88) }
    var secondary: Color { scheme == .dark ? Color.white.opacity(0.62) : Color.black.opacity(0.62) }
    var red: Color { Color(glanceHex: scheme == .dark ? "#ff453a" : "#ff3b30")! }
    var accent: Color { Color(glanceHex: scheme == .dark ? "#0a84ff" : "#007aff")! }
    var onAccent: Color { .white }
}

/// A button drawn like the popup's `.uc-button`: bordered, destructive (red words on a bordered
/// button) or prominent (filled with the accent), at full size or `.is-small`.
struct GlanceButtonStyle: ButtonStyle {
    enum Tone {
        case bordered
        case destructive
        case prominent
    }

    let tone: Tone
    var small = false
    var wide = false

    func makeBody(configuration: Configuration) -> some View {
        GlanceButtonBody(configuration: configuration, tone: tone, small: small, wide: wide)
    }
}

private struct GlanceButtonBody: View {
    let configuration: ButtonStyleConfiguration
    let tone: GlanceButtonStyle.Tone
    let small: Bool
    let wide: Bool
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let palette = GlanceControlPalette(scheme: colorScheme)
        let shape = RoundedRectangle(cornerRadius: 6, style: .continuous)
        configuration.label
            .font(.system(size: small ? 11.5 : 12, weight: .medium))
            .lineLimit(1)
            .padding(.horizontal, small ? 9 : 10)
            .frame(maxWidth: wide ? .infinity : nil)
            .frame(height: small ? 22 : 24)
            .foregroundStyle(ink(palette))
            .background(shape.fill(tone == .prominent ? palette.accent : palette.control))
            .overlay {
                if tone != .prominent {
                    shape.strokeBorder(palette.controlBorder, lineWidth: 0.5)
                }
            }
            .shadow(color: .black.opacity(tone == .prominent ? 0 : 0.08), radius: 0.5, y: 0.5)
            .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.5)
            .contentShape(shape)
    }

    private func ink(_ palette: GlanceControlPalette) -> Color {
        switch tone {
        case .bordered:
            return palette.label
        case .destructive:
            return palette.red
        case .prominent:
            return palette.onAccent
        }
    }
}

/// The popup's confirmation dialog (`DialogCard`), drawn in place of the button that asked for it:
/// the title, the words when there is room, then the two buttons side by side, cancel first.
struct GlanceConfirmCard<Actions: View>: View {
    let title: String
    let message: String?
    @ViewBuilder var actions: Actions
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let palette = GlanceControlPalette(scheme: colorScheme)
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        VStack(spacing: 8) {
            Text(title)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(palette.label)
            if let message, !message.isEmpty {
                Text(message)
                    .font(.system(size: 11.5))
                    .lineSpacing(2)
                    .foregroundStyle(palette.secondary)
            }
            HStack(spacing: 8) {
                actions
            }
            .padding(.top, 6)
        }
        .multilineTextAlignment(.center)
        .fixedSize(horizontal: false, vertical: true)
        .padding(EdgeInsets(top: 16, leading: 16, bottom: 14, trailing: 16))
        .frame(maxWidth: 272)
        .background(shape.fill(palette.menu))
        .overlay(shape.strokeBorder(palette.menuBorder, lineWidth: 0.5))
    }
}
