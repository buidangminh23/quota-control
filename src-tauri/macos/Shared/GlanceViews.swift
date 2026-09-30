import AppKit
import SwiftUI

/// The capsule meter (the popup's `Meter`): a faint track and a flat fill in the pace color. Any
/// non-zero fill is at least as wide as the bar is tall, so 1-2% never disappears.
struct GlanceMeter: View {
    let fraction: Double
    let severity: GlanceSeverity
    var onDark = false
    var height: CGFloat = 5

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let clamped = min(max(fraction, 0), 1)
            let fill = clamped > 0 ? max(width * clamped, height) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12))
                if fill > 0 {
                    Capsule()
                        .fill(GlancePalette.fill(severity, onDark: onDark))
                        .frame(width: min(fill, width))
                }
            }
        }
        .frame(height: height)
    }
}

/// A round gauge: a faint track and an arc in the pace color starting at twelve o'clock, with
/// whatever `center` draws inside. A metric without a limit draws the track alone.
struct GlanceRing<Center: View>: View {
    let fraction: Double?
    let severity: GlanceSeverity
    var onDark = false
    var lineWidth: CGFloat = 4
    @ViewBuilder var center: () -> Center

    var body: some View {
        ZStack {
            Circle()
                .stroke(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12), lineWidth: lineWidth)
            if let fraction, fraction > 0 {
                Circle()
                    .trim(from: 0, to: min(max(fraction, 0.03), 1))
                    .stroke(
                        GlancePalette.fill(severity, onDark: onDark),
                        style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)
                    )
                    .rotationEffect(.degrees(-90))
            }
            center()
        }
        .padding(lineWidth / 2)
    }
}

/// When the metric comes back: a countdown against `now`, the reset moment itself, or the metric's
/// own detail text.
struct GlanceResetText: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date

    var body: some View {
        Text(text)
            .monospacedDigit()
            .lineLimit(1)
    }

    private var text: String {
        if let resetsAt = metric.resetsAt {
            if resetsAt <= now { return labels.resetting }
            return "\(labels.resetsIn) \(GlanceFormat.countdown(to: resetsAt, from: now, units: labels.units))"
        }
        return metric.detail ?? ""
    }
}

/// One metric: its label and headline over the meter, and the reset countdown beneath.
struct GlanceMetricRow: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date
    var onDark = false
    var compact = false
    var showsReset = true

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 2 : 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(metric.label)
                    .font(.system(size: compact ? 10.5 : 11.5, weight: .medium))
                    .foregroundStyle(onDark ? Color.white.opacity(0.72) : Color.secondary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text(metric.headline)
                    .font(.system(size: compact ? 11 : 12.5, weight: .semibold))
                    .foregroundStyle(GlancePalette.text(metric.severity, onDark: onDark))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            if let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: onDark, height: compact ? 4 : 5)
            }
            if showsReset, metric.resetsAt != nil || metric.detail != nil {
                GlanceResetText(metric: metric, labels: labels, now: now)
                    .font(.system(size: compact ? 9.5 : 10.5))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary.opacity(0.9))
            }
        }
    }
}

/// A provider's mark in its brand color beside its name and plan, with the account's email under.
struct GlanceProviderHeader: View {
    let provider: GlanceProvider
    var shows: GlanceShows = .all
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                ProviderMark(mark: provider.mark)
                    .foregroundStyle(onDark ? provider.tint : markColor)
                    .frame(width: size, height: size)
                Text(provider.name)
                    .font(.system(size: size - 1, weight: .semibold))
                    .foregroundStyle(onDark ? Color.white : Color.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if shows.plan, let plan = provider.plan {
                    GlancePlanBadge(text: plan, onDark: onDark, size: size)
                }
            }
            if shows.account, let account = provider.account {
                Text(account)
                    .font(.system(size: max(size - 3.5, 8.5)))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, size + 5)
            }
        }
    }

    private var markColor: Color {
        provider.color.uppercased() == "#FFFFFF" ? .primary : provider.tint
    }
}

