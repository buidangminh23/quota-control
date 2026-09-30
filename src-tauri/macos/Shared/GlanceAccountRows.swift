import AppKit
import SwiftUI

/// A clock time with its day, as the popup words the moment a reset comes (`timeOnDayLabel`;
/// `GlanceDayWords` in `src/model/glance.ts`): `13:05 · hôm nay`, `1:05 PM · tomorrow`,
/// `13:05 · T2 05/10`. The day is picked at the moment drawn, in the device's zone as the popup
/// picks it, so the words stay right between documents.
struct GlanceDayWords: Decodable, Equatable {
    /// `{t} · hôm nay`, `{t}` standing for the clock time.
    var today: String
    /// `{t} · ngày mai`.
    var tomorrow: String
    /// `{t} · {d}`, `{d}` standing for a day neither today nor tomorrow.
    var other: String
    /// The clock time as a date pattern that follows the Time Format setting: `H:mm`, `h:mm a`, `HH:mm`.
    var time: String
    /// Another day as a date pattern: `EEEEEE dd/MM` (`T2 05/10`), `EEE, MMM d` (`Mon, Oct 5`).
    var date: String

    static let timePlaceholder = "{t}"
    static let dayPlaceholder = "{d}"

    /// `moment` as the popup words it at `now`: today for any day up to today, as the popup counts
    /// a moment that has passed, tomorrow, else its weekday and date.
    func label(_ moment: Date, now: Date, locale: Locale, calendar: Calendar = .current) -> String {
        let days = GlanceDays.between(now, moment, calendar: calendar)
        let clock = GlanceDays.format(moment, pattern: time, locale: locale, calendar: calendar)
        if days <= 0 { return today.replacingOccurrences(of: Self.timePlaceholder, with: clock) }
        if days == 1 { return tomorrow.replacingOccurrences(of: Self.timePlaceholder, with: clock) }
        return other
            .replacingOccurrences(of: Self.timePlaceholder, with: clock)
            .replacingOccurrences(of: Self.dayPlaceholder, with: GlanceDays.format(moment, pattern: date, locale: locale, calendar: calendar))
    }
}

/// Calendar days and the formatters the day words draw with.
enum GlanceDays {
    private static let formatters = NSCache<NSString, DateFormatter>()

    /// Calendar days from `start`'s day to `end`'s in the device's zone: 0 today, 1 tomorrow.
    static func between(_ start: Date, _ end: Date, calendar: Calendar = .current) -> Int {
        calendar.dateComponents([.day], from: calendar.startOfDay(for: start), to: calendar.startOfDay(for: end)).day ?? 0
    }

    /// The next midnight after `now`, when today's words turn into tomorrow's.
    static func nextMidnight(after now: Date, calendar: Calendar = .current) -> Date? {
        calendar.nextDate(after: now, matching: DateComponents(hour: 0, minute: 0, second: 0), matchingPolicy: .nextTime)
    }

    static func format(_ date: Date, pattern: String, locale: Locale, calendar: Calendar) -> String {
        let key = "\(locale.identifier)|\(calendar.timeZone.identifier)|\(pattern)" as NSString
        if let formatter = formatters.object(forKey: key) { return formatter.string(from: date) }
        let formatter = DateFormatter()
        formatter.locale = locale
        formatter.timeZone = calendar.timeZone
        formatter.dateFormat = pattern
        formatters.setObject(formatter, forKey: key)
        return formatter.string(from: date)
    }
}

extension GlanceDocument {
    /// `moment`'s clock time with its day as the popup words it at `now`; a document from before
    /// the day words carried none, and names the clock time alone.
    func dayLabel(_ moment: Date, now: Date) -> String {
        labels.days?.label(moment, now: now, locale: resolvedLocale)
            ?? GlanceFormat.time(moment, locale: resolvedLocale, hour12: hour12)
    }

    /// The picture of the account `handle` names, when the document carries one.
    func avatar(for handle: String) -> String? {
        avatars?[handle.lowercased()]
    }

