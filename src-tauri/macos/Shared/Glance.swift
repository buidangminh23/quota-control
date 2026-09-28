import AppKit
import Foundation
import SwiftUI

/// The readings as the popup last rendered them, shared by the Dynamic Island (pushed through the
/// bridge) and the desktop widgets (read from `glance.json`). Each surface carries the accounts and
/// metrics its Settings choose; every string is already localized and every value already follows
/// the Used/Left setting, so both surfaces show exactly what the popup shows. Keys added after the
/// first release decode with defaults, so a widget reading an older file still draws it.
struct GlanceDocument: Decodable, Equatable {
    var version: Int
    var generatedAt: Date
    var locale: String
    /// Settings → Time Format: `true` for 12-hour, `false` for 24-hour, absent to follow the locale.
    var hour12: Bool?
    var labels: GlanceLabels
    /// The open island's accounts.
    var providers: [GlanceProvider]
    var island: GlanceIsland
    var widget: GlanceWidgetContent
    /// The Codex free-reset tracker; absent while the Reset tab and reset notifications are both off.
    var resets: GlanceResets?
    var alert: GlanceAlert?

    static let supportedVersion = 1

    private enum CodingKeys: String, CodingKey {
        case version, generatedAt, locale, hour12, labels, providers, island, widget, resets, alert
    }

    init(
        version: Int,
        generatedAt: Date,
        locale: String,
        hour12: Bool?,
        labels: GlanceLabels,
        providers: [GlanceProvider],
        island: GlanceIsland,
        widget: GlanceWidgetContent,
        resets: GlanceResets? = nil,
        alert: GlanceAlert?
    ) {
        self.version = version
        self.generatedAt = generatedAt
        self.locale = locale
        self.hour12 = hour12
        self.labels = labels
        self.providers = providers
        self.island = island
        self.widget = widget
        self.resets = resets
        self.alert = alert
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        version = try container.decode(Int.self, forKey: .version)
        generatedAt = try container.decode(Date.self, forKey: .generatedAt)
        locale = try container.decode(String.self, forKey: .locale)
        hour12 = try container.decodeIfPresent(Bool.self, forKey: .hour12)
        labels = try container.decode(GlanceLabels.self, forKey: .labels)
        providers = try container.decode([GlanceProvider].self, forKey: .providers)
        island = try container.decode(GlanceIsland.self, forKey: .island)
        widget = try container.decodeIfPresent(GlanceWidgetContent.self, forKey: .widget)
            ?? GlanceWidgetContent(providers: providers, shows: .all, empty: labels.empty)
        resets = try? container.decodeIfPresent(GlanceResets.self, forKey: .resets)
        alert = try container.decodeIfPresent(GlanceAlert.self, forKey: .alert)
    }

    static func decode(_ data: Data) -> GlanceDocument? {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            guard let date = GlanceDates.parse(text) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Invalid date"))
            }
            return date
        }
        guard let document = try? decoder.decode(GlanceDocument.self, from: data),
              document.version == supportedVersion
        else {
            return nil
        }
        return document
    }

    var resolvedLocale: Locale { Locale(identifier: locale) }

    /// The island's accounts with something to show: readings, or a notice saying why there are none.
    var visibleProviders: [GlanceProvider] {
        providers.filter { !$0.metrics.isEmpty || $0.notice != nil }
    }

    /// The soonest reset still ahead of `now`, so a timeline can refresh when a limit comes back.
    func nextReset(after now: Date) -> Date? {
        (providers + widget.providers).flatMap(\.metrics).compactMap(\.resetsAt).filter { $0 > now }.min()
    }

    /// The document as the open island draws it: the reset tracker cut down to the island's parts.
    var forIsland: GlanceDocument {
        var copy = self
        copy.resets = resets?.showing(island.resetParts)
        return copy
    }

    /// The document as the widgets draw it: the reset tracker cut down to the widget's parts.
    var forWidget: GlanceDocument {
        var copy = self
        copy.resets = resets?.showing(widget.resetParts)
        return copy
    }

    /// The moments after `now` when something drawn from the reset tracker changes on its own: the
    /// announced reset's countdown ends or its row goes away.
    func resetMoments(after now: Date) -> [Date] {
        guard let upcoming = resets?.upcoming else { return [] }
        return [upcoming.countdown?.at, upcoming.hideAt].compactMap { $0 }.filter { $0 > now }
    }
}