/// The plan (`Pro`, `Max 5x`) in a small rounded tag.
struct GlancePlanBadge: View {
    let text: String
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        Text(text)
            .font(.system(size: max(size - 4, 8), weight: .semibold))
            .foregroundStyle(onDark ? Color.white.opacity(0.7) : Color.secondary)
            .lineLimit(1)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(
                RoundedRectangle(cornerRadius: 3, style: .continuous)
                    .fill(onDark ? Color.white.opacity(0.12) : Color.primary.opacity(0.08))
            )
            .fixedSize()
    }
}

/// Why an account shows no readings: signed out, session expired, no data yet.
struct GlanceNoticeRow: View {
    let text: String
    var onDark = false
    var size: CGFloat = 10.5

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: size - 1))
            Text(text)
                .font(.system(size: size))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(onDark ? Color(red: 1.0, green: 0.62, blue: 0.04) : Color.orange)
    }
}


enum GlanceResetTextStyle {
    case body, secondary, heading, value
}

enum GlanceResetElement {
    case text(String, GlanceResetTextStyle)
    case author(GlanceResetAuthor, String)
    case badge(String)
    case chances([GlanceResetForecastChance])
    case meter(Double)
    case calendar(GlanceResetCalendar, Range<Int>, Range<Int>)
    case legend(GlanceResetLegend)
    case rhythm(String, [GlanceResetBucket])
    case stat(String, String)
    case link(String)
    case divider
    /// An announcement quoted in a box, like the message under the Reset tab's latest reset.
    case message([GlanceResetElement])
}

struct GlanceResetCardData: Identifiable {
    let id: String
    var title: String
    var accent: Color?
    var elements: [GlanceResetElement]
}

struct GlanceResetPalette {
    let scheme: ColorScheme
    /// What the tracker's cards sit on, like the Reset tab's page.
    var background: Color { Color(glanceHex: scheme == .dark ? "#1e1e1e" : "#ffffff")! }
    var card: Color { Color(glanceHex: scheme == .dark ? "#2a2a2b" : "#f4f4f5")! }
    var blue: Color { Color(glanceHex: scheme == .dark ? "#0a84ff" : "#007aff")! }
    var yellow: Color { Color(glanceHex: scheme == .dark ? "#ffd60a" : "#f5b800")! }
    /// What a quoted announcement sits on inside a card, the popup's `--uc-quinary`.
    var message: Color { scheme == .dark ? Color.white.opacity(0.07) : Color.black.opacity(0.05) }
    static let orange = Color(glanceHex: "#ff9500")!
}