    /// The moments after `now` when the accounts of `providers` read differently on their own: a
    /// reset row's countdown ends, it goes, or its day turns into tomorrow or today; a reset credit's
    /// dot changes color; and, while a limit is coming back, the day of its reset time turns.
    func accountMoments(_ providers: [GlanceProvider], after now: Date, calendar: Calendar = .current) -> [Date] {
        var moments = providers.flatMap { provider in
            (provider.resetRow?.changes(after: now, calendar: calendar) ?? [])
                + provider.metrics.flatMap { $0.expiryChanges(after: now) }
        }
        if labels.days != nil, !GlanceUpcomingLimit.list(providers, now: now).isEmpty, let midnight = GlanceDays.nextMidnight(after: now, calendar: calendar) {
            moments.append(midnight)
        }
        return moments.filter { $0 > now }
    }
}

/// Whether another of `providers` carries the same heading as `provider`, so its email has to say
/// which account it is.
func sharesHeading(_ provider: GlanceProvider, in providers: [GlanceProvider]) -> Bool {
    providers.contains { $0.id != provider.id && $0.name == provider.name }
}

/// `provider`'s heading, with its email when another account shares the heading.
func accountName(_ provider: GlanceProvider, in providers: [GlanceProvider]) -> String {
    if sharesHeading(provider, in: providers), let account = provider.account {
        return "\(provider.name) (\(account))"
    }
    return provider.name
}

/// The row a Codex or Claude account starts with in the popup's card (`FreeResetRow`,
/// `BankedResetRow`; `GlanceResetRow` in `src/model/glance.ts`): Codex's coming free reset or the
/// banked reset a Claude account's plan can still apply. The countdown and the moment's day move
/// with the clock, so they are filled in when drawn, as the popup words them at every tick.
struct GlanceResetRow: Decodable, Equatable {
    enum Tone: String, Decodable, Equatable {
        /// An announced Codex reset.
        case positive
        /// The site's watch: a reset is likely, not announced.
        case notice
        /// A Claude banked reset.
        case accent
    }

    /// Whose tracker the row stands for, and the Reset tab view pressing it opens.
    var tracker: GlanceResetsProvider
    var title: String
    var tone: Tone
    /// The account whose picture sits before the title (`@thsottiaux`, `@ClaudeDevs`).
    var author: String
    /// The time left (`sau {d}`, `còn {d}`), reading its `after` once it has passed (`chờ xác nhận`).
    var countdown: GlanceCountdown?
    /// The value when there is no countdown (`chưa rõ giờ`).
    var value: String?
    /// The time it points at in the device's zone (`Lúc {at} · GMT+7`), or why there is none.
    var caption: String
    /// The caption once the countdown has passed (`Hẹn {at} · GMT+7`).
    var captionAfter: String?
    /// The moment `{at}` names.
    var at: Date?
    /// The poster's own day under an estimated time.
    var note: String?
    /// The post and how its time was read, the popup row's hover text.
    var details: String
    /// When the row goes.
    var hideAt: Date
    /// Pressing the row opens the Reset tab at `tracker`, as the popup's row does while that tab is on.
    var opens: Bool?

    static let momentPlaceholder = "{at}"

    /// Whether the row is still shown at `now`.
    func shows(at now: Date) -> Bool { hideAt > now }

    /// The row at `now`: the countdown's words (or the fixed value), the caption with `{at}` worded by
    /// `moment`, the note, and whether the countdown has reached its time, when the popup draws the
    /// value in the secondary color.
    func lines(now: Date, units: GlanceUnits, moment: (Date) -> String) -> (value: String, caption: String, note: String?, awaiting: Bool) {
        let awaiting = countdown?.passed(now) ?? false
        let value = countdown?.text(now: now, units: units) ?? (self.value ?? "")
        let words = (awaiting ? captionAfter : nil) ?? caption
        let caption = at.map { words.replacingOccurrences(of: Self.momentPlaceholder, with: moment($0)) } ?? words
        return (value, caption, note, awaiting)
    }

    /// The moments after `now` when the row reads differently without a new document: its countdown
    /// reaches its time, the row goes, and the midnights that turn its moment's day into tomorrow or
    /// today.
    func changes(after now: Date, calendar: Calendar = .current) -> [Date] {
        var moments = [countdown?.at, hideAt].compactMap { $0 }
        if let at, (1...2).contains(GlanceDays.between(now, at, calendar: calendar)),
           let midnight = GlanceDays.nextMidnight(after: now, calendar: calendar) {
            moments.append(midnight)
        }
        return moments.filter { $0 > now }
    }
}