/// A limit coming back: one metric of one account and its reset time.
struct GlanceUpcomingLimit: Identifiable, Equatable {
    var provider: GlanceProvider
    var metric: GlanceMetric
    var at: Date

    var id: String { "\(provider.id)|\(metric.id)" }

    /// Every metric of `providers` with a reset still ahead of `now`, soonest first.
    static func list(_ providers: [GlanceProvider], now: Date) -> [GlanceUpcomingLimit] {
        var limits: [GlanceUpcomingLimit] = []
        for provider in providers {
            for metric in provider.metrics {
                if let at = metric.resetsAt, at > now {
                    limits.append(GlanceUpcomingLimit(provider: provider, metric: metric, at: at))
                }
            }
        }
        return limits.sorted { left, right in
            left.at == right.at ? left.id < right.id : left.at < right.at
        }
    }
}

struct GlanceLabels: Decodable, Equatable {
    var title: String
    var empty: String
    var updated: String
    var resetsIn: String
    var resetting: String
    var open: String
    var notRunning: String
    var noData: String
    var more: String
    var units: GlanceUnits
    /// What a reset surface says while the tracker is off.
    var resetsOff: String
    /// The heading of the next limits to come back, and what it says when none has a reset time.
    var upcoming: String
    var upcomingEmpty: String
    /// The open island's tab names, as the popup's tabs read.
    var tabs: GlanceTabLabels

    private enum CodingKeys: String, CodingKey {
        case title, empty, updated, resetsIn, resetting, open, notRunning, noData, more, units, resetsOff, upcoming, upcomingEmpty, tabs
    }

    init(
        title: String,
        empty: String,
        updated: String,
        resetsIn: String,
        resetting: String,
        open: String,
        notRunning: String,
        noData: String,
        more: String,
        units: GlanceUnits,
        resetsOff: String = "",
        upcoming: String = "",
        upcomingEmpty: String = "",
        tabs: GlanceTabLabels = .fallback
    ) {
        self.title = title
        self.empty = empty
        self.updated = updated
        self.resetsIn = resetsIn
        self.resetting = resetting
        self.open = open
        self.notRunning = notRunning
        self.noData = noData
        self.more = more
        self.units = units
        self.resetsOff = resetsOff
        self.upcoming = upcoming
        self.upcomingEmpty = upcomingEmpty
        self.tabs = tabs
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        title = try container.decode(String.self, forKey: .title)
        empty = try container.decode(String.self, forKey: .empty)
        updated = try container.decode(String.self, forKey: .updated)
        resetsIn = try container.decode(String.self, forKey: .resetsIn)
        resetting = try container.decode(String.self, forKey: .resetting)
        open = try container.decode(String.self, forKey: .open)
        notRunning = try container.decode(String.self, forKey: .notRunning)
        noData = try container.decodeIfPresent(String.self, forKey: .noData) ?? "—"
        more = try container.decodeIfPresent(String.self, forKey: .more) ?? ""
        units = try container.decode(GlanceUnits.self, forKey: .units)
        resetsOff = try container.decodeIfPresent(String.self, forKey: .resetsOff) ?? ""
        upcoming = try container.decodeIfPresent(String.self, forKey: .upcoming) ?? ""
        upcomingEmpty = try container.decodeIfPresent(String.self, forKey: .upcomingEmpty) ?? ""
        tabs = (try? container.decodeIfPresent(GlanceTabLabels.self, forKey: .tabs)) ?? .fallback
    }
}

/// The names of the open island's tabs.
struct GlanceTabLabels: Decodable, Equatable {
    var quota: String
    var resets: String
    var upcoming: String

    static let fallback = GlanceTabLabels(quota: "Limits", resets: "Codex Resets", upcoming: "Coming Back")

