import SwiftUI
import WidgetKit
import AppIntents
import AppKit

// MARK: Reset tracker parts

/// The reset tracker's heading: its mark, the title and, where there is room, the source.
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

// MARK: Codex Resets widget

/// The reset tracker the widget chose, Codex's or Claude's: the announced (or banked) reset or the
/// chance of one, how long since the last, and with room the wait so far, the calendar and the rhythm.
struct CodexResetsLayout: View {
    let document: GlanceDocument
    let resets: GlanceResets
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    var body: some View {
        ResetWidgetPager(document: document, resets: resets, family: family, now: now, size: size, namespace: "resets")
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

    var body: some View {
        ResetWidgetPager(document: document, resets: resets, family: family, now: now, size: size, namespace: "calendar", initialCard: "calendar")
    }

}

struct ChangeResetWidgetPage: AppIntent {
    static var title: LocalizedStringResource = "Change reset page"
    static var openAppWhenRun: Bool = false

    @Parameter(title: "Widget") var key: String
    @Parameter(title: "Page") var page: Int

    init() {}
    init(key: String, page: Int) {
        self.key = key
        self.page = page
    }

    func perform() async throws -> some IntentResult {
        UserDefaults.standard.set(page, forKey: key)
        WidgetCenter.shared.reloadAllTimelines()
        return .result()
    }
}

struct ResetWidgetPager: View {
    let document: GlanceDocument
    let resets: GlanceResets
    let family: WidgetFamily
    let now: Date
    let size: CGSize
    let namespace: String
    var initialCard: String?
    var prefixPages: [AnyView] = []

    var body: some View {
        let height = max(40, size.height - 46)
        let cards = GlanceResetCards.make(resets: resets, units: document.labels.units, now: now)
        let fragments = ResetWidgetPagination.pages(cards, width: size.width, height: height)
        let pages = ResetWidgetPagination.spreads(fragments, width: size.width, height: height)
        let tracker = document.widget.resetsProvider == .codex ? "" : ".\(document.widget.resetsProvider.rawValue)"
        let key = "reset-page.\(namespace)\(tracker).\(family.rawValue)"
        let initial = initialCard.flatMap { id in pages.firstIndex(where: { $0.contains(where: { $0.id.hasPrefix(id + "|") || $0.id == id }) }) }.map { prefixPages.count + $0 } ?? 0
        let stored = UserDefaults.standard.object(forKey: key) == nil ? initial : UserDefaults.standard.integer(forKey: key)
        let count = prefixPages.count + pages.count
        let index = min(max(0, stored), max(0, count - 1))
        let sections = ResetWidgetSections(pageIDs: prefixPages.indices.map { ["overview-\($0)"] } + pages.map { $0.map(\.id) }, index: index)
        VStack(alignment: .leading, spacing: 4) {
            Group {
                if index < prefixPages.count {
                    prefixPages[index]
                } else if index - prefixPages.count < pages.count {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(pages[index - prefixPages.count]) { card in
                            GlanceResetCardView(card: card, availableWidth: size.width)
                        }
                    }
                }
            }
            .frame(width: size.width, height: height, alignment: .topLeading)
            HStack(spacing: 2) {
                Button(intent: ChangeResetWidgetPage(key: key, page: sections.previous)) {
                    Image(systemName: "chevron.left.2").frame(width: 20, height: 22)
                }
                .disabled(!sections.hasPrevious)
                .accessibilityLabel(document.isVietnamese ? "Phần trước" : "Previous section")
                .help(document.isVietnamese ? "Phần trước" : "Previous section")
                Button(intent: ChangeResetWidgetPage(key: key, page: max(0, index - 1))) {
                    Image(systemName: "chevron.left").frame(width: 20, height: 22)
                }
                .disabled(index == 0)
                .accessibilityLabel(document.isVietnamese ? "Trang trước" : "Previous page")
                Spacer(minLength: 0)
                Text("\(sections.page) / \(sections.count)")
                    .font(.system(size: 10, weight: .medium)).monospacedDigit()
                    .accessibilityLabel("\(sections.page) / \(sections.count)")
                Spacer(minLength: 0)
                Button(intent: ChangeResetWidgetPage(key: key, page: min(max(0, count - 1), index + 1))) {
                    Image(systemName: "chevron.right").frame(width: 20, height: 22)
                }
                .disabled(index + 1 >= count)
                .accessibilityLabel(document.isVietnamese ? "Trang sau" : "Next page")
                Button(intent: ChangeResetWidgetPage(key: key, page: sections.next)) {
                    Image(systemName: "chevron.right.2").frame(width: 20, height: 22)
                }
                .disabled(!sections.hasNext)
                .accessibilityLabel(document.isVietnamese ? "Phần sau" : "Next section")
                .help(document.isVietnamese ? "Phần sau" : "Next section")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .frame(height: 22)
            UpdatedFooter(document: document, now: now)
        }
    }
}

struct ResetWidgetSections {
    let previous: Int
    let next: Int
    let hasPrevious: Bool
    let hasNext: Bool
    let page: Int
    let count: Int

