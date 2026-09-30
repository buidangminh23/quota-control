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
    /// Settings → Theme when it is not System (`light` or `dark`): every widget draws in it, as the
    /// whole popup does.
    var theme: String?
    /// Settings → Reduce Animations, while it is on: the island and widgets stop animating.
    var reduceMotion: Bool? = nil
    /// Settings → Density: `compact` for Compact, absent for Default.
    var density: String? = nil
    /// Settings → Reset Times: `absolute` for Exact Time, absent for Countdown.
    var resetDisplay: String? = nil
    /// Settings → Always Show Pacing, while it is on.
    var alwaysShowPacing: Bool? = nil
    /// Settings → Used/Left: `used` for Used, absent for Left.
    var displayMode: String? = nil
    var labels: GlanceLabels
    /// The open island's accounts.
    var providers: [GlanceProvider]
    var island: GlanceIsland
    var widget: GlanceWidgetContent
    /// The Codex free-reset tracker; absent while the Reset tab and reset notifications are both off.
    /// In a surface's copy (`forIsland`, `forWidget`) it is the tracker that surface chose.
    var resets: GlanceResets?
    /// What a reset view says in place of the Codex tracker while it is on but has nothing yet; in a
    /// surface's copy, for the tracker that surface chose.
    var resetsPending: GlanceResetsPending?
    /// The Claude reset tracker, sent while the island or the widget chose it; absent otherwise, and
    /// while the Reset tab and Claude reset notifications are both off.
    var claudeResets: GlanceResets?
    /// `resetsPending` for the Claude tracker.
    var claudeResetsPending: GlanceResetsPending?
    /// The pictures before the accounts' reset rows, as data URLs by lowercase handle.
    var avatars: [String: String]? = nil
    var alert: GlanceAlert?

    static let supportedVersion = 1

    private enum CodingKeys: String, CodingKey {
        case version, generatedAt, locale, hour12, theme, reduceMotion, density, resetDisplay, alwaysShowPacing, displayMode, labels, providers, island, widget, resets, resetsPending, claudeResets, claudeResetsPending, avatars, alert
    }

    init(
        version: Int,
        generatedAt: Date,
        locale: String,
        hour12: Bool?,
        theme: String? = nil,
        labels: GlanceLabels,
        providers: [GlanceProvider],
        island: GlanceIsland,
        widget: GlanceWidgetContent,
        resets: GlanceResets? = nil,
        resetsPending: GlanceResetsPending? = nil,
        claudeResets: GlanceResets? = nil,
        claudeResetsPending: GlanceResetsPending? = nil,
        alert: GlanceAlert?
    ) {
        self.version = version
        self.generatedAt = generatedAt
        self.locale = locale
        self.hour12 = hour12
        self.theme = theme
        self.labels = labels
        self.providers = providers
        self.island = island
        self.widget = widget
        self.resets = resets
        self.resetsPending = resetsPending
        self.claudeResets = claudeResets
        self.claudeResetsPending = claudeResetsPending
        self.alert = alert
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        version = try container.decode(Int.self, forKey: .version)
        generatedAt = try container.decode(Date.self, forKey: .generatedAt)
        locale = try container.decode(String.self, forKey: .locale)
        hour12 = try container.decodeIfPresent(Bool.self, forKey: .hour12)
        theme = try? container.decodeIfPresent(String.self, forKey: .theme)
        reduceMotion = try? container.decodeIfPresent(Bool.self, forKey: .reduceMotion)
        density = try? container.decodeIfPresent(String.self, forKey: .density)
        resetDisplay = try? container.decodeIfPresent(String.self, forKey: .resetDisplay)
        alwaysShowPacing = try? container.decodeIfPresent(Bool.self, forKey: .alwaysShowPacing)
        displayMode = try? container.decodeIfPresent(String.self, forKey: .displayMode)
        labels = try container.decode(GlanceLabels.self, forKey: .labels)
        providers = try container.decode([GlanceProvider].self, forKey: .providers)
        island = try container.decode(GlanceIsland.self, forKey: .island)
        widget = try container.decodeIfPresent(GlanceWidgetContent.self, forKey: .widget)
            ?? GlanceWidgetContent(providers: providers, shows: .all, empty: labels.empty)
        resets = try? container.decodeIfPresent(GlanceResets.self, forKey: .resets)
        resetsPending = try? container.decodeIfPresent(GlanceResetsPending.self, forKey: .resetsPending)
        claudeResets = try? container.decodeIfPresent(GlanceResets.self, forKey: .claudeResets)
        claudeResetsPending = try? container.decodeIfPresent(GlanceResetsPending.self, forKey: .claudeResetsPending)
        avatars = try? container.decodeIfPresent([String: String].self, forKey: .avatars)
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

    /// Whether the readings are worded in Vietnamese, for the few words a surface adds itself.
    var isVietnamese: Bool { locale.lowercased().hasPrefix("vi") }

    /// The reset view's name on a surface showing the Claude tracker: the popup's word for it, else
    /// the tracker's title, else the same words in the document's language.
    var claudeResetsTitle: String {
        labels.claudeResetsTab ?? claudeResets?.title ?? (isVietnamese ? "Reset Claude" : "Claude Resets")
    }

    /// The appearance a widget draws in: the app's theme when it forces one, else `nil` to follow
    /// the Mac. A document from before the theme was its own key carries it on its reset trackers.
    var forcedScheme: ColorScheme? {
        switch theme ?? resets?.theme ?? claudeResets?.theme {
        case "light": return .light
        case "dark": return .dark
        default: return nil
        }
    }

    /// Whether the popup's Reduce Animations is on; a surface also stops animating for the Mac's own
    /// Reduce Motion.
    var reducesMotion: Bool { reduceMotion == true }

    /// Whether the popup is in Compact density, which the open island follows.
    var isCompact: Bool { density == "compact" }

    /// The island's accounts with something to show: readings, or a notice saying why there are none.
    var visibleProviders: [GlanceProvider] {
        providers.filter { !$0.metrics.isEmpty || $0.notice != nil }
    }

    /// The soonest reset still ahead of `now`, so a timeline can refresh when a limit comes back.
    func nextReset(after now: Date) -> Date? {
        (providers + widget.providers).flatMap(\.metrics).compactMap(\.resetsAt).filter { $0 > now }.min()
    }

    /// The document as the open island draws it: the reset tracker the island chose, cut down to its
    /// parts.
    var forIsland: GlanceDocument {
        showing(island.resetsProvider, parts: island.resetParts)
    }

    /// The document as the widgets draw it: the reset tracker the widget chose, cut down to its parts.
    var forWidget: GlanceDocument {
        showing(widget.resetsProvider, parts: widget.resetParts)
    }

    /// The document with `resets` as `provider`'s tracker showing only `parts`; for Claude the reset
    /// view is named after the Claude tracker and, while it has nothing, says why in its own words.
    private func showing(_ provider: GlanceResetsProvider, parts: GlanceResetParts) -> GlanceDocument {
        var copy = self
        switch provider {
        case .codex:
            copy.resets = resets?.showing(parts)
        case .claude:
            copy.resets = claudeResets?.showing(parts)
            copy.resetsPending = claudeResetsPending
            copy.labels.tabs.resets = claudeResetsTitle
            if let off = labels.claudeResetsOff { copy.labels.resetsOff = off }
        }
        return copy
    }

    /// The moments after `now` when something drawn from `resets` (in a surface's copy, the tracker
    /// it chose) changes on its own: an announced or banked reset's countdown ends or its row goes
    /// away, or the last reset's age moves on.
    func resetMoments(after now: Date) -> [Date] {
        guard let resets else { return [] }
        var moments: [Date] = []
        if let upcoming = resets.upcoming {
            moments += [upcoming.countdown?.at, upcoming.hideAt].compactMap { $0 }
        }
        if let presentation = resets.presentation {
            moments += presentation.statuses.flatMap { [$0.hideAt, $0.dueCountdown?.at].compactMap { $0 } }
            if let latest = presentation.latest {
                let elapsed = max(0, now.timeIntervalSince(latest.at))
                let unit: TimeInterval = elapsed < 3600 ? 60 : (elapsed < 86400 ? 3600 : 86400)
                moments.append(latest.at.addingTimeInterval((floor(elapsed / unit) + 1) * unit))
            }
        }
        return Array(Set(moments.filter { $0 > now })).sorted()
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
    /// A limit's reset text in its last five minutes (`Sắp đặt lại`); a document from before it counts
    /// down to the end and then says `resetting`.
    var resetsSoon: String? = nil
    /// Countdown: the line under a limit's countdown, `{at}` standing for its clock time and day
    /// (`Hồi lại lúc {at}`).
    var restoresAt: String? = nil
    /// Exact Time: a limit's reset text, `Đặt lại lúc {t} hôm nay`, worded like the day words.
    var resetAbsolute: GlanceDayWords? = nil
    /// The pace notes' words, sent while a limit carries a pace.
    var pace: GlancePaceWords? = nil
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
    /// The reset view's name on a surface showing the Claude tracker; sent while the island or the
    /// widget chose it.
    var claudeResetsTab: String?
    /// What that view says while the Claude tracker is off, naming the Claude reset notifications;
    /// sent with `claudeResetsTab`.
    var claudeResetsOff: String?
    /// The plan-period corner's words, sent while an account the island or the widget lists has a
    /// plan period.
    var planTerm: GlancePlanTermWords?
    /// A reset's clock time with its day, for the reset rows and the limits coming back; a document
    /// from before them names the clock time alone.
    var days: GlanceDayWords?

    private enum CodingKeys: String, CodingKey {
        case title, empty, updated, resetsIn, resetting, resetsSoon, restoresAt, resetAbsolute, pace, open, notRunning, noData, more, units, resetsOff, upcoming, upcomingEmpty, tabs, claudeResetsTab, claudeResetsOff, planTerm, days
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
        tabs: GlanceTabLabels = .fallback,
        claudeResetsTab: String? = nil,
        claudeResetsOff: String? = nil,
        planTerm: GlancePlanTermWords? = nil,
        days: GlanceDayWords? = nil
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
        self.claudeResetsTab = claudeResetsTab
        self.claudeResetsOff = claudeResetsOff
        self.planTerm = planTerm
        self.days = days
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        title = try container.decode(String.self, forKey: .title)
        empty = try container.decode(String.self, forKey: .empty)
        updated = try container.decode(String.self, forKey: .updated)
        resetsIn = try container.decode(String.self, forKey: .resetsIn)
        resetting = try container.decode(String.self, forKey: .resetting)
        resetsSoon = try? container.decodeIfPresent(String.self, forKey: .resetsSoon)
        restoresAt = try? container.decodeIfPresent(String.self, forKey: .restoresAt)
        resetAbsolute = try? container.decodeIfPresent(GlanceDayWords.self, forKey: .resetAbsolute)
        pace = try? container.decodeIfPresent(GlancePaceWords.self, forKey: .pace)
        open = try container.decode(String.self, forKey: .open)
        notRunning = try container.decode(String.self, forKey: .notRunning)
        noData = try container.decodeIfPresent(String.self, forKey: .noData) ?? "—"
        more = try container.decodeIfPresent(String.self, forKey: .more) ?? ""
        units = try container.decode(GlanceUnits.self, forKey: .units)
        resetsOff = try container.decodeIfPresent(String.self, forKey: .resetsOff) ?? ""
        upcoming = try container.decodeIfPresent(String.self, forKey: .upcoming) ?? ""
        upcomingEmpty = try container.decodeIfPresent(String.self, forKey: .upcomingEmpty) ?? ""
        tabs = (try? container.decodeIfPresent(GlanceTabLabels.self, forKey: .tabs)) ?? .fallback
        claudeResetsTab = try? container.decodeIfPresent(String.self, forKey: .claudeResetsTab)
        claudeResetsOff = try? container.decodeIfPresent(String.self, forKey: .claudeResetsOff)
        planTerm = try? container.decodeIfPresent(GlancePlanTermWords.self, forKey: .planTerm)
        days = try? container.decodeIfPresent(GlanceDayWords.self, forKey: .days)
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

/// Whose reset tracker a surface draws: Codex's (codex-resets.com) or Claude's (claude-resets.com).
/// A document from an older popup, or with a value this version does not know, means Codex.
enum GlanceResetsProvider: String, Decodable, Equatable {
    case codex
    case claude
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
    /// Whose reset tracker the reset view draws.
    var resetsProvider: GlanceResetsProvider

    private enum CodingKeys: String, CodingKey {
        case enabled, alerts, style, wings, expandOnHover, shows, empty, sections, tabs, arrangement, resetParts, upcomingLimit, resetsProvider
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
        upcomingLimit: Int = 6,
        resetsProvider: GlanceResetsProvider = .codex
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
        self.resetsProvider = resetsProvider
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
        resetsProvider = (try? container.decodeIfPresent(GlanceResetsProvider.self, forKey: .resetsProvider)) ?? .codex
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
    /// Whose reset tracker the reset widgets and the Overview's reset part draw.
    var resetsProvider: GlanceResetsProvider

    private enum CodingKeys: String, CodingKey {
        case providers, shows, empty, tabs, resetParts, upcomingLimit, resetsProvider
    }

    init(
        providers: [GlanceProvider],
        shows: GlanceShows,
        empty: String,
        tabs: [GlanceView] = GlanceView.allCases,
        resetParts: GlanceResetParts = .all,
        upcomingLimit: Int = 0,
        resetsProvider: GlanceResetsProvider = .codex
    ) {
        self.providers = providers
        self.shows = shows
        self.empty = empty
        self.tabs = tabs
        self.resetParts = resetParts
        self.upcomingLimit = upcomingLimit
        self.resetsProvider = resetsProvider
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providers = try container.decode([GlanceProvider].self, forKey: .providers)
        shows = try container.decodeIfPresent(GlanceShows.self, forKey: .shows) ?? .all
        empty = try container.decodeIfPresent(String.self, forKey: .empty) ?? ""
        tabs = decodeViews(container, forKey: .tabs) ?? GlanceView.allCases
        resetParts = (try? container.decodeIfPresent(GlanceResetParts.self, forKey: .resetParts)) ?? .all
        upcomingLimit = max(0, (try? container.decodeIfPresent(Int.self, forKey: .upcomingLimit)) ?? 0)
        resetsProvider = (try? container.decodeIfPresent(GlanceResetsProvider.self, forKey: .resetsProvider)) ?? .codex
    }

    func has(_ view: GlanceView) -> Bool { tabs.contains(view) }

    /// Accounts with something to show: readings, or a notice saying why there are none.
    var visibleProviders: [GlanceProvider] {
        providers.filter { !$0.metrics.isEmpty || $0.notice != nil }
    }
}

struct GlanceProvider: Equatable, Identifiable {
    var id: String
    /// The card heading: the brand for an account named by its email, the account title otherwise.
    var name: String
    var account: String?
    var plan: String?
    /// The plan's paid period, the card header's right corner.
    var term: GlancePlanTerm? = nil
    /// `Dữ liệu cũ`, while the reading is two refresh intervals old, as beside the card's name.
    var outdated: String? = nil
    /// Why the card header shows its warning triangle: a failed refresh, an error, a provider
    /// warning; with readings or without.
    var problem: String? = nil
    /// What an account without readings says in their place: its problem, or `Không có dữ liệu`.
    var notice: String?
    var brand: String
    /// The mark's color on the island's black, `#FFFFFF` for a brand without one.
    var color: String
    /// The mark's color on a light background, where it differs from `color`.
    var lightColor: String? = nil
    var mark: GlanceMark?
    var metrics: [GlanceMetric]
    /// The row the popup's card starts with: Codex's free reset, or Claude's banked reset for this
    /// account's plan.
    var resetRow: GlanceResetRow? = nil

    var tint: Color { Color(glanceHex: color) ?? .white }

    /// The reset row while it is still shown at `now`.
    func resetRow(at now: Date) -> GlanceResetRow? {
        resetRow.flatMap { $0.shows(at: now) ? $0 : nil }
    }

    /// The rows a layout deals out for this account at `now`: its reset row and its metrics, or one
    /// for the line saying why it has none.
    func rowCount(at now: Date) -> Int {
        (resetRow(at: now) == nil ? 0 : 1) + max(metrics.count, 1)
    }

    /// Whether the popup draws this mark in its brand color: a brand with a color and a mark (or
    /// its official color logo, which keeps its own colors).
    var hasBrandTint: Bool {
        color.uppercased() != "#FFFFFF" && (mark?.art != nil || !(mark?.paths.isEmpty ?? true))
    }

    /// The mark's color on a widget in `scheme`, as the popup picks it for its theme: the brand color
    /// for that appearance, else the secondary text color (a brand without a color, or without a
    /// mark, whose initial is drawn instead).
    func markColor(in scheme: ColorScheme) -> Color {
        guard hasBrandTint else { return .secondary }
        if scheme == .light, let lightColor, let light = Color(glanceHex: lightColor) { return light }
        return tint
    }
}

extension GlanceProvider: Decodable {
    private enum CodingKeys: String, CodingKey {
        case id, name, account, plan, term, outdated, problem, notice, brand, color, lightColor, mark, metrics, resetRow
    }

    /// Keys added after the first release are read leniently, so one malformed key never loses the
    /// whole document.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        account = try container.decodeIfPresent(String.self, forKey: .account)
        plan = try container.decodeIfPresent(String.self, forKey: .plan)
        term = try? container.decodeIfPresent(GlancePlanTerm.self, forKey: .term)
        outdated = try? container.decodeIfPresent(String.self, forKey: .outdated)
        problem = try? container.decodeIfPresent(String.self, forKey: .problem)
        notice = try container.decodeIfPresent(String.self, forKey: .notice)
        brand = try container.decode(String.self, forKey: .brand)
        color = try container.decode(String.self, forKey: .color)
        lightColor = try? container.decodeIfPresent(String.self, forKey: .lightColor)
        mark = try container.decodeIfPresent(GlanceMark.self, forKey: .mark)
        metrics = try container.decode([GlanceMetric].self, forKey: .metrics)
        resetRow = try? container.decodeIfPresent(GlanceResetRow.self, forKey: .resetRow)
    }
}

/// The plan's paid period (see `GlancePlanTerm` in `src/model/glance.ts`): when it ends, when the
/// corner turns to the warning color, and the day it ends as a date. The words are filled in at the
/// moment drawn, so the corner stays right between documents, as the popup's does.
struct GlancePlanTerm: Decodable, Equatable {
    var endsAt: Date
    var soonAt: Date
    var on: String
    var estimated: Bool? = nil

    /// The corner at `now`: the time left over the day it ends, and whether it takes the warning
    /// color; `nil` for an estimate whose day has passed, which the next document replaces.
    func lines(now: Date, words: GlancePlanTermWords, calendar: Calendar = .current) -> (left: String, day: String, soon: Bool)? {
        let about = estimated == true ? "~" : ""
        let remaining = endsAt.timeIntervalSince(now)
        let days = Self.calendarDays(from: now, to: endsAt, calendar: calendar)
        if remaining <= 0 {
            if estimated == true { return nil }
            return (words.due, days >= 0 ? words.today : on, true)
        }
        let word = days <= 0 ? words.today : days == 1 ? words.tomorrow : on
        let (forms, count) = Self.left(remaining, words: words)
        guard let form = count == 1 ? forms.first : forms.last else { return nil }
        let left = form.replacingOccurrences(of: "{n}", with: "\(about)\(count)")
        return (left, words.until.replacingOccurrences(of: "{d}", with: "\(about)\(word)"), now > soonAt)
    }

    /// The moments after `now` when `lines` reads differently: each step of the count within `span`
    /// and the first beyond it, but every minute of the last hour through the end, so a widget whose
    /// reload comes late still counts down; the corner turning to the warning color, the period
    /// ending, and the midnight that turns its day into tomorrow or today.
    func changes(after now: Date, within span: TimeInterval = 15 * 60, calendar: Calendar = .current) -> [Date] {
        var moments = [soonAt.addingTimeInterval(1), endsAt]
        var cursor = now
        while let step = nextStep(after: cursor) {
            moments.append(step)
            if step.timeIntervalSince(now) > span && endsAt.timeIntervalSince(step) >= Self.hour { break }
            cursor = step
        }
        if Self.calendarDays(from: now, to: endsAt, calendar: calendar) <= 2,
           let midnight = calendar.nextDate(after: now, matching: DateComponents(hour: 0, minute: 0, second: 0), matchingPolicy: .nextTime) {
            moments.append(midnight)
        }
        return moments.filter { $0 > now }
    }

    /// The first moment after `moment` when the count of time left steps down, as `left` rounds it.
    private func nextStep(after moment: Date) -> Date? {
        let remaining = endsAt.timeIntervalSince(moment)
        if remaining <= 0 { return nil }
        if remaining < Self.hour { return endsAt.addingTimeInterval(-(ceil(remaining / 60) - 1) * 60) }
        if remaining < Self.day { return endsAt.addingTimeInterval(-floor(remaining / Self.hour) * Self.hour + 1) }
        return endsAt.addingTimeInterval(-floor(remaining / Self.day) * Self.day + 1)
    }

    private static let hour: TimeInterval = 3600
    private static let day: TimeInterval = 86400

    /// The time left in its largest whole unit, as the popup counts it: whole days, whole hours in the
    /// last day, minutes rounded up in the last hour.
    private static func left(_ remaining: TimeInterval, words: GlancePlanTermWords) -> ([String], Int) {
        if remaining < hour { return (words.minutes, Int(ceil(remaining / 60))) }
        if remaining < day { return (words.hours, Int(floor(remaining / hour))) }
        return (words.days, Int(floor(remaining / day)))
    }

    /// Calendar days from `start`'s day to `end`'s in the device's zone: 0 today, 1 tomorrow.
    private static func calendarDays(from start: Date, to end: Date, calendar: Calendar) -> Int {
        calendar.dateComponents([.day], from: calendar.startOfDay(for: start), to: calendar.startOfDay(for: end)).day ?? 0
    }
}

/// The plan-period corner's words: `{n}` stands for the count (the form for 1, then any other) and
/// `{d}` for the day; an estimate puts `~` before either.
struct GlancePlanTermWords: Decodable, Equatable {
    var days: [String]
    var hours: [String]
    var minutes: [String]
    var due: String
    var until: String
    var today: String
    var tomorrow: String
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
    /// When the soonest of the row's reset credits expires, for the dot before its value.
    var expiresAt: Date? = nil
    /// The limit window's short name (`5h`, `week`) the menu bar strip labels a reading with; sent
    /// on the readings beside the notch.
    var period: String? = nil
    /// What the pace note and the even-pace tick are worked out from (`GlanceLimitRows.swift`).
    var pace: GlancePace? = nil
    /// The reading once `resetsAt` has passed.
    var after: GlanceAfterReset? = nil
    /// The note on the title line, worked out when the metric is read at a moment
    /// (`reading(at:pacing:)`); never decoded.
    var note: GlancePaceNote? = nil
    /// The even-pace tick 0...1 along the meter, worked out with `note`; never decoded.
    var tick: Double? = nil

    fileprivate enum CodingKeys: String, CodingKey {
        case id, label, value, headline, fraction, severity, resetsAt, detail, countdown, expiresAt, period, pace, after
    }

    /// `value`, or the countdown's words at `now`, its span as short as `GlanceFormat.shortSpan`.
    func liveValue(now: Date, units: GlanceUnits) -> String {
        countdown?.text(now: now, units: units, short: true) ?? value
    }
}

extension GlanceMetric {
    /// Keys added after the first release are read leniently, so one malformed key never loses the
    /// whole document.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(String.self, forKey: .id)
        label = try container.decode(String.self, forKey: .label)
        value = try container.decode(String.self, forKey: .value)
        headline = try container.decode(String.self, forKey: .headline)
        fraction = try container.decodeIfPresent(Double.self, forKey: .fraction)
        severity = try container.decode(GlanceSeverity.self, forKey: .severity)
        resetsAt = try container.decodeIfPresent(Date.self, forKey: .resetsAt)
        detail = try container.decodeIfPresent(String.self, forKey: .detail)
        countdown = try container.decodeIfPresent(GlanceCountdown.self, forKey: .countdown)
        expiresAt = try? container.decodeIfPresent(Date.self, forKey: .expiresAt)
        period = try? container.decodeIfPresent(String.self, forKey: .period)
        pace = try? container.decodeIfPresent(GlancePace.self, forKey: .pace)
        after = try? container.decodeIfPresent(GlanceAfterReset.self, forKey: .after)
    }
}

/// Words around a moving span of time (see `GlanceCountdown` in `src/model/glance.ts`): `text` with
/// `{d}` replaced by the time left until `at`, or gone by since it when `since`; once a countdown
/// has passed, `after`; a `since` one reads `recent` while under a minute has gone by.
struct GlanceCountdown: Decodable, Equatable {
    var at: Date
    var text: String
    var since: Bool?
    var after: String?
    var recent: String? = nil

    static let placeholder = "{d}"

    func passed(_ now: Date) -> Bool { since != true && at <= now }

    func text(now: Date, units: GlanceUnits, short: Bool = false) -> String {
        if passed(now), let after { return after }
        if since == true, let recent, now.timeIntervalSince(at) < 60 { return recent }
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

/// A reset tracker: the Codex free-reset tracker (codex-resets.com) or the Claude one
/// (claude-resets.com, where `upcoming` is a banked reset's deadline); see `GlanceResets` in
/// `src/model/glance.ts`.
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
    var presentation: GlanceResetPresentation? = nil
    var theme: String? = nil
    /// The site the source line links to; absent for Codex, whose site is `https://codex-resets.com`.
    var site: String? = nil

    var tint: Color { Color(glanceHex: color) ?? .white }

    /// The tracker's mark color, the text color when the brand is white.
    var markTint: Color { color.uppercased() == "#FFFFFF" ? .primary : tint }

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
        if !parts.next {
            copy.upcoming = nil
            copy.presentation?.statuses = []
            copy.presentation?.quietTitle = nil
        }
        if !parts.latest {
            copy.latest = nil
            copy.presentation?.latest = nil
        }
        if !parts.chances {
            copy.forecast = []
            copy.forecastNote = ""
            copy.presentation?.forecast.chances = []
            copy.presentation?.forecast.sampleNote = nil
            copy.presentation?.forecast.reliability = nil
            copy.presentation?.forecast.disclaimer = nil
            copy.presentation?.forecast.unavailable = nil
        }
        if !parts.wait {
            copy.wait = nil
            copy.median = nil
            copy.presentation?.forecast.wait = nil
            copy.presentation?.forecast.waitFraction = nil
            copy.presentation?.forecast.median = nil
        }
        if !parts.calendar { copy.calendar = nil }
        if !parts.rhythm { copy.rhythm = nil }
        return copy
    }
}

/// The Reset tab's own line for a tracker that is on but has nothing yet (see `GlanceResetsPending`
/// in `src/model/glance.ts`): still loading, or could not load.
struct GlanceResetsPending: Decodable, Equatable {
    var text: String
    /// The feeds could not be loaded, which the Reset tab says in the notice color.
    var failed: Bool?
}

struct GlanceResetAuthor: Decodable, Equatable {
    var handle: String
}

struct GlanceResetLatestPresentation: Decodable, Equatable {
    var title: String
    var ago: String
    var at: Date
    var meta: String
    var author: GlanceResetAuthor?
    /// Lines under the announcement, e.g. whether the reset covers this account's plan (Claude).
    var notes: [String]? = nil
    /// The words it was announced with, as the reset notification quoted them, cut like the other cards'.
    var excerpt: String? = nil
    /// A link to the post that announced it.
    var url: String? = nil
    /// Said when no post announced it: the site recorded the reset itself.
    var observed: String? = nil

    /// How long ago `at` was at `now`, as the Reset tab's latest reset words it (`resetAgoText`):
    /// in whole minutes, hours or days.
    static func ago(since at: Date, now: Date, locale: String) -> String {
        let minutes = max(1, Int(floor(now.timeIntervalSince(at) / 60)))
        let hours = minutes / 60
        let formatter = RelativeDateTimeFormatter()
        formatter.locale = Locale(identifier: locale)
        formatter.dateTimeStyle = .numeric
        formatter.unitsStyle = .full
        let components = hours < 1 ? DateComponents(minute: -minutes) : (hours < 24 ? DateComponents(hour: -hours) : DateComponents(day: -(hours / 24)))
        return formatter.localizedString(from: components)
    }
}

struct GlanceResetStatusCard: Decodable, Equatable, Identifiable {
    var id: String
    var kind: String
    var level: String?
    var title: String
    var excerpt: String?
    var meta: [String]
    var due: String?
    var author: GlanceResetAuthor?
    var url: String?
    var hideAt: Date?
    var announced: GlanceCountdown? = nil
    var scheduledMeta: String? = nil
    var dueCountdown: GlanceCountdown? = nil
    var overdueCountdown: GlanceCountdown? = nil
    /// The card is about the latest reset (a Claude banked reset), whose card already quotes the post
    /// while it is drawn with it.
    var sameAsLatest: Bool? = nil

    /// The card's meta lines: a scheduled card's first one says, as time passes, how long ago the
    /// reset was announced; the others stay as the popup wrote them.
    func liveMetadata(units: GlanceUnits) -> [GlanceResetElement] {
        guard kind == "scheduled", let announced, let scheduledMeta else { return meta.map { .text($0, .secondary) } }
        return [.live(.countdownThen(announced, units, scheduledMeta), .secondary)]
    }

    /// The time left to the card's stated time (or deadline), then how long it is past it, in
    /// `style`: a meta line's, or the big accent line a banked reset's time left is.
    func liveDue(units: GlanceUnits, style: GlanceResetTextStyle = .secondary) -> GlanceResetElement? {
        guard let dueCountdown else { return due.map { .text($0, style) } }
        return .live(.dueThenOverdue(dueCountdown, overdueCountdown, units), style)
    }
}

struct GlanceResetForecastChance: Decodable, Equatable, Identifiable {
    var days: Int
    var label: String
    var percent: String
    var fraction: Double
    var id: Int { days }
}

struct GlanceResetForecastPresentation: Decodable, Equatable {
    var title: String
    var chances: [GlanceResetForecastChance]
    var wait: String?
    var waitFraction: Double?
    var median: String?
    var sampleNote: String?
    /// How the estimate would have done on this history (`Thử lại trên 60 ngày…`), as the Reset tab
    /// says it under the chances; absent while the history is too short to try.
    var reliability: String? = nil
    var disclaimer: String?
    var unavailable: String?
}

struct GlanceResetStat: Decodable, Equatable {
    var label: String
    var value: String
}

struct GlanceResetHistoryItem: Decodable, Equatable, Identifiable {
    var id: String
    var kind: String
    var kindLabel: String
    var when: String
    var excerpt: String
    var author: GlanceResetAuthor?
    var url: String?
    var observed: String?
    /// Who the reset covered (Claude).
    var scope: String? = nil
    /// `Chưa kiểm chứng` / `Not reviewed`, while the site has not reviewed the entry (Claude).
    var provisional: String? = nil
}

/// A limit change (Claude): a row of the list the Reset tab keeps apart from the history, since it
/// reset nothing.
struct GlanceResetChangeItem: Decodable, Equatable, Identifiable {
    var id: String
    var when: String
    var excerpt: String
    var author: GlanceResetAuthor?
    var url: String?
    /// Who the change covered.
    var scope: String?
    /// `Chưa kiểm chứng` / `Not reviewed`, while the site has not reviewed the entry.
    var provisional: String?
}

/// Claude against Codex over the time both were tracked, the last card of the Reset tab's Claude
/// view (see `GlanceResetCompare` in src/model/glance.ts).
struct GlanceResetCompare: Decodable, Equatable {
    var title: String
    /// `Tính các lần reset sau …`, the note under the months.
    var since: String
    var columns: Columns?
    var rows: [Row]
    var monthsTitle: String
    var months: [Month]

    /// One side's column heading: the tracker's name beside its mark, in its brand color.
    struct Column: Decodable, Equatable {
        var name: String
        var color: String
        var mark: GlanceMark?

        var tint: Color { Color(glanceHex: color) ?? .primary }
    }

    struct Columns: Decodable, Equatable {
        var claude: Column
        var codex: Column
    }

    struct Row: Decodable, Equatable {
        var label: String
        var claude: String
        var codex: String
    }

    struct Month: Decodable, Equatable {
        var label: String
        var claude: Int
        var codex: Int
        /// What VoiceOver reads for the month: `T9: Claude 3, Codex 2`.
        var summary: String
    }

    /// The column headings, named and colored like the Reset tab's when an older document has none.
    var heads: Columns {
        columns ?? Columns(claude: Column(name: "Claude", color: "#DE7356"), codex: Column(name: "Codex", color: "#10A37F"))
    }

    /// The most resets either side had in a month, which the tallest bar stands for.
    var busiestMonth: Int {
        max(1, months.flatMap { [$0.claude, $0.codex] }.max() ?? 0)
    }
}

struct GlanceResetPresentation: Decodable, Equatable {
    var locale: String
    var authorAvatar: String
    /// The one author `authorAvatar` pictures (Claude: `@ClaudeDevs`); absent, it pictures every
    /// author (Codex).
    var avatarHandle: String? = nil
    /// Lines above the cards (Claude): the site is behind, or only its published copy could be read.
    var notices: [String]? = nil
    var latest: GlanceResetLatestPresentation?
    var statuses: [GlanceResetStatusCard]
    var quietTitle: String? = nil
    var forecast: GlanceResetForecastPresentation
    var statsTitle: String
    var stats: [GlanceResetStat]
    var historyTitle: String
    var history: [GlanceResetHistoryItem]
    /// The limit changes (Claude), under their heading, each row's badge word, then the note under them.
    var changesTitle: String? = nil
    var changeBadge: String? = nil
    var changes: [GlanceResetChangeItem]? = nil
    var changesNote: String? = nil
    /// Claude against Codex (Claude), once the Codex history is at hand.
    var compare: GlanceResetCompare? = nil
    var patternNote: String
    /// When the copy shown was read (`Tải 5 phút trước`), the line above the source.
    var fetched: GlanceCountdown? = nil
    var source: String
    var methodTitle: String
    var method: [String]

    func statuses(at now: Date) -> [GlanceResetStatusCard] {
        let current = statuses.filter { card in card.hideAt.map { $0 > now } ?? true }
        if current.isEmpty, let quietTitle {
            return [GlanceResetStatusCard(id: "quiet", kind: "quiet", title: quietTitle, meta: [])]
        }
        return current
    }

    /// The picture beside `author`: `authorAvatar`, unless it pictures `avatarHandle` alone and
    /// `author` is someone else, who then gets none.
    func avatar(for author: GlanceResetAuthor) -> String {
        if let avatarHandle, avatarHandle.caseInsensitiveCompare(author.handle) != .orderedSame { return "" }
        return authorAvatar
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

    /// The key's entries, in the order the calendar lists them.
    enum Item: CaseIterable {
        case regular
        case banked
        case today
    }
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
    /// A meter's fill in the popup's pace colors (`--uc-blue`, `--uc-yellow`, `--uc-red`): the dark
    /// theme's on the island's black or a dark widget, the light theme's on a light one, where the
    /// yellow is darkened to read on white.
    static func fill(_ severity: GlanceSeverity, onDark: Bool) -> Color {
        switch severity {
        case .normal:
            return onDark ? Color(red: 0.04, green: 0.52, blue: 1.0) : Color(red: 0, green: 0.478, blue: 1.0)
        case .warning:
            return onDark ? Color(red: 1.0, green: 0.84, blue: 0.04) : Color(red: 0.961, green: 0.722, blue: 0)
        case .critical:
            return onDark ? Color(red: 1.0, green: 0.27, blue: 0.23) : Color(red: 1.0, green: 0.231, blue: 0.188)
        case .none:
            return .secondary
        }
    }

    /// A reading that no meter or ring beside it colors (a percentage beside the notch, an alert's
    /// title): the pace color for a warning or a limit running out, the text color otherwise.
    /// Everywhere a meter carries the pace, the reading keeps the text color, as the popup's rows do.
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