    func name(_ view: GlanceView) -> String {
        switch view {
        case .quota: return quota
        case .resets: return resets
        case .upcoming: return upcoming
        }
    }
}

/// A view a glance surface can show, named after the popup tab it mirrors.
enum GlanceView: String, Decodable, Equatable, Hashable, CaseIterable {
    case quota
    case resets
    case upcoming
}

/// How the open island shows several views: one at a time behind a tab bar, or stacked.
enum IslandArrangement: String, Decodable, Equatable {
    case tabs
    case stacked
}

/// The parts of the reset tracker a surface shows; a part missing from an older document shows.
struct GlanceResetParts: Decodable, Equatable {
    var next = true
    var latest = true
    var chances = true
    var wait = true
    var calendar = true
    var rhythm = true

    static let all = GlanceResetParts()

    init() {}

    private enum CodingKeys: String, CodingKey {
        case next, latest, chances, wait, calendar, rhythm
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        next = (try? container.decodeIfPresent(Bool.self, forKey: .next)) ?? true
        latest = (try? container.decodeIfPresent(Bool.self, forKey: .latest)) ?? true
        chances = (try? container.decodeIfPresent(Bool.self, forKey: .chances)) ?? true
        wait = (try? container.decodeIfPresent(Bool.self, forKey: .wait)) ?? true
        calendar = (try? container.decodeIfPresent(Bool.self, forKey: .calendar)) ?? true
        rhythm = (try? container.decodeIfPresent(Bool.self, forKey: .rhythm)) ?? true
    }
}

/// Decodes a list of views, keeping the known ones once each in order.
private func decodeViews<Key: CodingKey>(_ container: KeyedDecodingContainer<Key>, forKey key: Key) -> [GlanceView]? {
    guard let raw = try? container.decodeIfPresent([String].self, forKey: key) else { return nil }
    var views: [GlanceView] = []
    for name in raw {
        if let view = GlanceView(rawValue: name), !views.contains(view) { views.append(view) }
    }
    return views.isEmpty ? nil : views
}

/// Suffixes for countdowns, so the island words them like the popup (`4 ngày 3 giờ`, `4d 3h`).
struct GlanceUnits: Decodable, Equatable {
    var day: String
    var hour: String
    var minute: String
}

/// Which parts of an account a surface shows.
struct GlanceShows: Decodable, Equatable {
    var account: Bool
    var plan: Bool
    var resets: Bool

    static let all = GlanceShows(account: true, plan: true, resets: true)
}

/// How the closed island shows a reading beside the notch.
enum IslandStyle: String, Decodable, Equatable {
    case percent
    case ring
    case bar
}

struct GlanceIsland: Decodable, Equatable {
    var enabled: Bool
    /// Whether a new alert opens the island; alerts travel either way so they are marked seen.
    var alerts: Bool
    var style: IslandStyle
    /// The readings beside the notch, left then right: one account with one metric each.
    var wings: [GlanceProvider]
    var expandOnHover: Bool
    var shows: GlanceShows
    var empty: String?
    /// What the open island lists, top to bottom.
    var sections: GlanceIslandSections
    /// The chosen views in order; an older document lists its switched-on sections.
    var tabs: [GlanceView]
    var arrangement: IslandArrangement
    var resetParts: GlanceResetParts
    /// The most limits coming back listed; `0` for as many as fit.
    var upcomingLimit: Int

    private enum CodingKeys: String, CodingKey {
        case enabled, alerts, style, wings, expandOnHover, shows, empty, sections, tabs, arrangement, resetParts, upcomingLimit
    }

