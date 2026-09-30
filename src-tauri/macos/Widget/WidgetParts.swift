import SwiftUI
import WidgetKit

/// The type sizes and spacings every widget shares, so the same kind of text reads the same size in
/// every style and family.
enum WidgetScale {
    /// An account or section heading.
    static let title: CGFloat = 11
    /// The mark beside a heading.
    static let mark: CGFloat = 12
    /// A metric's name.
    static let label: CGFloat = 10.5
    /// A reading: a headline, a percentage, a countdown.
    static let value: CGFloat = 11.5
    /// A reset countdown, a caption, a note.
    static let caption: CGFloat = 9.5
    /// The update time and other small print.
    static let footnote: CGFloat = 9
    /// The one big number of a small widget.
    static let hero: CGFloat = 28
    static let columnSpacing: CGFloat = 14
    static let blockSpacing: CGFloat = 9
    /// The height `UpdatedFooter` takes with the space above it.
    static let footerHeight: CGFloat = 16
    /// The height of a one-line `MoreLine`.
    static let moreHeight: CGFloat = 14
}

extension GlanceUpcomingReset {
    /// The announced reset's reading (its live countdown or fixed value) at `now`.
    func liveValue(now: Date, units: GlanceUnits) -> Text {
        if let countdown { return countdown.live(now: now, units: units) }
        return Text(value ?? "")
    }

    /// The caption under the reading, the after-caption once the countdown has passed.
    func currentCaption(now: Date) -> String {
        if let countdown, countdown.passed(now) { return captionAfter ?? caption }
        return caption
    }

    /// The tone's color: green for a reset confirmed, orange for one only likely.
    var toneColor: Color {
        switch tone {
        case .positive: return Color(red: 0.13, green: 0.72, blue: 0.4)
        case .notice: return .orange
        }
    }
}

private struct GlanceColorlessKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Whether the widget is drawn without its colors: dimmed on the desktop while a window is in
    /// front, or in the monochrome widget style, where macOS draws it its own way.
    var glanceColorless: Bool {
        get { self[GlanceColorlessKey.self] }
        set { self[GlanceColorlessKey.self] = newValue }
    }
}

/// The mark before a reading where a widget is drawn without its colors: an outlined triangle for a
/// limit in the warning color, a filled one for a limit in the critical color (running out, used
/// up), which in full color the meter's color says, as the popup's does. In full color it draws
/// nothing.
struct ColorlessSeverityMark: View {
    let severity: GlanceSeverity
    var size: CGFloat = WidgetScale.value
    @Environment(\.glanceColorless) private var colorless

    var body: some View {
        if colorless, severity == .warning || severity == .critical {
            Image(systemName: severity == .critical ? "exclamationmark.triangle.fill" : "exclamationmark.triangle")
                .font(.system(size: size * 0.85, weight: .semibold))
                .foregroundStyle(.primary)
                .accessibilityHidden(true)
        }
    }
}

/// A capsule meter in any color, for readings that are not a limit's pace (reset chances).
struct TintMeter: View {
    let fraction: Double
    let color: Color
    var height: CGFloat = 5

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let clamped = min(max(fraction, 0), 1)
            let fill = clamped > 0 ? max(width * clamped, height) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.12))
                if fill > 0 {
                    Capsule().fill(color).frame(width: min(fill, width))
                }
            }
        }
        .frame(height: height)
    }
}

/// A section's heading: an optional mark in its color, the title, and something on the right.
struct WidgetHeading<Trailing: View>: View {
    var mark: GlanceMark?
    var markColor: Color = .primary
    var showsMark = true
    let title: String
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            if showsMark {
                ProviderMark(mark: mark)
                    .foregroundStyle(markColor)
                    .frame(width: WidgetScale.mark, height: WidgetScale.mark)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 2 }
            }
            Text(title)
                .font(.system(size: WidgetScale.title, weight: .semibold))
                .lineLimit(1)
                .layoutPriority(1)
            Spacer(minLength: 4)
            trailing()
        }
    }
}

extension WidgetHeading where Trailing == EmptyView {
    init(mark: GlanceMark?, markColor: Color = .primary, showsMark: Bool = true, title: String) {
        self.init(mark: mark, markColor: markColor, showsMark: showsMark, title: title) { EmptyView() }
    }
}

