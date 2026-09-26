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
    var alert: GlanceAlert?

    static let supportedVersion = 1

    private enum CodingKeys: String, CodingKey {
        case version, generatedAt, locale, hour12, labels, providers, island, widget, alert
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

    private enum CodingKeys: String, CodingKey {
        case title, empty, updated, resetsIn, resetting, open, notRunning, noData, more, units
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
        units: GlanceUnits
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
    }
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

    private enum CodingKeys: String, CodingKey {
        case enabled, alerts, style, wings, expandOnHover, shows, empty
    }

    init(
        enabled: Bool,
        alerts: Bool = true,
        style: IslandStyle = .percent,
        wings: [GlanceProvider] = [],
        expandOnHover: Bool = true,
        shows: GlanceShows = .all,
        empty: String? = nil
    ) {
        self.enabled = enabled
        self.alerts = alerts
        self.style = style
        self.wings = wings
        self.expandOnHover = expandOnHover
        self.shows = shows
        self.empty = empty
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
    }
}

/// What the desktop widgets list.
struct GlanceWidgetContent: Decodable, Equatable {
    var providers: [GlanceProvider]
    var shows: GlanceShows
    var empty: String

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

struct GlanceMark: Decodable, Equatable {
    var box: [Double]
    var paths: [GlancePath]
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