    init(
        enabled: Bool,
        alerts: Bool = true,
        style: IslandStyle = .percent,
        wings: [GlanceProvider] = [],
        expandOnHover: Bool = true,
        shows: GlanceShows = .all,
        empty: String? = nil,
        sections: GlanceIslandSections = .quotaOnly,
        tabs: [GlanceView]? = nil,
        arrangement: IslandArrangement = .stacked,
        resetParts: GlanceResetParts = .all,
        upcomingLimit: Int = 6
    ) {
        self.enabled = enabled
        self.alerts = alerts
        self.style = style
        self.wings = wings
        self.expandOnHover = expandOnHover
        self.shows = shows
        self.empty = empty
        self.sections = sections
        self.tabs = tabs ?? sections.views
        self.arrangement = arrangement
        self.resetParts = resetParts
        self.upcomingLimit = upcomingLimit
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        enabled = try container.decode(Bool.self, forKey: .enabled)
        alerts = try container.decodeIfPresent(Bool.self, forKey: .alerts) ?? true
        style = (try? container.decodeIfPresent(IslandStyle.self, forKey: .style)) ?? .percent
        wings = try container.decodeIfPresent([GlanceProvider].self, forKey: .wings) ?? []
        expandOnHover = try container.decodeIfPresent(Bool.self, forKey: .expandOnHover) ?? true
        shows = try container.decodeIfPresent(GlanceShows.self, forKey: .shows) ?? .all
        empty = try container.decodeIfPresent(String.self, forKey: .empty)
        sections = (try? container.decodeIfPresent(GlanceIslandSections.self, forKey: .sections)) ?? .quotaOnly
        tabs = decodeViews(container, forKey: .tabs) ?? sections.views
        arrangement = (try? container.decodeIfPresent(IslandArrangement.self, forKey: .arrangement)) ?? .stacked
        resetParts = (try? container.decodeIfPresent(GlanceResetParts.self, forKey: .resetParts)) ?? .all
        upcomingLimit = max(0, (try? container.decodeIfPresent(Int.self, forKey: .upcomingLimit)) ?? 6)
    }
}

/// The parts of the open island, each switched in Settings.
struct GlanceIslandSections: Decodable, Equatable {
    var quota: Bool
    var resets: Bool
    var upcoming: Bool

    static let quotaOnly = GlanceIslandSections(quota: true, resets: false, upcoming: false)

    /// The switched-on sections in drawing order.
    var views: [GlanceView] {
        let views = GlanceView.allCases.filter { view in
            switch view {
            case .quota: return quota
            case .resets: return resets
            case .upcoming: return upcoming
            }
        }
        return views.isEmpty ? [.quota] : views
    }
}

/// What the desktop widgets list.
struct GlanceWidgetContent: Decodable, Equatable {
    var providers: [GlanceProvider]
    var shows: GlanceShows
    var empty: String
    /// The Overview widget's parts, in order.
    var tabs: [GlanceView]
    var resetParts: GlanceResetParts
    /// The most limits coming back listed; `0` for as many as fit.
    var upcomingLimit: Int

    private enum CodingKeys: String, CodingKey {
        case providers, shows, empty, tabs, resetParts, upcomingLimit
    }

    init(providers: [GlanceProvider], shows: GlanceShows, empty: String, tabs: [GlanceView] = GlanceView.allCases, resetParts: GlanceResetParts = .all, upcomingLimit: Int = 0) {
        self.providers = providers
        self.shows = shows
        self.empty = empty
        self.tabs = tabs
        self.resetParts = resetParts
        self.upcomingLimit = upcomingLimit
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providers = try container.decode([GlanceProvider].self, forKey: .providers)
        shows = try container.decodeIfPresent(GlanceShows.self, forKey: .shows) ?? .all
        empty = try container.decodeIfPresent(String.self, forKey: .empty) ?? ""
        tabs = decodeViews(container, forKey: .tabs) ?? GlanceView.allCases
        resetParts = (try? container.decodeIfPresent(GlanceResetParts.self, forKey: .resetParts)) ?? .all
        upcomingLimit = max(0, (try? container.decodeIfPresent(Int.self, forKey: .upcomingLimit)) ?? 0)
    }

    func has(_ view: GlanceView) -> Bool { tabs.contains(view) }

    /// Accounts with something to show: readings, or a notice saying why there are none.
    var visibleProviders: [GlanceProvider] {
        providers.filter { !$0.metrics.isEmpty || $0.notice != nil }
    }
}