    init(ids: [String], index: Int) {
        self.init(pageIDs: ids.map { [$0] }, index: index)
    }

    init(pageIDs: [[String]], index: Int) {
        var starts: [Int] = []
        var previousGroup: String?
        for (page, ids) in pageIDs.enumerated() {
            for id in ids {
                let group = id.hasPrefix("history-") ? "history" : String(id.split(separator: "|")[0])
                if group != previousGroup && starts.last != page { starts.append(page) }
                previousGroup = group
            }
        }
        let section = starts.lastIndex(where: { $0 <= index }) ?? 0
        let start = starts.isEmpty ? 0 : starts[section]
        let end = section + 1 < starts.count ? starts[section + 1] : pageIDs.count
        previous = starts.isEmpty ? 0 : starts[max(0, section - 1)]
        next = starts.isEmpty ? 0 : starts[min(starts.count - 1, section + 1)]
        hasPrevious = section > 0
        hasNext = section + 1 < starts.count
        page = pageIDs.isEmpty ? 0 : index - start + 1
        count = end - start
    }
}

@MainActor
enum ResetWidgetPagination {
    private static var cache: [String: [GlanceResetCardData]] = [:]
    private static var keys: [String] = []
    private static var spreadCache: [String: [[GlanceResetCardData]]] = [:]

    static func pages(_ cards: [GlanceResetCardData], width: CGFloat, height: CGFloat) -> [GlanceResetCardData] {
        let key = "\(width)|\(height)|\(String(reflecting: cards).hashValue)"
        if let saved = cache[key] { return saved }
        var result: [GlanceResetCardData] = []
        for card in cards {
            let needsPartition = card.elements.contains { element in
                switch element {
                case let .chances(chances): return width < 250 && chances.count > 1
                case let .calendar(_, weeks, _): return CGFloat(weeks.count) * 9 > width - 46
                default: return false
                }
            }
            if !needsPartition && fits(card, width: width, height: height) { result.append(card); continue }
            var current = GlanceResetCardData(id: card.id + "|0", title: card.title, accent: card.accent, elements: [])
            var part = 0
            var continuation = card
            continuation.title = ""
            for element in card.elements {
                for piece in pieces(element, card: continuation, width: width, height: height) {
                    var candidate = current
                    candidate.elements.append(piece)
                    if !fits(candidate, width: width, height: height) && (!current.elements.isEmpty || !current.title.isEmpty) {
                        result.append(current)
                        part += 1
                        current = GlanceResetCardData(id: card.id + "|\(part)", title: "", accent: card.accent, elements: [piece])
                    } else {
                        current = candidate
                    }
                }
            }
            if !current.elements.isEmpty { result.append(current) }
        }
        cache[key] = result
        keys.append(key)
        if keys.count > 12 { cache.removeValue(forKey: keys.removeFirst()) }
        return result
    }

    static func spreads(_ fragments: [GlanceResetCardData], width: CGFloat, height: CGFloat) -> [[GlanceResetCardData]] {
        let key = "\(width)|\(height)|\(String(reflecting: fragments).hashValue)"
        if let saved = spreadCache[key] { return saved }
        var result: [[GlanceResetCardData]] = []
        var current: [GlanceResetCardData] = []
        var used: CGFloat = 0
        for fragment in fragments {
            let measured = measuredHeight(fragment, width: width)
            let required = measured + (current.isEmpty ? 0 : 8)
            if !current.isEmpty && used + required > height - 4 {
                result.append(current)
                current = [fragment]
                used = measured
            } else {
                current.append(fragment)
                used += required
            }
        }
        if !current.isEmpty { result.append(current) }
        if spreadCache.count >= 12 { spreadCache.removeAll(keepingCapacity: true) }
        spreadCache[key] = result
        return result
    }