struct GlanceResetCardView: View {
    let card: GlanceResetCardData
    let availableWidth: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    private var grouped: Bool {
        ["forecast", "calendar", "rhythm", "stats", "history", "method"].contains { card.id.hasPrefix($0) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if grouped && !card.title.isEmpty {
                Text(card.title).font(.system(size: 10, weight: .semibold)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: 8) {
                if !grouped && !card.title.isEmpty {
                    Text(card.id.hasPrefix("latest") ? card.title.uppercased() : card.title)
                        .font(.system(size: card.id.hasPrefix("latest") ? 10 : 14, weight: card.id.hasPrefix("latest") ? .semibold : .bold))
                        .tracking(card.id.hasPrefix("latest") ? 0.6 : 0)
                        .foregroundStyle(card.id.hasPrefix("latest") ? Color.secondary : Color.primary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(Array(card.elements.enumerated()), id: \.offset) { _, element in
                    GlanceResetElementView(element: element, availableWidth: max(1, availableWidth - 24))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(GlanceResetPalette(scheme: colorScheme).card))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(card.accent ?? Color.primary.opacity(0.09), lineWidth: card.accent == nil ? 0.5 : 2))
        }
        .frame(width: availableWidth, alignment: .leading)
    }

}

struct GlanceResetElementView: View {
    let element: GlanceResetElement
    let availableWidth: CGFloat
    @Environment(\.colorScheme) private var colorScheme
    private var palette: GlanceResetPalette { GlanceResetPalette(scheme: colorScheme) }

    @ViewBuilder
    var body: some View {
        switch element {
        case let .text(text, style):
            Text(text)
                .font(.system(size: style == .value ? 20 : style == .secondary ? 10 : 11, weight: style == .value ? .bold : style == .heading ? .semibold : .regular))
                .foregroundStyle(style == .secondary ? Color.secondary : Color.primary)
                .fixedSize(horizontal: false, vertical: true)
        case let .author(author, avatar):
            HStack(spacing: 6) {
                if let image = Self.avatar(avatar) {
                    Image(nsImage: image).resizable().scaledToFill().frame(width: 22, height: 22).clipShape(Circle())
                }
                Text(author.handle).font(.system(size: 10, weight: .medium)).lineLimit(1).minimumScaleFactor(0.8)
            }
        case let .badge(text):
            Text(text)
                .font(.system(size: 24, weight: .heavy))
                .foregroundStyle(Color(red: 0.11, green: 0.11, blue: 0.12))
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 10).padding(.vertical, 2)
                .background(RoundedRectangle(cornerRadius: 6).fill(palette.yellow))
        case let .chances(chances):
            HStack(alignment: .top, spacing: 12) {
                ForEach(chances) { chance in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(chance.percent).font(.system(size: 20, weight: .bold)).monospacedDigit()
                        meter(chance.fraction)
                        Text(chance.label).font(.system(size: 10)).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        case let .meter(fraction):
            meter(fraction)
        case let .calendar(calendar, weeks, days):
            calendarGrid(calendar, weeks: weeks, days: days)
        case let .legend(legend):
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { legendItems(legend) }
                VStack(alignment: .leading, spacing: 5) { legendItems(legend) }
            }
            .font(.system(size: 10.5)).foregroundStyle(.secondary)
        case let .rhythm(title, buckets):
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.system(size: 11)).foregroundStyle(.secondary)
                HStack(alignment: .bottom, spacing: 4) {
                    let peak = max(1, buckets.map(\.count).max() ?? 1)
                    ForEach(Array(buckets.enumerated()), id: \.offset) { _, bucket in
                        VStack(spacing: 4) {
                            Text("\(bucket.count)").font(.system(size: 10)).monospacedDigit()
                            RoundedRectangle(cornerRadius: 2).fill(palette.blue.opacity(bucket.count == peak ? 1 : 0.5))
                                .frame(height: max(2, 42 * CGFloat(bucket.count) / CGFloat(peak)))
                                .frame(height: 42, alignment: .bottom)
                            Text(bucket.label).font(.system(size: 10)).foregroundStyle(.secondary)
                        }.frame(maxWidth: .infinity)
                    }
                }
            }
        case let .stat(label, value):
            VStack(alignment: .leading, spacing: 3) {
                Text(label).foregroundStyle(.secondary)
                Text(value).fontWeight(.semibold)
            }.font(.system(size: 11.5)).fixedSize(horizontal: false, vertical: true)
        case .divider:
            Rectangle().fill(Color.primary.opacity(0.1)).frame(height: 0.5)
        case let .link(url):
            if let destination = URL(string: url), ["https", "http"].contains(destination.scheme ?? "") {
                Link(destination: destination) { Image(systemName: "arrow.up.right.square").font(.system(size: 14)) }
                    .accessibilityLabel(url)
            }
        case let .message(lines):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    GlanceResetElementView(element: line, availableWidth: max(1, availableWidth - 20))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(palette.message))
        }
    }