struct GlanceProvider: Decodable, Equatable, Identifiable {
    var id: String
    /// The card heading: the brand for an account named by its email, the account title otherwise.
    var name: String
    var account: String?
    var plan: String?
    /// Why an account without readings shows none (signed out, session expired).
    var notice: String?
    var brand: String
    var color: String
    var mark: GlanceMark?
    var metrics: [GlanceMetric]

    var tint: Color { Color(glanceHex: color) ?? .white }
}

/// A provider's logo: single-color path data drawn in the provider's tint, and for a brand whose
/// official logo is several colors (Antigravity, Gemini…) that logo as a base64 PNG, drawn instead.
struct GlanceMark: Decodable, Equatable {
    var box: [Double]
    var paths: [GlancePath]
    var art: String?

    private enum CodingKeys: String, CodingKey {
        case box, paths, art
    }

    init(box: [Double], paths: [GlancePath], art: String? = nil) {
        self.box = box
        self.paths = paths
        self.art = art
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        box = try container.decodeIfPresent([Double].self, forKey: .box) ?? []
        paths = try container.decodeIfPresent([GlancePath].self, forKey: .paths) ?? []
        art = try container.decodeIfPresent(String.self, forKey: .art)
    }

    /// The color logo, decoded once per distinct picture.
    var artImage: NSImage? {
        guard let art else { return nil }
        return GlanceMarkArt.image(for: art)
    }
}

/// Decoded color logos, shared by every view that draws the same picture.
enum GlanceMarkArt {
    private static let cache = NSCache<NSString, NSImage>()

    static func image(for base64: String) -> NSImage? {
        let key = base64 as NSString
        if let image = cache.object(forKey: key) { return image }
        guard let data = Data(base64Encoded: base64), let image = NSImage(data: data) else { return nil }
        cache.setObject(image, forKey: key)
        return image
    }
}

struct GlancePath: Decodable, Equatable {
    var d: String
    var evenOdd: Bool?
}

enum GlanceSeverity: String, Decodable, Equatable {
    case normal
    case warning
    case critical
    case none
}

struct GlanceMetric: Decodable, Equatable, Identifiable {
    var id: String
    var label: String
    /// The strip reading: `42%` for a meter, the compact value otherwise.
    var value: String
    /// The row headline, e.g. `58% left` / `Còn 58%`.
    var headline: String
    /// Meter fill 0...1 following Used/Left; `nil` for a metric without a limit.
    var fraction: Double?
    var severity: GlanceSeverity
    var resetsAt: Date?
    /// Text shown where no reset countdown applies (`Not started`, a plan badge, `No data`).
    var detail: String?
    /// A value that moves with the clock, drawn in place of `value` (island wings).
    var countdown: GlanceCountdown?

    /// `value`, or the countdown's words at `now`, its span as short as `GlanceFormat.shortSpan`.
    func liveValue(now: Date, units: GlanceUnits) -> String {
        countdown?.text(now: now, units: units, short: true) ?? value
    }
}

/// Words around a moving span of time (see `GlanceCountdown` in `src/model/glance.ts`): `text` with
/// `{d}` replaced by the time left until `at`, or gone by since it when `since`; once a countdown
/// has passed, `after`.
struct GlanceCountdown: Decodable, Equatable {
    var at: Date
    var text: String
    var since: Bool?
    var after: String?

    static let placeholder = "{d}"

    func passed(_ now: Date) -> Bool { since != true && at <= now }

    func text(now: Date, units: GlanceUnits, short: Bool = false) -> String {
        if passed(now), let after { return after }
        let from = since == true ? at : now
        let to = since == true ? now : at
        let span = short
            ? GlanceFormat.shortSpan(from: from, to: to, units: units)
            : GlanceFormat.span(from: from, to: to, units: units)
        return text.replacingOccurrences(of: Self.placeholder, with: span)
    }

    /// `text` split around `{d}`, for a widget that draws the span as a live `Text(date, style:)`.
    var parts: (before: String, after: String) {
        guard let range = text.range(of: Self.placeholder) else { return (text, "") }
        return (String(text[..<range.lowerBound]), String(text[range.upperBound...]))
    }
}