    private static func measuredHeight(_ card: GlanceResetCardData, width: CGFloat) -> CGFloat {
        let view = GlanceResetCardView(card: card, availableWidth: width)
            .frame(width: width).fixedSize(horizontal: false, vertical: true)
        let controller = NSHostingController(rootView: view)
        return controller.sizeThatFits(in: CGSize(width: width, height: 10_000)).height
    }

    private static func fits(_ card: GlanceResetCardData, width: CGFloat, height: CGFloat) -> Bool {
        measuredHeight(card, width: width) <= height - 4
    }

    private static func pieces(_ element: GlanceResetElement, card: GlanceResetCardData, width: CGFloat, height: CGFloat) -> [GlanceResetElement] {
        var single = card
        single.elements = [element]
        if fits(single, width: width, height: height) {
            if case let .calendar(calendar, weeks, days) = element, CGFloat(weeks.count) * 9 > width - 46 {
                return calendarPieces(calendar, weeks: weeks, days: days, card: card, width: width, height: height)
            }
            if case let .chances(chances) = element, chances.count > 1 && width < 250 {
                return chances.flatMap { pieces(.chances([$0]), card: card, width: width, height: height) }
            }
            return [element]
        }
        switch element {
        case let .text(text, style):
            return split(text, card: card, width: width, height: height) { .text($0, style) }
        case let .badge(text):
            return split(text, card: card, width: width, height: height) { .badge($0) }
        case let .chances(chances):
            if chances.count > 1 {
                return chances.flatMap { pieces(.chances([$0]), card: card, width: width, height: height) }
            }
            return chances.flatMap { chance in
                pieces(.text(chance.percent, .value), card: card, width: width, height: height)
                    + [.meter(chance.fraction)]
                    + pieces(.text(chance.label, .secondary), card: card, width: width, height: height)
            }
        case let .calendar(calendar, weeks, days):
            return calendarPieces(calendar, weeks: weeks, days: days, card: card, width: width, height: height)
        case let .legend(legend):
            return [legend.regular, legend.banked, legend.today].map { .text($0, .secondary) }
        case let .rhythm(title, buckets):
            return [.text(title, .secondary)] + buckets.flatMap { pieces(.stat($0.label, String($0.count)), card: card, width: width, height: height) }
        case let .stat(label, value):
            return pieces(.text(label, .secondary), card: card, width: width, height: height)
                + pieces(.text(value, .heading), card: card, width: width, height: height)
        default:
            return [element]
        }
    }

    private static func split(_ text: String, card: GlanceResetCardData, width: CGFloat, height: CGFloat, make: (String) -> GlanceResetElement) -> [GlanceResetElement] {
        var remaining = text
        var result: [GlanceResetElement] = []
        while !remaining.isEmpty {
            let characters = Array(remaining)
            var low = 1
            var high = characters.count
            var best = 1
            while low <= high {
                let middle = (low + high) / 2
                var candidate = card
                candidate.elements = [make(String(characters.prefix(middle)))]
                if fits(candidate, width: width, height: height) { best = middle; low = middle + 1 }
                else { high = middle - 1 }
            }
            if best < characters.count, let space = characters.prefix(best).lastIndex(where: { $0.isWhitespace }), space > 0 {
                best = space + 1
            }
            result.append(make(String(characters.prefix(best))))
            remaining = String(characters.dropFirst(best))
        }
        return result
    }

    private static func calendarPieces(_ calendar: GlanceResetCalendar, weeks: Range<Int>, days: Range<Int>, card: GlanceResetCardData, width: CGFloat, height: CGFloat) -> [GlanceResetElement] {
        let count = max(1, Int((width - 46) / 10))
        var result: [GlanceResetElement] = []
        for first in stride(from: weeks.lowerBound, to: weeks.upperBound, by: count) {
            let range = first..<min(first + count, weeks.upperBound)
            var firstDay = days.lowerBound
            while firstDay < days.upperBound {
                var endDay = firstDay + 1
                while endDay < days.upperBound {
                    var candidate = card
                    candidate.elements = [.calendar(calendar, range, firstDay..<(endDay + 1))]
                    if !fits(candidate, width: width, height: height) { break }
                    endDay += 1
                }
                result.append(.calendar(calendar, range, firstDay..<endDay))
                firstDay = endDay
            }
        }
        return result
    }
}
