import SwiftUI
import WidgetKit

/// One reset tracker in brief, as much of it as `style` has room for.
struct ResetsSummaryBody: View {
    let resets: GlanceResets
    let units: GlanceUnits
    let now: Date
    let style: ResetsSummaryStyle

    var body: some View {
        let upcoming = resets.upcoming(at: now)
        switch style {
        case .narrow:
            VStack(alignment: .leading, spacing: 6) {
                ResetsHeader(resets: resets)
                if let upcoming {
                    AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13)
                } else {
                    ChanceHero(resets: resets)
                }
                Spacer(minLength: 0)
                if let latest = resets.latest {
                    latest.since.live(now: now, units: units)
                        .font(.glance(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .band:
            VStack(alignment: .leading, spacing: 7) {
                ResetsHeader(resets: resets, showsSource: true)
                HStack(alignment: .top, spacing: WidgetScale.columnSpacing + 4) {
                    VStack(alignment: .leading, spacing: 7) {
                        if let upcoming {
                            AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13.5)
                        }
                        if let latest = resets.latest {
                            LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now, valueSize: upcoming == nil ? 13.5 : WidgetScale.caption, showsWhen: upcoming == nil)
                                .foregroundStyle(upcoming == nil ? Color.primary : Color.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    ChanceBars(resets: resets, spacing: 5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        case .column:
            VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
                ResetsHeader(resets: resets)
                if let upcoming {
                    AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13.5)
                }
                if let latest = resets.latest {
                    LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now)
                }
                ChanceBars(resets: resets, spacing: 5)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .pair:
            VStack(alignment: .leading, spacing: 4) {
                ResetsHeader(resets: resets)
                if let upcoming {
                    AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13)
                } else if let day = resets.chance(days: 1) ?? resets.forecast.first {
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Text("\(day.percent)%")
                            .font(.glance(size: 20, weight: .bold))
                            .monospacedDigit()
                            .foregroundStyle(resets.tint)
                            .fixedSize()
                        Text(day.label)
                            .font(.glance(size: WidgetScale.caption, weight: .medium))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }
                    let longer = resets.forecast.filter { $0.days != day.days }
                    if !longer.isEmpty {
                        Text(longer.map { "\($0.label) \($0.percent)%" }.joined(separator: " · "))
                            .font(.glance(size: WidgetScale.footnote))
                            .monospacedDigit()
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.8)
                    }
                }
                Spacer(minLength: 0)
                if upcoming == nil, let latest = resets.latest {
                    latest.since.live(now: now, units: units)
                        .font(.glance(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .line:
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    ProviderMark(mark: resets.mark)
                        .foregroundStyle(resets.markTint)
                        .frame(width: 10, height: 10)
                        .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
                        .accessibilityLabel(resets.title)
                    if let upcoming {
                        Text(upcoming.title)
                            .font(.glance(size: WidgetScale.caption, weight: .semibold))
                            .foregroundStyle(upcoming.toneColor)
                            .lineLimit(1)
                    } else if let day = resets.chance(days: 1) ?? resets.forecast.first {
                        Text("\(day.percent)%")
                            .font(.glance(size: 15, weight: .bold))
                            .monospacedDigit()
                            .foregroundStyle(resets.tint)
                            .fixedSize()
                        Text(day.label)
                            .font(.glance(size: WidgetScale.caption))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.8)
                    } else {
                        Text(resets.title)
                            .font(.glance(size: WidgetScale.caption, weight: .semibold))
                            .lineLimit(1)
                    }
                }
                if let upcoming {
                    upcoming.liveValue(now: now, units: units)
                        .font(.glance(size: 13, weight: .semibold))
                        .monospacedDigit()
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                } else if let latest = resets.latest {
                    latest.since.live(now: now, units: units)
                        .font(.glance(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .accessibilityElement(children: .combine)
        }
    }
}

/// A tracker of a widget showing both that has nothing to draw: its name and the line saying why
/// (on its way, could not load, or off), in the notice color once it could not load.
struct ResetsMissingPart: View {
    let part: GlanceResetsShown
    let style: ResetsSummaryStyle
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        VStack(alignment: .leading, spacing: style == .line ? 2 : 5) {
            Text(part.title)
                .font(.glance(size: style == .line ? WidgetScale.caption : WidgetScale.title, weight: .semibold))
                .lineLimit(1)
            Text(part.message)
                .font(.glance(size: WidgetScale.caption))
                .foregroundStyle(part.failed ? GlanceResetPalette(scheme: colorScheme).noticeText : Color.secondary)
                .lineLimit(style == .line ? 2 : 4)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

/// Both trackers at once, a widget's first page while it shows both: each tracker in brief, the
/// Codex one first, side by side on a medium or extra large widget, one above the other on a small
/// or large one.
struct ResetsBothPage: View {
    let document: GlanceDocument
    /// The two trackers, `shownResets(.both)`.
    let shown: [GlanceResetsShown]
    let family: WidgetFamily
    let now: Date

    private var style: ResetsSummaryStyle {
        switch family {
        case .systemSmall: return .line
        case .systemMedium: return .pair
        case .systemLarge: return .band
        default: return .column
        }
    }

    var body: some View {
        if style == .line || style == .band {
            VStack(alignment: .leading, spacing: style == .line ? 7 : WidgetScale.blockSpacing) {
                parts
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                parts
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private var parts: some View {
        ForEach(Array(shown.enumerated()), id: \.offset) { index, part in
            if index > 0 {
                Divider()
            }
            Group {
                if let resets = part.resets {
                    ResetsSummaryBody(resets: resets, units: document.labels.units, now: now, style: style)
                } else {
                    ResetsMissingPart(part: part, style: style)
                }
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
    }
}

/// Both trackers' reset calendars at once, a Reset Calendar widget's first page while it shows both:
/// each tracker's latest weeks under its name, the Codex one first, side by side on a medium or extra
/// large widget, one above the other on a large one, with the legend under them where there is room.
/// A tracker without its calendar shows itself in brief, or why it has nothing.
struct ResetCalendarsBothPage: View {
    let document: GlanceDocument
    /// The two trackers, `shownResets(.both)`.
    let shown: [GlanceResetsShown]
    let family: WidgetFamily
    let now: Date
    /// The page's size, the widget without its page buttons and footer.
    let size: CGSize

    /// The heading above each calendar and the gap under it.
    private static let heading: CGFloat = 18
    /// The legend's line and the gap above it.
    private static let legend: CGFloat = 20
    /// The month line above the grid and the gaps between its seven rows (`calendarGrid`).
    private static let gridFrame: CGFloat = 14 + 6 * 3
    /// The weekday names before the grid's rows.
    private static let weekdays: CGFloat = 22

    private var stacked: Bool { family == .systemLarge }
    private var legendShown: Bool { family != .systemMedium }

    var body: some View {
        let divider = WidgetScale.columnSpacing * 2 + 1
        let count = CGFloat(max(shown.count, 1))
        let width = stacked ? size.width : max(1, (size.width - (count - 1) * divider) / count)
        let room = size.height - (legendShown ? Self.legend : 0)
        let height = stacked ? max(1, (room - (count - 1) * (WidgetScale.blockSpacing * 2 + 1)) / count) : room
        VStack(alignment: .leading, spacing: 6) {
            if stacked {
                VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
                    parts(width: width, height: height)
                }
            } else {
                HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                    parts(width: width, height: height)
                }
            }
            if legendShown, let calendar = shown.lazy.compactMap({ $0.resets?.calendar }).first {
                GlanceResetElementView(element: .legend(calendar.legend, GlanceResetLegend.Item.allCases), availableWidth: size.width)
            }
        }
        .frame(width: size.width, height: size.height, alignment: .topLeading)
    }

    private func parts(width: CGFloat, height: CGFloat) -> some View {
        ForEach(Array(shown.enumerated()), id: \.offset) { index, part in
            if index > 0 {
                Divider()
            }
            VStack(alignment: .leading, spacing: 4) {
                if let resets = part.resets {
                    ResetsHeader(resets: resets)
                    if let calendar = resets.calendar {
                        calendarGrid(calendar, width: width, height: height - Self.heading)
                    } else {
                        ResetsSummaryBody(resets: resets, units: document.labels.units, now: now, style: .line)
                    }
                } else {
                    ResetsMissingPart(part: part, style: family == .systemMedium ? .line : .pair)
                }
            }
            .frame(width: width, alignment: .topLeading)
        }
    }

    /// The tracker's latest weeks that fit `width` with squares as big as `height` allows, newest on
    /// the right as in the Reset tab: the Reset tab's grid where its weekday names fit beside the
    /// rows, the compact one on a medium widget, where they do not.
    @ViewBuilder
    private func calendarGrid(_ calendar: GlanceResetCalendar, width: CGFloat, height: CGFloat) -> some View {
        let total = calendar.weekRows().count
        if family == .systemMedium {
            let side = max(3, min(9, ((height - CompactResetCalendar.frame) / 7).rounded(.down)))
            let shown = max(1, min(total, Int((width + CompactResetCalendar.gap) / (side + CompactResetCalendar.gap))))
            CompactResetCalendar(calendar: calendar, weeks: (total - shown)..<total, side: side)
        } else {
            let side = max(4, min(14, ((height - Self.gridFrame) / 7).rounded(.down)))
            let shown = max(1, min(total, Int((width - Self.weekdays) / (side + 2))))
            GlanceResetElementView(element: .calendar(calendar, (total - shown)..<total, 0..<7), availableWidth: Self.weekdays + CGFloat(shown) * (side + 2))
        }
    }
}

/// A reset calendar small enough for half a medium widget: the latest weeks as columns of seven
/// squares, Monday on top as in the Reset tab, under the months they start, today outlined. Its
/// weekday names, too tall beside rows this small, are left out.
struct CompactResetCalendar: View {
    let calendar: GlanceResetCalendar
    let weeks: Range<Int>
    let side: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    static let gap: CGFloat = 2
    /// The month line above the squares and the gaps between their seven rows.
    static let frame: CGFloat = 10 + gap + 6 * gap

    var body: some View {
        let rows = calendar.weekRows()
        let palette = GlanceResetPalette(scheme: colorScheme)
        VStack(alignment: .leading, spacing: Self.gap) {
            HStack(spacing: Self.gap) {
                ForEach(Array(weeks), id: \.self) { week in
                    Text(calendar.months.first(where: { $0.week == week })?.label ?? "")
                        .font(.glance(size: 8))
                        .foregroundStyle(.secondary)
                        .fixedSize()
                        .frame(width: side, height: 10, alignment: .bottomLeading)
                }
            }
            HStack(alignment: .top, spacing: Self.gap) {
                ForEach(Array(weeks), id: \.self) { week in
                    VStack(spacing: Self.gap) {
                        ForEach(0..<7, id: \.self) { day in
                            let value = week < rows.count && day < rows[week].count ? rows[week][day] : .future
                            RoundedRectangle(cornerRadius: 1.5)
                                .fill(value == .regular ? palette.blue : value == .banked ? GlanceResetPalette.orange : Color.primary.opacity(value == .future ? 0.035 : 0.1))
                                .overlay(RoundedRectangle(cornerRadius: 1.5).strokeBorder(Color.primary.opacity(week * 7 + day == calendar.today ? 1 : 0), lineWidth: 1))
                                .frame(width: side, height: side)
                        }
                    }
                }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(calendar.title)
    }
}