/// The Codex free-reset tracker (see `GlanceResets` in `src/model/glance.ts`).
struct GlanceResets: Decodable, Equatable {
    var title: String
    var source: String
    var brand: String
    var color: String
    var mark: GlanceMark?
    var stale: String?
    var upcoming: GlanceUpcomingReset?
    var latest: GlanceLatestReset?
    var forecastTitle: String
    var forecast: [GlanceResetChance]
    var forecastNote: String
    var wait: String?
    var median: String?
    var calendar: GlanceResetCalendar?
    var rhythm: GlanceResetRhythm?

    var tint: Color { Color(glanceHex: color) ?? .white }

    /// The announced reset while it is still to be shown at `now`.
    func upcoming(at now: Date) -> GlanceUpcomingReset? {
        guard let upcoming, upcoming.hideAt > now else { return nil }
        return upcoming
    }

    func chance(days: Int) -> GlanceResetChance? {
        forecast.first { $0.days == days }
    }

    /// The tracker with only `parts`: the announced reset, the last one, the chances, the wait
    /// with its median, the calendar and the rhythm each go when switched off.
    func showing(_ parts: GlanceResetParts) -> GlanceResets {
        var copy = self
        if !parts.next { copy.upcoming = nil }
        if !parts.latest { copy.latest = nil }
        if !parts.chances {
            copy.forecast = []
            copy.forecastNote = ""
        }
        if !parts.wait {
            copy.wait = nil
            copy.median = nil
        }
        if !parts.calendar { copy.calendar = nil }
        if !parts.rhythm { copy.rhythm = nil }
        return copy
    }
}

struct GlanceUpcomingReset: Decodable, Equatable {
    var title: String
    var tone: GlanceResetTone
    var countdown: GlanceCountdown?
    var value: String?
    var caption: String
    var captionAfter: String?
    var note: String?
    var hideAt: Date
    var chancePercent: Int?

    /// The countdown's words (or the fixed value) and the caption under them, at `now`.
    func lines(now: Date, units: GlanceUnits) -> (value: String, caption: String, awaiting: Bool) {
        if let countdown {
            let awaiting = countdown.passed(now)
            return (countdown.text(now: now, units: units), awaiting ? (captionAfter ?? caption) : caption, awaiting)
        }
        return (value ?? "", caption, false)
    }
}

enum GlanceResetTone: String, Decodable, Equatable {
    case positive
    case notice
}

struct GlanceLatestReset: Decodable, Equatable {
    var at: Date
    var kind: String
    var label: String
    var kindLabel: String
    var since: GlanceCountdown
    var when: String
}

struct GlanceResetChance: Decodable, Equatable, Identifiable {
    var days: Int
    var percent: Int
    var label: String

    var id: Int { days }
    var fraction: Double { min(max(Double(percent) / 100, 0), 1) }
}

struct GlanceResetCalendar: Decodable, Equatable {
    var title: String
    var weeks: Int
    var cells: String
    var today: Int
    var weekdays: [String]
    var months: [GlanceResetMonth]
    var legend: GlanceResetLegend

    enum Cell: Equatable {
        case none
        case regular
        case banked
        case future
    }

    /// The cells as rows of seven, oldest week first, keeping only the newest `last` weeks.
    func weekRows(last: Int? = nil) -> [[Cell]] {
        let all = Array(cells).map { character -> Cell in
            switch character {
            case "r": return .regular
            case "b": return .banked
            case "-": return .future
            default: return .none
            }
        }
        let rows = stride(from: 0, to: all.count, by: 7).map { Array(all[$0..<min($0 + 7, all.count)]) }
        guard let last, last < rows.count else { return rows }
        return Array(rows.suffix(last))
    }
}

struct GlanceResetMonth: Decodable, Equatable {
    var week: Int
    var label: String
}

struct GlanceResetLegend: Decodable, Equatable {
    var regular: String
    var banked: String
    var today: String
}

