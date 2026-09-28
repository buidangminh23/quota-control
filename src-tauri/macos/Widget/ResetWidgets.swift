import SwiftUI
import WidgetKit

// MARK: Reset tracker parts

/// The reset tracker's heading: the Codex mark, the title and, where there is room, the source.
struct ResetsHeader: View {
    let resets: GlanceResets
    var title: String?
    var showsSource = false

    var body: some View {
        WidgetHeading(mark: resets.mark, markColor: resets.markTint, title: title ?? resets.title) {
            if showsSource {
                Text(resets.source)
                    .font(.system(size: WidgetScale.footnote))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
    }
}

/// Why the tracker's readings may be old, in orange.
struct ResetsStaleLine: View {
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 3) {
            Image(systemName: "exclamationmark.circle")
            Text(text).lineLimit(1)
        }
        .font(.system(size: WidgetScale.caption))
        .foregroundStyle(Color.orange)
    }
}

/// The announced reset: its title in the tone's color, the live countdown (or fixed value), the
/// caption and, where asked, the note.
struct AnnouncedResetBlock: View {
    let upcoming: GlanceUpcomingReset
    let units: GlanceUnits
    let now: Date
    var valueSize: CGFloat = 15
    var captionLines = 2
    var showsNote = false

    private var title: String {
        if let percent = upcoming.chancePercent { return "\(upcoming.title) · \(percent)%" }
        return upcoming.title
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 4) {
                Circle().fill(upcoming.toneColor).frame(width: 6, height: 6)
                Text(title)
                    .font(.system(size: WidgetScale.caption, weight: .semibold))
                    .foregroundStyle(upcoming.toneColor)
                    .lineLimit(1)
            }
            upcoming.liveValue(now: now, units: units)
                .font(.system(size: valueSize, weight: .semibold))
                .monospacedDigit()
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            Text(upcoming.currentCaption(now: now))
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(.secondary)
                .lineLimit(captionLines)
                .fixedSize(horizontal: false, vertical: true)
            if showsNote, let note = upcoming.note {
                Text(note)
                    .font(.system(size: WidgetScale.footnote))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
        }
    }
}

/// The last reset: how long ago, live, and when it was and of which kind.
struct LatestResetBlock: View {
    let latest: GlanceLatestReset
    let tint: Color
    let units: GlanceUnits
    let now: Date
    var valueSize: CGFloat = WidgetScale.value
    var showsWhen = true

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            latest.since.live(now: now, units: units)
                .font(.system(size: valueSize, weight: .semibold))
                .monospacedDigit()
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
            if showsWhen {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .fill(latest.kind == "banked" ? Color.orange : tint)
                        .frame(width: 7, height: 7)
                    Text("\(latest.label): \(latest.when) · \(latest.kindLabel)")
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(.secondary)
            }
        }
    }
}

/// The chance of a reset within one, three and seven days, one bar each.
struct ChanceBars: View {
    let resets: GlanceResets
    var showsTitle = true
    var spacing: CGFloat = 6

    var body: some View {
        if !resets.forecast.isEmpty {
            bars
        }
    }

    private var bars: some View {
        VStack(alignment: .leading, spacing: spacing) {
            if showsTitle {
                SectionLabel(text: resets.forecastTitle)
            }
            ForEach(resets.forecast) { chance in
                VStack(alignment: .leading, spacing: 3) {
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        Text(chance.label)
                            .font(.system(size: WidgetScale.label))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text("\(chance.percent)%")
                            .font(.system(size: WidgetScale.value, weight: .semibold))
                            .monospacedDigit()
                            .fixedSize()
                    }
                    TintMeter(fraction: chance.fraction, color: resets.tint, height: 5)
                }
            }
        }
    }
}