    private func meter(_ fraction: Double) -> some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.1))
                Capsule().fill(palette.blue).frame(width: proxy.size.width * min(max(fraction, 0), 1))
            }
        }.frame(height: 5)
    }

    private static func avatar(_ value: String) -> NSImage? {
        guard let comma = value.firstIndex(of: ","), value.hasPrefix("data:image/"),
              let data = Data(base64Encoded: String(value[value.index(after: comma)...])) else { return nil }
        return NSImage(data: data)
    }

    @ViewBuilder
    private func legendItems(_ legend: GlanceResetLegend) -> some View {
        HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).fill(palette.blue).frame(width: 9, height: 9); Text(legend.regular) }
        HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).fill(Color.orange).frame(width: 9, height: 9); Text(legend.banked) }
        HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).strokeBorder(Color.primary, lineWidth: 1).frame(width: 9, height: 9); Text(legend.today) }
    }

    private func calendarGrid(_ calendar: GlanceResetCalendar, weeks: Range<Int>, days: Range<Int>) -> some View {
        let rows = calendar.weekRows()
        let pitch = max(3, (availableWidth - 22) / CGFloat(max(weeks.count, 1)))
        let side = min(16, pitch - 2)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 0) {
                Color.clear.frame(width: 22, height: 11)
                ForEach(Array(weeks), id: \.self) { week in
                    Text(calendar.months.first(where: { $0.week == week })?.label ?? "")
                        .font(.system(size: 8.5)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: true, vertical: false)
                        .frame(width: pitch, alignment: .leading)
                }
            }
            ForEach(Array(days), id: \.self) { day in
                HStack(spacing: 0) {
                    Text(day < calendar.weekdays.count ? calendar.weekdays[day] : "")
                        .font(.system(size: 8.5)).foregroundStyle(.secondary).frame(width: 22, alignment: .leading)
                    ForEach(Array(weeks), id: \.self) { week in
                        let value = week < rows.count && day < rows[week].count ? rows[week][day] : .future
                        RoundedRectangle(cornerRadius: 2)
                            .fill(value == .regular ? palette.blue : value == .banked ? GlanceResetPalette.orange : Color.primary.opacity(value == .future ? 0.035 : 0.1))
                            .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Color.primary.opacity(week * 7 + day == calendar.today ? 1 : 0), lineWidth: 1.5))
                            .frame(width: max(1, side), height: max(1, side))
                            .frame(width: pitch, height: max(1, side))
                    }
                }
            }
        }
    }
}

struct GlanceResetContent: View {
    @Environment(\.colorScheme) private var colorScheme
    let resets: GlanceResets
    let units: GlanceUnits
    let now: Date
    let availableWidth: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(GlanceResetCards.make(resets: resets, units: units, now: now)) { card in
                GlanceResetCardView(card: card, availableWidth: availableWidth)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.colorScheme, resets.theme == "dark" ? .dark : resets.theme == "light" ? .light : colorScheme)
    }
}