struct GlanceResetRhythm: Decodable, Equatable {
    var title: String
    var total: Int
    var weekdayTitle: String
    var weekdays: [GlanceResetBucket]
    var hourTitle: String
    var hours: [GlanceResetBucket]
}

struct GlanceResetBucket: Decodable, Equatable {
    var label: String
    var count: Int
}

struct GlanceAlert: Decodable, Equatable, Identifiable {
    var id: String
    var title: String
    var body: String
    var brand: String?
    var severity: GlanceSeverity
}

enum GlanceDates {
    private static let fractional: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let plain: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    static func parse(_ text: String) -> Date? {
        fractional.date(from: text) ?? plain.date(from: text)
    }
}

enum GlancePalette {
    static func fill(_ severity: GlanceSeverity, onDark: Bool) -> Color {
        switch severity {
        case .normal:
            return onDark ? Color(red: 0.04, green: 0.52, blue: 1.0) : .blue
        case .warning:
            return onDark ? Color(red: 1.0, green: 0.84, blue: 0.04) : .yellow
        case .critical:
            return onDark ? Color(red: 1.0, green: 0.27, blue: 0.23) : .red
        case .none:
            return .secondary
        }
    }

    static func text(_ severity: GlanceSeverity, onDark: Bool) -> Color {
        switch severity {
        case .warning, .critical:
            return fill(severity, onDark: onDark)
        case .normal, .none:
            return onDark ? .white : .primary
        }
    }
}

enum GlanceFormat {
    /// A span in the popup's words, as `countdown` words the time left: `4 ngày 3 giờ`, `2h 5m`.
    static func span(from start: Date, to end: Date, units: GlanceUnits) -> String {
        countdown(to: max(end, start.addingTimeInterval(1)), from: start, units: units)
    }

    /// The span in its largest unit only (`3 ngày`, `5h`, `12m`), for a reading beside the notch.
    static func shortSpan(from start: Date, to end: Date, units: GlanceUnits) -> String {
        let minutes = max(1, Int((end.timeIntervalSince(start) / 60).rounded(.up)))
        if minutes >= 24 * 60 { return "\(minutes / (24 * 60))\(units.day)" }
        if minutes >= 60 { return "\(minutes / 60)\(units.hour)" }
        return "\(minutes)\(units.minute)"
    }

    /// The popup's countdown (`format.duration`): days and hours, hours and minutes, or minutes,
    /// rounding up to the next whole minute.
    static func countdown(to date: Date, from now: Date, units: GlanceUnits) -> String {
        let minutes = max(1, Int((date.timeIntervalSince(now) / 60).rounded(.up)))
        let days = minutes / (24 * 60)
        let hours = (minutes % (24 * 60)) / 60
        let rest = minutes % 60
        if days > 0 { return "\(days)\(units.day) \(hours)\(units.hour)" }
        if hours > 0 { return rest > 0 ? "\(hours)\(units.hour) \(rest)\(units.minute)" : "\(hours)\(units.hour)" }
        return "\(rest)\(units.minute)"
    }

    /// `19:42` or `7:42 PM`: the 12- or 24-hour clock Settings chose, else the locale's own.
    static func time(_ date: Date, locale: Locale, hour12: Bool? = nil) -> String {
        let formatter = DateFormatter()
        formatter.locale = locale
        if let hour12 {
            formatter.setLocalizedDateFormatFromTemplate(hour12 ? "hmm a" : "HHmm")
        } else {
            formatter.dateStyle = .none
            formatter.timeStyle = .short
        }
        return formatter.string(from: date)
    }
}

extension Color {
    init?(glanceHex text: String) {
        var hex = text.trimmingCharacters(in: .whitespaces)
        if hex.hasPrefix("#") { hex.removeFirst() }
        guard hex.count == 6, let value = UInt32(hex, radix: 16) else { return nil }
        self.init(
            red: Double((value >> 16) & 0xFF) / 255,
            green: Double((value >> 8) & 0xFF) / 255,
            blue: Double(value & 0xFF) / 255
        )
    }
}