extension GlanceMetric {
    private static let expiryCritical: TimeInterval = 48 * 3600
    private static let expiryWarning: TimeInterval = 7 * 24 * 3600

    /// The color of the dot before a reset-credit count (`expirySeverity`): red once the soonest
    /// credit expires within 48 hours, yellow within 7 days, blue otherwise; `nil` for a row without
    /// credits.
    func expirySeverity(at now: Date) -> GlanceSeverity? {
        guard let expiresAt else { return nil }
        let left = expiresAt.timeIntervalSince(now)
        if left <= Self.expiryCritical { return .critical }
        if left <= Self.expiryWarning { return .warning }
        return .normal
    }

    /// The moments after `now` when the dot changes color.
    func expiryChanges(after now: Date) -> [Date] {
        guard let expiresAt else { return [] }
        return [expiresAt.addingTimeInterval(-Self.expiryWarning), expiresAt.addingTimeInterval(-Self.expiryCritical)].filter { $0 > now }
    }
}

/// The popup's text colors (`src/styles/tokens.css`) for its light or dark theme.
enum GlanceRowInk {
    /// `--uc-label`.
    static func label(dark: Bool) -> Color { dark ? Color.white.opacity(0.9) : Color.black.opacity(0.88) }
    /// `--uc-secondary`.
    static func secondary(dark: Bool) -> Color { dark ? Color.white.opacity(0.62) : Color.black.opacity(0.62) }
    /// `--uc-tertiary`.
    static func tertiary(dark: Bool) -> Color { dark ? Color.white.opacity(0.5) : Color.black.opacity(0.56) }
    /// `--uc-quaternary`.
    static func quaternary(dark: Bool) -> Color { dark ? Color.white.opacity(0.12) : Color.black.opacity(0.08) }

    /// A reset row's value: `--uc-positive-text` for an announced reset, `--uc-notice-text` for a
    /// watch, `--uc-accent` for a banked reset.
    static func tone(_ tone: GlanceResetRow.Tone, dark: Bool) -> Color {
        switch tone {
        case .positive: return Color(glanceHex: dark ? "#30d158" : "#1d7a35")!
        case .notice: return Color(glanceHex: dark ? "#ff9f0a" : "#b25900")!
        case .accent: return Color(glanceHex: dark ? "#0a84ff" : "#007aff")!
        }
    }
}

/// The type sizes a reset row is drawn at on a surface.
struct GlanceResetRowSizes {
    var avatar: CGFloat
    var title: CGFloat
    var value: CGFloat
    var caption: CGFloat

    /// The open island, a little smaller than its metric rows as in the popup's card.
    static let island = GlanceResetRowSizes(avatar: 14, title: 11.5, value: 11.5, caption: 10)
    /// The open island in Compact density: the popup's compact step down, never under its caption.
    static let islandCompact = GlanceResetRowSizes(avatar: 12, title: 10.5, value: 10.5, caption: 10)
}