enum GlanceResetCards {
    static func make(resets: GlanceResets, units: GlanceUnits, now: Date) -> [GlanceResetCardData] {
        var cards: [GlanceResetCardData] = []
        func add(_ id: String, _ title: String, _ elements: [GlanceResetElement], accent: Color? = nil) {
            cards.append(GlanceResetCardData(id: id, title: title, accent: accent, elements: elements))
        }
        if let stale = resets.stale {
            add("stale", "", [.text(stale, .body)], accent: GlanceResetPalette.orange)
        }
        if let presentation = resets.presentation {
            if let latest = presentation.latest {
                let author = latest.author.map { GlanceResetElement.author($0, presentation.avatar(for: $0)) }
                var elements: [GlanceResetElement] = []
                if latest.excerpt == nil, let author { elements.append(author) }
                elements += [.badge(latest.ago(now: now, locale: presentation.locale)), .text(latest.meta, .secondary)]
                if let excerpt = latest.excerpt {
                    var lines: [GlanceResetElement] = author.map { [$0] } ?? []
                    lines.append(.text(excerpt, .body))
                    if let observed = latest.observed { lines.append(.text(observed, .secondary)) }
                    if let url = latest.url { lines.append(.link(url)) }
                    elements.append(.message(lines))
                }
                elements += (latest.notes ?? []).map { .text($0, .secondary) }
                add("latest", latest.title, elements)
            }
            let quoted = presentation.latest?.excerpt != nil
            for status in presentation.statuses(at: now) {
                let repeats = quoted && status.sameAsLatest == true
                var elements: [GlanceResetElement] = []
                let metadata = status.metadata(now: now, units: units)
                if status.kind == "watch", metadata.count > 1 { elements.append(.text(metadata[0], .secondary)) }
                if let author = status.author, !repeats { elements.append(.author(author, presentation.avatar(for: author))) }
                if let excerpt = status.excerpt, !repeats { elements.append(.text(excerpt, .body)) }
                elements += (status.kind == "watch" && metadata.count > 1 ? Array(metadata.dropFirst()) : metadata).map { .text($0, .secondary) }
                if let due = status.due(now: now, units: units) { elements.append(.text(due, .secondary)) }
                if let url = status.url { elements.append(.link(url)) }
                add(status.id, status.title, elements, accent: accent(status.kind))
            }
            let forecast = presentation.forecast
            var elements: [GlanceResetElement] = []
            if !forecast.chances.isEmpty { elements.append(.chances(forecast.chances)) }
            if let wait = forecast.wait { elements += [.divider, .text(wait, .heading)] }
            if let fraction = forecast.waitFraction { elements.append(.meter(fraction)) }
            for text in [forecast.median, forecast.sampleNote, forecast.reliability, forecast.disclaimer, forecast.unavailable].compactMap({ $0 }) {
                elements.append(.text(text, .secondary))
            }
            if !elements.isEmpty { add("forecast", forecast.title, elements) }
        } else {
            if let latest = resets.latest {
                add("latest", latest.label, [.badge(latest.since.text(now: now, units: units)), .text(latest.when, .secondary)])
            }
            if let upcoming = resets.upcoming(at: now) {
                let lines = upcoming.lines(now: now, units: units)
                var elements: [GlanceResetElement] = [.text(lines.value, .heading), .text(lines.caption, .secondary)]
                if let note = upcoming.note { elements.append(.text(note, .secondary)) }
                add("upcoming", upcoming.title, elements, accent: upcoming.tone == .positive ? .green : GlanceResetPalette.orange)
            }
            var elements: [GlanceResetElement] = []
            if !resets.forecast.isEmpty {
                elements.append(.chances(resets.forecast.map { GlanceResetForecastChance(days: $0.days, label: $0.label, percent: "\($0.percent)%", fraction: $0.fraction) }))
            }
            for text in [resets.wait, resets.median, resets.forecastNote.isEmpty ? nil : resets.forecastNote].compactMap({ $0 }) {
                elements.append(.text(text, .secondary))
            }
            if !elements.isEmpty { add("forecast", resets.forecastTitle, elements) }
        }
        if let calendar = resets.calendar {
            add("calendar", calendar.title, [.calendar(calendar, 0..<calendar.weekRows().count, 0..<7), .legend(calendar.legend)])
        }
        if let rhythm = resets.rhythm {
            var elements: [GlanceResetElement] = [.rhythm(rhythm.weekdayTitle, rhythm.weekdays), .rhythm(rhythm.hourTitle, rhythm.hours)]
            if let text = resets.presentation?.patternNote, !text.isEmpty { elements.append(.text(text, .secondary)) }
            add("rhythm", rhythm.title, elements)
        }
        if let presentation = resets.presentation {
            if !presentation.stats.isEmpty {
                add("stats", presentation.statsTitle, presentation.stats.map { .stat($0.label, $0.value) })
            }
            for item in presentation.history {
                var elements: [GlanceResetElement] = []
                if let author = item.author { elements.append(.author(author, presentation.avatar(for: author))) }
                elements += [.text("\(item.kindLabel) · \(item.when)", .secondary), .text(item.excerpt, .body)]
                elements += [item.scope, item.provisional, item.observed].compactMap { $0 }.map { .text($0, .secondary) }
                if let url = item.url { elements.append(.link(url)) }
                add("history-\(item.id)", presentation.historyTitle, elements)
            }
            add("source", "", [.text(presentation.source, .secondary), .link(resets.site ?? "https://codex-resets.com")])
            if !presentation.method.isEmpty {
                add("method", presentation.methodTitle, presentation.method.map { .text($0, .body) })
            }
        }
        return cards
    }

    /// A status card's border: orange for a watch, green for a scheduled reset, blue for a banked
    /// reset still to apply, none for the quiet card.
    private static func accent(_ kind: String) -> Color? {
        switch kind {
        case "watch": return GlanceResetPalette.orange
        case "scheduled": return .green
        case "banked": return .blue
        default: return nil
        }
    }
}