/// A small gray label over a group of readings.
struct SectionLabel: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: WidgetScale.caption, weight: .semibold))
            .foregroundStyle(.secondary)
            .lineLimit(1)
    }
}

/// `+17`: how many of an account's metrics a layout left out.
struct HiddenCount: View {
    let count: Int

    var body: some View {
        if count > 0 {
            Text("+\(count)")
                .font(.system(size: WidgetScale.footnote, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(Capsule().fill(Color.primary.opacity(0.08)))
                .fixedSize()
        }
    }
}

/// The line under a layout saying what it had to leave out.
struct MoreLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: WidgetScale.caption, weight: .medium))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.tail)
    }
}

enum MoreText {
    /// `+2 tài khoản khác: Copilot, Antigravity`: accounts left out, by name.
    static func accounts(_ hidden: [GlanceProvider], labels: GlanceLabels) -> String {
        "+\(hidden.count) \(labels.more): \(hidden.map(\.name).joined(separator: ", "))"
    }

    /// `+12 chỉ số khác · Antigravity 11 · Copilot 1`: metrics left out, counted per account.
    static func metrics(_ hidden: [(GlanceProvider, Int)], document: GlanceDocument) -> String {
        let total = hidden.reduce(0) { $0 + $1.1 }
        let word = document.isVietnamese ? "chỉ số khác" : "more"
        let parts = hidden.filter { $0.1 > 0 }.map { "\($0.0.name) \($0.1)" }
        return (["+\(total) \(word)"] + parts).joined(separator: " · ")
    }
}

/// When the readings were taken, in orange with a mark once the app has stopped refreshing them.
struct UpdatedFooter: View {
    let document: GlanceDocument
    let now: Date

    var body: some View {
        let stale = now.timeIntervalSince(document.generatedAt) > GlanceStaleness.after
        HStack(spacing: 3) {
            if stale {
                Image(systemName: "exclamationmark.circle")
            }
            Text("\(document.labels.updated) \(GlanceFormat.time(document.generatedAt, locale: document.resolvedLocale, hour12: document.hour12))")
        }
        .font(.system(size: WidgetScale.footnote))
        .foregroundStyle(stale ? Color.orange : Color.secondary)
        .lineLimit(1)
        .frame(height: WidgetScale.footerHeight - 4, alignment: .bottomLeading)
        .padding(.top, 4)
    }
}

/// Readings older than this come from an app that stopped refreshing. The timeline adds an entry at
/// that moment, so the marker appears on time even when nothing reloads the widget.
enum GlanceStaleness {
    static let after: TimeInterval = 20 * 60
}

/// A centered symbol and sentence for a widget with nothing to draw; a sentence saying something
/// could not load reads in the notice color, as the popup says it.
struct WidgetMessage: View {
    let text: String
    var symbol = "gauge.with.dots.needle.33percent"
    var failed = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: symbol)
                .font(.system(size: 22, weight: .medium))
                .foregroundStyle(.secondary)
            Text(text)
                .font(.system(size: 11))
                .multilineTextAlignment(.center)
                .foregroundStyle(failed ? GlanceResetPalette(scheme: colorScheme).noticeText : Color.secondary)
                .lineLimit(4)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// When a limit comes back, for a list of upcoming resets: the clock time with its day, as the
/// popup names the moment a limit comes back (`13:05 · hôm nay`, `13:05 · T2 05/10`).
enum ResetClock {
    static func text(_ date: Date, now: Date, document: GlanceDocument) -> String {
        document.dayLabel(date, now: now)
    }

    /// The clock time alone, for a row without room for its day: only for a moment still today,
    /// which the time names without doubt.
    static func timeToday(_ date: Date, now: Date, document: GlanceDocument) -> String? {
        guard GlanceDays.between(now, date) <= 0 else { return nil }
        guard let days = document.labels.days else { return GlanceFormat.time(date, locale: document.resolvedLocale, hour12: document.hour12) }
        return GlanceDays.format(date, pattern: days.time, locale: document.resolvedLocale, calendar: .current)
    }
}