/// The chance of a reset in the next 24 hours as the big number, the longer spans under it.
struct ChanceHero: View {
    let resets: GlanceResets

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            if let day = resets.chance(days: 1) ?? resets.forecast.first {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text("\(day.percent)%")
                        .font(.system(size: WidgetScale.hero, weight: .bold, design: .rounded))
                        .monospacedDigit()
                        .foregroundStyle(resets.tint)
                        .fixedSize()
                    Text(day.label)
                        .font(.system(size: WidgetScale.caption, weight: .medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text(resets.forecastTitle)
                    .font(.system(size: WidgetScale.footnote))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .padding(.bottom, 3)
                ForEach(resets.forecast.filter { $0.days != day.days }) { chance in
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        Text(chance.label)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                        Spacer(minLength: 4)
                        Text("\(chance.percent)%")
                            .fontWeight(.semibold)
                            .monospacedDigit()
                            .fixedSize()
                    }
                    .font(.system(size: WidgetScale.caption))
                }
            }
        }
    }
}

struct ResetChanceStrip: View {
    let resets: GlanceResets

    var body: some View {
        if !resets.forecast.isEmpty {
            VStack(alignment: .leading, spacing: 5) {
                SectionLabel(text: resets.forecastTitle)
                HStack(alignment: .top, spacing: 7) {
                    ForEach(resets.forecast) { chance in
                        VStack(alignment: .leading, spacing: 3) {
                            Text("\(chance.percent)%")
                                .font(.system(size: 13, weight: .semibold))
                                .monospacedDigit()
                            TintMeter(fraction: chance.fraction, color: resets.tint, height: 4)
                            Text(chance.label)
                                .font(.system(size: WidgetScale.footnote))
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
    }
}

// MARK: Calendar

/// The weeks and square size a calendar gets in a space: every week up to `most` at the biggest
/// square that fits, fewer weeks when squares would get too small to read.
struct CalendarFit {
    let weeks: Int
    let pitch: CGFloat

    static let labelWidth: CGFloat = 18
    static let monthsHeight: CGFloat = 12

    static func fit(_ calendar: GlanceResetCalendar, in size: CGSize, most: Int, largest: CGFloat = 18, smallest: CGFloat = 9) -> CalendarFit {
        let available = calendar.weekRows().count
        var weeks = max(1, min(most, available))
        let tall = (size.height - monthsHeight - 2) / 7
        var pitch = min(largest, tall, (size.width - labelWidth) / CGFloat(weeks))
        if pitch < smallest {
            let side = max(smallest, min(largest, tall))
            weeks = max(1, min(weeks, Int((size.width - labelWidth) / side)))
            pitch = min(largest, tall, (size.width - labelWidth) / CGFloat(weeks))
        }
        return CalendarFit(weeks: weeks, pitch: (pitch * 2).rounded(.down) / 2)
    }

    /// The grid's own size at this fit.
    var size: CGSize {
        CGSize(width: Self.labelWidth + CGFloat(weeks) * pitch, height: Self.monthsHeight + 2 + 7 * pitch)
    }
}

/// The reset calendar as a heat map: one column per week, oldest on the left, Monday on top; a
/// square per day, in the tracker's color for a regular reset, orange for a banked one, faint for
/// none and fainter still for days to come, with today outlined. Month names run along the top and
/// weekday names down the left.
struct ResetCalendarGrid: View {
    let calendar: GlanceResetCalendar
    let tint: Color
    let fit: CalendarFit

    var body: some View {
        let rows = calendar.weekRows(last: fit.weeks)
        let offset = calendar.weekRows().count - rows.count
        let todayWeek = calendar.today / 7 - offset
        let todayDay = calendar.today % 7
        let pitch = fit.pitch
        let side = pitch - max(2, (pitch * 0.18).rounded())
        VStack(alignment: .leading, spacing: 2) {
            ZStack(alignment: .topLeading) {
                ForEach(Array(monthMarks(offset: offset).enumerated()), id: \.offset) { _, mark in
                    Text(mark.label)
                        .font(.system(size: 8, weight: .medium))
                        .foregroundStyle(.secondary)
                        .fixedSize()
                        .offset(x: CalendarFit.labelWidth + CGFloat(mark.week) * pitch)
                }
            }
            .frame(width: fit.size.width, height: CalendarFit.monthsHeight, alignment: .topLeading)
            HStack(alignment: .top, spacing: 0) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(0..<7, id: \.self) { day in
                        Text(weekdayLabel(day))
                            .font(.system(size: 8))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .frame(width: CalendarFit.labelWidth, height: pitch, alignment: .leading)
                    }
                }
                ForEach(rows.indices, id: \.self) { week in
                    VStack(spacing: 0) {
                        ForEach(0..<7, id: \.self) { day in
                            cell(day < rows[week].count ? rows[week][day] : .future, side: side, today: week == todayWeek && day == todayDay)
                                .frame(width: pitch, height: pitch)
                        }
                    }
                }
            }
        }
        .frame(width: fit.size.width, height: fit.size.height, alignment: .topLeading)
    }

    private func weekdayLabel(_ day: Int) -> String {
        guard day < calendar.weekdays.count else { return "" }
        if fit.pitch >= 13 || day % 2 == 0 { return calendar.weekdays[day] }
        return ""
    }

    private func cell(_ kind: GlanceResetCalendar.Cell, side: CGFloat, today: Bool) -> some View {
        let shape = RoundedRectangle(cornerRadius: max(1.5, side * 0.24), style: .continuous)
        return shape
            .fill(color(kind))
            .overlay(today ? shape.strokeBorder(Color.primary, lineWidth: max(1.2, side * 0.12)) : nil)
            .frame(width: side, height: side)
    }

    private func color(_ kind: GlanceResetCalendar.Cell) -> Color {
        switch kind {
        case .regular: return tint
        case .banked: return .orange
        case .none: return Color.primary.opacity(0.09)
        case .future: return Color.primary.opacity(0.03)
        }
    }

    /// The month names over the visible weeks, dropping any that would overlap the one before; the
    /// month already running at the first visible week is named over it.
    private func monthMarks(offset: Int) -> [GlanceResetMonth] {
        var marks: [GlanceResetMonth] = []
        let running = calendar.months.last { $0.week <= offset }
        let visible = calendar.months.filter { $0.week > offset }
        if let running {
            marks.append(GlanceResetMonth(week: 0, label: running.label))
        }
        marks += visible.map { GlanceResetMonth(week: $0.week - offset, label: $0.label) }
        var kept: [GlanceResetMonth] = []
        var end: CGFloat = -.infinity
        for mark in marks where mark.week < fit.weeks {
            let start = CGFloat(mark.week) * fit.pitch
            if start >= end + 3 {
                kept.append(mark)
                end = start + CGFloat(mark.label.count) * 5 + 2
            } else if let last = kept.last, last.week == 0, mark.week * 2 <= fit.weeks {
                kept[kept.count - 1] = mark
                end = start + CGFloat(mark.label.count) * 5 + 2
            }
        }
        return kept
    }
}

/// What the calendar's colors mean.
struct CalendarLegend: View {
    let legend: GlanceResetLegend
    let tint: Color
    var axis: Axis = .horizontal

    var body: some View {
        let items = Group {
            swatch(tint, legend.regular)
            swatch(.orange, legend.banked)
            HStack(spacing: 3) {
                RoundedRectangle(cornerRadius: 2, style: .continuous)
                    .strokeBorder(Color.primary, lineWidth: 1.2)
                    .frame(width: 8, height: 8)
                Text(legend.today).lineLimit(1)
            }
        }
        Group {
            if axis == .horizontal {
                HStack(spacing: 8) { items }
            } else {
                VStack(alignment: .leading, spacing: 3) { items }
            }
        }
        .font(.system(size: WidgetScale.footnote))
        .foregroundStyle(.secondary)
        .fixedSize()
    }

    private func swatch(_ color: Color, _ text: String) -> some View {
        HStack(spacing: 3) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(color)
                .frame(width: 8, height: 8)
            Text(text).lineLimit(1)
        }
    }
}

/// A small bar chart of how many resets fell in each bucket (weekday or four-hour block), the
/// busiest bucket in full color.
struct RhythmBars: View {
    let title: String
    let buckets: [GlanceResetBucket]
    let tint: Color
    var height: CGFloat = 44

    var body: some View {
        let top = max(buckets.map(\.count).max() ?? 0, 1)
        VStack(alignment: .leading, spacing: 3) {
            SectionLabel(text: title)
            HStack(alignment: .bottom, spacing: 4) {
                ForEach(Array(buckets.enumerated()), id: \.offset) { _, bucket in
                    VStack(spacing: 2) {
                        Text("\(bucket.count)")
                            .font(.system(size: 8, weight: .medium))
                            .monospacedDigit()
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .fixedSize()
                        RoundedRectangle(cornerRadius: 2, style: .continuous)
                            .fill(bucket.count == top ? tint : tint.opacity(0.4))
                            .frame(height: max(2, height * CGFloat(bucket.count) / CGFloat(top)))
                        Text(bucket.label)
                            .font(.system(size: 8))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .fixedSize()
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .frame(height: height + 24, alignment: .bottom)
        }
    }
}

/// The weekday and hour charts side by side, or stacked in a narrow column.
struct RhythmView: View {
    let rhythm: GlanceResetRhythm
    let tint: Color
    var height: CGFloat = 44
    var stacked = false

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .firstTextBaseline) {
                Text(rhythm.title)
                    .font(.system(size: WidgetScale.title, weight: .semibold))
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text("\(rhythm.total)")
                    .font(.system(size: WidgetScale.caption, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
            }
            if stacked {
                VStack(alignment: .leading, spacing: 8) { charts }
            } else {
                HStack(alignment: .top, spacing: WidgetScale.columnSpacing) { charts }
            }
        }
    }

    @ViewBuilder
    private var charts: some View {
        RhythmBars(title: rhythm.weekdayTitle, buckets: rhythm.weekdays, tint: tint, height: height)
        RhythmBars(title: rhythm.hourTitle, buckets: rhythm.hours, tint: tint, height: height)
    }
}

// MARK: Codex Resets widget

/// The Codex free-reset tracker: the announced reset or the chance of one, how long since the last,
/// and with room the wait so far, the calendar and the rhythm.
struct CodexResetsLayout: View {
    let document: GlanceDocument
    let resets: GlanceResets
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    private var units: GlanceUnits { document.labels.units }

    var body: some View {
        switch family {
        case .systemSmall: small
        case .systemMedium: medium
        case .systemLarge: large
        default: extraLarge
        }
    }

    private var small: some View {
        VStack(alignment: .leading, spacing: 6) {
            ResetsHeader(resets: resets)
            if let stale = resets.stale {
                ResetsStaleLine(text: stale)
            }
            if let upcoming = resets.upcoming(at: now) {
                AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 14)
            } else {
                ChanceHero(resets: resets)
            }
            Spacer(minLength: 0)
            if let latest = resets.latest {
                latest.since.live(now: now, units: units)
                    .font(.system(size: WidgetScale.caption))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var medium: some View {
        VStack(alignment: .leading, spacing: 7) {
            ResetsHeader(resets: resets, showsSource: resets.stale == nil)
            if let stale = resets.stale {
                ResetsStaleLine(text: stale)
            }
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing + 4) {
                VStack(alignment: .leading, spacing: 8) {
                    let upcoming = resets.upcoming(at: now)
                    if let upcoming {
                        AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13.5)
                    }
                    if let latest = resets.latest {
                        LatestResetBlock(
                            latest: latest,
                            tint: resets.tint,
                            units: units,
                            now: now,
                            valueSize: upcoming == nil ? 13.5 : WidgetScale.caption,
                            showsWhen: upcoming == nil
                        )
                        .foregroundStyle(upcoming == nil ? Color.primary : Color.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                ChanceBars(resets: resets, spacing: 5)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var large: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                ResetsHeader(resets: resets)
                HiddenCount(count: largeSecondaryCount)
            }
            if let stale = resets.stale {
                ResetsStaleLine(text: stale)
            }
            if let upcoming = resets.upcoming(at: now) {
                AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 15, captionLines: 2)
            }
            if let latest = resets.latest {
                VStack(alignment: .leading, spacing: 2) {
                    if resets.upcoming(at: now) == nil {
                        latest.since.live(now: now, units: units)
                            .font(.system(size: WidgetScale.value, weight: .semibold))
                            .lineLimit(2)
                    }
                    Text("\(latest.label): \(latest.when) · \(latest.kindLabel)")
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            ResetChanceStrip(resets: resets)
            if let calendar = resets.calendar {
                GeometryReader { proxy in
                    let fit = CalendarFit.fit(
                        calendar, in: CGSize(width: proxy.size.width, height: max(42, proxy.size.height - 30)),
                        most: calendar.weeks, largest: 15, smallest: 7
                    )
                    VStack(alignment: .leading, spacing: 4) {
                        HStack(spacing: 4) {
                            SectionLabel(text: calendar.title)
                            Spacer(minLength: 0)
                            HiddenCount(count: calendar.weeks - fit.weeks)
                        }
                        ResetCalendarGrid(calendar: calendar, tint: resets.tint, fit: fit)
                        CalendarLegend(legend: calendar.legend, tint: resets.tint)
                    }
                }
            } else if let rhythm = resets.rhythm {
                RhythmView(rhythm: rhythm, tint: resets.tint, height: 30)
            } else {
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var largeSecondaryCount: Int {
        (resets.wait == nil ? 0 : 1) + (resets.median == nil ? 0 : 1)
            + (resets.forecastNote.isEmpty ? 0 : 1)
            + (resets.upcoming(at: now)?.note == nil ? 0 : 1)
            + (resets.calendar != nil && resets.rhythm != nil ? 1 : 0)
    }

    private var extraLarge: some View { balanced }

    private var balanced: some View {
        let hasSummary = resets.upcoming(at: now) != nil || resets.latest != nil || !resets.forecast.isEmpty
            || !resets.forecastNote.isEmpty || resets.wait != nil || resets.median != nil
        let hasHistory = resets.calendar != nil || resets.rhythm != nil
        let left = hasHistory ? (size.width - WidgetScale.columnSpacing) * (family == .systemExtraLarge ? 0.46 : 0.5) : size.width
        let right = hasSummary ? size.width - left - WidgetScale.columnSpacing : size.width
        return VStack(alignment: .leading, spacing: 7) {
            ResetsHeader(resets: resets, showsSource: true)
            if let stale = resets.stale {
                ResetsStaleLine(text: stale)
            }
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                if hasSummary {
                    summaryColumn(width: left)
                }
                if hasHistory {
                    historyColumn(width: right)
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func summaryColumn(width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            if let upcoming = resets.upcoming(at: now) {
                AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13, captionLines: 1, showsNote: true)
            }
            if let latest = resets.latest {
                LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now, valueSize: WidgetScale.value)
            }
            ResetChanceStrip(resets: resets)
            if !resets.forecastNote.isEmpty {
                note(resets.forecastNote)
            }
            if let wait = resets.wait { note(wait) }
            if let median = resets.median { note(median) }
        }
        .frame(width: width, alignment: .topLeading)
    }

    private func historyColumn(width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            if let calendar = resets.calendar {
                let fit = CalendarFit.fit(
                    calendar, in: CGSize(width: width, height: min(120, size.height * 0.3)),
                    most: calendar.weeks, largest: 14, smallest: 4
                )
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 4) {
                        SectionLabel(text: calendar.title)
                        Spacer(minLength: 0)
                        HiddenCount(count: calendar.weeks - fit.weeks)
                    }
                    ResetCalendarGrid(calendar: calendar, tint: resets.tint, fit: fit)
                    ViewThatFits(in: .horizontal) {
                        CalendarLegend(legend: calendar.legend, tint: resets.tint)
                        CalendarLegend(legend: calendar.legend, tint: resets.tint, axis: .vertical)
                    }
                }
            }
            if let rhythm = resets.rhythm {
                RhythmView(rhythm: rhythm, tint: resets.tint, height: width < 220 ? 20 : 38, stacked: width < 220)
            }
        }
        .frame(width: width, alignment: .topLeading)
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.system(size: WidgetScale.caption))
            .foregroundStyle(.secondary)
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
    }

}

// MARK: Reset calendar widget

/// The reset calendar as big as the widget allows, with its legend and, with room, the rhythm by
/// weekday and hour and the last reset.
struct ResetCalendarLayout: View {
    let document: GlanceDocument
    let resets: GlanceResets
    let calendar: GlanceResetCalendar
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    private var units: GlanceUnits { document.labels.units }

    var body: some View {
        switch family {
        case .systemMedium, .systemSmall: medium
        case .systemLarge: large
        default: extraLarge
        }
    }

    private var heading: some View {
        ResetsHeader(resets: resets, title: "\(resets.title) · \(calendar.title)")
    }

    private var medium: some View {
        let fit = CalendarFit.fit(calendar, in: CGSize(width: size.width, height: size.height - 34), most: calendar.weeks)
        return VStack(alignment: .leading, spacing: 5) {
            ResetsHeader(resets: resets)
            ResetCalendarGrid(calendar: calendar, tint: resets.tint, fit: fit)
            Spacer(minLength: 0)
            CalendarLegend(legend: calendar.legend, tint: resets.tint)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var large: some View {
        let fit = CalendarFit.fit(calendar, in: CGSize(width: size.width, height: 150), most: calendar.weeks)
        return VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
            heading
            VStack(alignment: .leading, spacing: 5) {
                ResetCalendarGrid(calendar: calendar, tint: resets.tint, fit: fit)
                CalendarLegend(legend: calendar.legend, tint: resets.tint)
            }
            Spacer(minLength: 0)
            if let rhythm = resets.rhythm {
                RhythmView(rhythm: rhythm, tint: resets.tint, height: 58)
            }
            if let latest = resets.latest {
                LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now, valueSize: WidgetScale.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var extraLarge: some View {
        let side = min(250, size.width * 0.33)
        let fit = CalendarFit.fit(calendar, in: CGSize(width: size.width - side - WidgetScale.columnSpacing * 2 - 1, height: size.height - 120), most: calendar.weeks, largest: 22)
        return HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
            VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
                heading
                VStack(alignment: .leading, spacing: 5) {
                    ResetCalendarGrid(calendar: calendar, tint: resets.tint, fit: fit)
                    CalendarLegend(legend: calendar.legend, tint: resets.tint)
                }
                Spacer(minLength: 0)
                if let latest = resets.latest {
                    LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now)
                }
                if let wait = resets.wait {
                    Text(wait)
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            Divider()
            VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
                if let rhythm = resets.rhythm {
                    RhythmView(rhythm: rhythm, tint: resets.tint, height: 52, stacked: true)
                }
                Spacer(minLength: 0)
                if let median = resets.median {
                    Text(median)
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                Text(resets.source)
                    .font(.system(size: WidgetScale.footnote))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            }
            .frame(width: side, alignment: .topLeading)
            .frame(maxHeight: .infinity, alignment: .topLeading)
        }
    }
}