/// The row a Codex or Claude account starts with, as the popup's card draws it: the poster's picture
/// and the title, the value in the row's color on the right (the secondary color once its countdown
/// has reached its time), then the time it points at and the poster's own day, right-aligned in the
/// tertiary color. Where the title and the value do not fit one line, the value goes under the
/// title. The countdown is worded at `now` as the popup words it (a widget's timeline has an entry
/// for each minute it steps); a compact list leaves the caption and the note out.
struct GlanceResetRowView: View {
    let row: GlanceResetRow
    let document: GlanceDocument
    let now: Date
    let sizes: GlanceResetRowSizes
    var onDark = false
    var showsCaption = true
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let dark = onDark || colorScheme == .dark
        let lines = row.lines(now: now, units: document.labels.units) { document.dayLabel($0, now: now) }
        VStack(alignment: .trailing, spacing: 2) {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    heading(dark: dark)
                    Spacer(minLength: 8)
                    value(lines, dark: dark)
                        .fixedSize()
                }
                VStack(alignment: .trailing, spacing: 2) {
                    heading(dark: dark)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    value(lines, dark: dark)
                        .truncationMode(.tail)
                }
            }
            if showsCaption {
                caption(lines.caption)
                if let note = lines.note {
                    caption(note)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
        .accessibilityElement(children: .combine)
    }

    private func heading(dark: Bool) -> some View {
        HStack(spacing: 4) {
            GlanceRowAvatar(handle: row.author, picture: document.avatar(for: row.author), size: sizes.avatar, dark: dark)
            Text(row.title)
                .font(.system(size: sizes.title, weight: .semibold))
                .foregroundStyle(onDark ? AnyShapeStyle(GlanceRowInk.label(dark: true)) : AnyShapeStyle(.primary))
                .lineLimit(1)
        }
    }

    private func value(_ lines: (value: String, caption: String, note: String?, awaiting: Bool), dark: Bool) -> some View {
        Text(lines.value)
            .font(.system(size: sizes.value))
            .monospacedDigit()
            .foregroundStyle(lines.awaiting ? GlanceRowInk.secondary(dark: dark) : GlanceRowInk.tone(row.tone, dark: dark))
            .lineLimit(1)
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(.system(size: sizes.caption))
            .monospacedDigit()
            .foregroundStyle(onDark ? AnyShapeStyle(GlanceRowInk.tertiary(dark: true)) : AnyShapeStyle(.secondary))
            .lineLimit(1)
            .truncationMode(.tail)
    }
}

extension GlanceCountdown {
    /// The words with the span drawn as text WidgetKit keeps counting by itself, or the words for a
    /// countdown that has already passed. The timeline adds an entry at `at`, so the switch happens
    /// on time.
    func live(now: Date, units: GlanceUnits) -> Text {
        if passed(now) { return Text(text(now: now, units: units)) }
        let (before, after) = parts
        return Text("\(before)\(Text(at, style: .relative))\(after)")
    }
}

/// The round picture of the account a reset row names (the popup's `ResetAuthorAvatar`): its
/// picture when the document carries one, else its initial on a faint circle, with a faint ring.
struct GlanceRowAvatar: View {
    let handle: String
    let picture: String?
    let size: CGFloat
    let dark: Bool

    private static let cache = NSCache<NSString, NSImage>()

    var body: some View {
        Group {
            if let image = picture.flatMap(Self.image) {
                Image(nsImage: image)
                    .resizable()
                    .interpolation(.high)
                    .scaledToFill()
            } else {
                Text(initial)
                    .font(.system(size: (size * 0.55).rounded(), weight: .bold))
                    .foregroundStyle(GlanceRowInk.secondary(dark: dark))
                    .frame(width: size, height: size)
                    .background(GlanceRowInk.quaternary(dark: dark))
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .background(Circle().fill(GlanceRowInk.quaternary(dark: dark)).padding(-1))
        .accessibilityHidden(true)
    }

    private var initial: String {
        String(handle.drop { $0 == "@" }.prefix(1)).uppercased()
    }

    /// A `data:image/…;base64,` picture, decoded once per distinct picture.
    static func image(_ value: String) -> NSImage? {
        let key = value as NSString
        if let image = cache.object(forKey: key) { return image }
        guard let comma = value.firstIndex(of: ","), value.hasPrefix("data:image/"),
              let data = Data(base64Encoded: String(value[value.index(after: comma)...])),
              let image = NSImage(data: data)
        else { return nil }
        cache.setObject(image, forKey: key)
        return image
    }
}

/// The dot before a reset-credit count (the popup's `ExpiryDot`), in the pace colors of the light
/// or the dark theme.
struct GlanceExpiryDot: View {
    let severity: GlanceSeverity
    var onDark = false
    var size: CGFloat = 6
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Circle()
            .fill(GlancePalette.fill(severity, onDark: onDark || colorScheme == .dark))
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// A metric's value with the expiry dot before it when the row carries reset credits, the dot
/// centered on the text as in the popup's row.
struct GlanceValueWithDot<Value: View>: View {
    let metric: GlanceMetric
    let now: Date
    var onDark = false
    var dotSize: CGFloat = 6
    @ViewBuilder var value: () -> Value

    var body: some View {
        HStack(spacing: 4) {
            if let severity = metric.expirySeverity(at: now) {
                GlanceExpiryDot(severity: severity, onDark: onDark, size: dotSize)
            }
            value()
        }
    }
}
