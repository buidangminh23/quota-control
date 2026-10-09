import SwiftUI

/// What a limit's pace note and even-pace tick are worked out from (`GlancePace` in
/// `src/model/glance.ts`): `spent` for a limit used up; for a limit counting down, the share of it
/// used and its window's length in milliseconds, the window starting that long before its reset.
struct GlancePace: Decodable, Equatable {
    var spent: Bool? = nil
    var used: Double? = nil
    var period: Double? = nil
}

/// A limit's unavailable reading after its reset until the provider confirms the new allowance.
struct GlanceAfterReset: Decodable, Equatable {
    var value: String
    var headline: String
    var fraction: Double?
    var detail: String? = nil
    var cadence: String? = nil
    var severity: GlanceSeverity? = nil
}

/// The words of the pace note on a limit's title line, `{n}` standing for its figure.
struct GlancePaceWords: Decodable, Equatable {
    /// `Đã hết hạn mức`, after a flame.
    var limitReached: String
    /// `Dư ~{n}%`.
    var spare: String
    /// `Còn ~{n}% khi đặt lại`.
    var leftAtReset: String

    static let figurePlaceholder = "{n}"
}

/// The note on a limit's title line, as the popup's `PaceWarning` shows it.
struct GlancePaceNote: Equatable {
    var text: String
    /// A limit used up, whose note follows a flame in the meter's color.
    var spent: Bool
}

/// The popup's pace verdict for a limit at a moment (`meterState`). Without one the popup colors the
/// meter by how much is used, as the document's `severity` already does.
enum GlancePaceVerdict: Equatable {
    case spent
    /// On a pace to reach the limit before its reset, or all but there; `projected` is the share
    /// used by the reset at that pace.
    case runningOut(projected: Double)
    /// Close to the limit: `spare` percent left at the reset.
    case closeToLimit(spare: Int, projected: Double)
    case healthy(projected: Double)

    var severity: GlanceSeverity {
        switch self {
        case .spent, .runningOut: return .critical
        case .closeToLimit: return .warning
        case .healthy: return .normal
        }
    }

    /// The note on the title line: nothing for a limit running out, as in the popup, and for a
    /// healthy one only while Always Show Pacing is on.
    func note(words: GlancePaceWords?, always: Bool) -> GlancePaceNote? {
        guard let words else { return nil }
        switch self {
        case .spent:
            return GlancePaceNote(text: words.limitReached, spent: true)
        case .runningOut:
            return nil
        case let .closeToLimit(spare, _):
            return GlancePaceNote(text: words.spare.replacingOccurrences(of: GlancePaceWords.figurePlaceholder, with: "\(spare)"), spent: false)
        case let .healthy(projected):
            guard always else { return nil }
            let left = GlancePace.roundedPercent(1 - projected)
            return GlancePaceNote(text: words.leftAtReset.replacingOccurrences(of: GlancePaceWords.figurePlaceholder, with: "\(left)"), spent: false)
        }
    }
}

extension GlancePace {
    /// The share used below which the popup does not trust a verdict (`PACE_DISTRUST_SHARE`): a
    /// window just begun extrapolates wildly.
    private static let distrustShare = 0.05

    /// How far into a window of `length` seconds a projection means anything (`minimumElapsed`).
    static func minimumElapsed(_ length: TimeInterval) -> TimeInterval {
        max(60, length * 0.01)
    }

    /// `share` as a whole percent, rounded as the popup's `Math.round` rounds it: halves up.
    static func roundedPercent(_ share: Double) -> Int {
        Int((share * 100 + 0.5).rounded(.down))
    }

    /// How far into its window a limit resetting at `resetsAt` is at `now`, with the window's length,
    /// both in seconds; `nil` without a length.
    private func elapsed(resetsAt: Date, now: Date) -> (elapsed: TimeInterval, length: TimeInterval)? {
        guard let period, period > 0 else { return nil }
        let length = period / 1000
        return (now.timeIntervalSince(resetsAt.addingTimeInterval(-length)), length)
    }

    /// The verdict at `now` for a limit resetting at `resetsAt`, as `meterState` and `evaluatePace`
    /// work it out; `nil` where the popup has none: too early in the window, too little used to
    /// trust, or past the reset.
    func verdict(resetsAt: Date?, now: Date) -> GlancePaceVerdict? {
        if spent == true { return .spent }
        guard let used, used > 0, let resetsAt, let window = elapsed(resetsAt: resetsAt, now: now) else { return nil }
        guard window.elapsed >= Self.minimumElapsed(window.length), now < resetsAt else { return nil }
        let projected = used / window.elapsed * window.length
        let behind = used >= 1 || projected > 1
        if !behind, projected <= 0.9 { return .healthy(projected: projected) }
        if used < Self.distrustShare { return nil }
        if behind { return .runningOut(projected: projected) }
        let spare = Self.roundedPercent(1 - projected)
        if spare < 1 { return .runningOut(projected: projected) }
        return .closeToLimit(spare: spare, projected: projected)
    }

    /// The even-pace tick at `now` (`paceTick`): the share of the window gone by, where an even pace
    /// would have the limit, counted from the other end while the meters fill with the share left.
    /// The popup shows it on a limit close to its limit or running out, and on a healthy one only
    /// while Always Show Pacing is on.
    func tick(for verdict: GlancePaceVerdict, resetsAt: Date?, now: Date, always: Bool, towardsUsed: Bool) -> Double? {
        switch verdict {
        case .spent:
            return nil
        case .healthy:
            if !always { return nil }
        case .runningOut, .closeToLimit:
            break
        }
        guard let resetsAt, let window = elapsed(resetsAt: resetsAt, now: now) else { return nil }
        guard window.elapsed >= Self.minimumElapsed(window.length), now < resetsAt else { return nil }
        let gone = min(max(window.elapsed / window.length, 0), 1)
        return towardsUsed ? gone : 1 - gone
    }
}

/// How a surface reads its limits' pace (`GlanceDocument.pacing`): the note's words, Always Show
/// Pacing, and which way the meters fill.
struct GlancePacing: Equatable {
    var words: GlancePaceWords?
    var always = false
    /// Used/Left on Used: the meters fill with the share used.
    var towardsUsed = false

    /// The color alone, for a reading without a title line to put a note on.
    static let colorOnly = GlancePacing(words: nil)
}

extension GlanceMetric {
    /// The metric as the popup's row reads it at `now`: unavailable after an unconfirmed reset;
    /// while it counts down, the meter's color, the note on its title line and
    /// the even-pace tick of its pace verdict at `now`, which the popup works out every time it
    /// renders.
    func unavailable(_ noData: String) -> GlanceMetric {
        var copy = self
        copy.value = "—"
        copy.headline = noData
        copy.fraction = nil
        copy.detail = nil
        copy.cadence = nil
        copy.severity = .none
        copy.resetsAt = nil
        copy.pace = nil
        copy.after = nil
        copy.note = nil
        copy.tick = nil
        copy.countdown = nil
        copy.expiresAt = nil
        copy.redeem = nil
        return copy
    }

    func reading(at now: Date, pacing: GlancePacing, noData: String = "—") -> GlanceMetric {
        var copy = self
        copy.note = nil
        copy.tick = nil
        if let resetsAt, resetsAt <= now {
            return unavailable(after?.fraction == nil ? after?.headline ?? noData : noData)
        }
        guard let pace, let verdict = pace.verdict(resetsAt: resetsAt, now: now) else { return copy }
        copy.severity = verdict.severity
        copy.note = verdict.note(words: pacing.words, always: pacing.always)
        copy.tick = pace.tick(for: verdict, resetsAt: resetsAt, now: now, always: pacing.always, towardsUsed: pacing.towardsUsed)
        return copy
    }
}

extension GlanceProvider {
    /// The account with every metric read at `now` (`GlanceMetric.reading(at:pacing:)`).
    func reading(at now: Date, pacing: GlancePacing, noData: String = "—") -> GlanceProvider {
        var copy = self
        if let validUntil, validUntil <= now {
            copy.metrics = metrics.map { $0.unavailable(noData) }
        } else {
            copy.metrics = metrics.map { $0.reading(at: now, pacing: pacing, noData: noData) }
        }
        return copy
    }
}

extension GlanceDocument {
    /// The pace settings and words the document's limits are read with.
    var pacing: GlancePacing {
        GlancePacing(words: labels.pace, always: alwaysShowPacing == true, towardsUsed: displayMode == "used")
    }

    /// The document as the popup reads it at `now`: every limit, on the island, the widgets and
    /// beside the notch, unavailable after its validity boundary and paced at `now`. A surface draws
    /// this at each moment it draws, so its limits read as the popup's do then.
    func reading(at now: Date) -> GlanceDocument {
        let pacing = self.pacing
        var copy = self
        copy.providers = providers.map { $0.reading(at: now, pacing: pacing, noData: labels.noData) }
        copy.widget.providers = widget.providers.map { $0.reading(at: now, pacing: pacing, noData: labels.noData) }
        copy.island.wings = island.wings.map { $0.reading(at: now, pacing: pacing, noData: labels.noData) }
        return copy
    }

    /// How the document words when a limit comes back.
    var resetWording: GlanceResetWording {
        GlanceResetWording(
            exact: resetDisplay == "absolute" && labels.resetAbsolute != nil,
            resetsIn: labels.resetsIn,
            resetting: labels.resetting,
            soon: labels.resetsSoon,
            restoresAt: labels.restoresAt,
            resetMoment: labels.resetMoment,
            absolute: labels.resetAbsolute,
            days: labels.days,
            units: labels.units,
            locale: resolvedLocale,
            hour12: hour12
        )
    }

    /// The reset text of `metric` at `now` in the Reset Times setting's form, for a line with room for
    /// one reset text only (a compact line, a ring), `short` without its verb; where reset times are
    /// switched off only a detail that is not one (`Chưa bắt đầu`, `Hạn mức 50 $`).
    func resetText(for metric: GlanceMetric, now: Date, showsReset: Bool = true, short: Bool = false) -> String? {
        guard let at = metric.resetsAt else { return metric.detail }
        guard showsReset else { return nil }
        let wording = resetWording
        return short ? wording.span(at, now: now) : wording.line(at, now: now)
    }

    /// The words at the right of `metric`'s title line at `now`, as the popup's row puts them there
    /// whatever Reset Times says (`resetCountdownText`): the countdown to its reset, to the second
    /// through its last five minutes, `short` without its verb; for a limit with no reset time yet, how
    /// long its window runs. Nothing for a row saying a status instead, or with reset times switched off.
    func countdownText(for metric: GlanceMetric, now: Date, showsReset: Bool = true, short: Bool = false) -> String? {
        guard showsReset else { return nil }
        if let at = metric.resetsAt { return resetWording.countdown(at, now: now, short: short) }
        guard let cadence = metric.cadence else { return nil }
        return short ? Self.withoutLead(cadence, lead: labels.resetsIn) : cadence
    }

    /// The words at the right of `metric`'s reading at `now` (`boundedDetailText`): the exact moment it
    /// resets, `short` without its lead-in, nothing once that has passed; for a row without a reset
    /// time, its status or what the limit is a limit of, which shows even with reset times switched off.
    func momentText(for metric: GlanceMetric, now: Date, showsReset: Bool = true, short: Bool = false) -> String? {
        guard let at = metric.resetsAt else { return metric.cadence == nil ? metric.detail : nil }
        guard showsReset else { return nil }
        return resetWording.resetMoment(at, now: now, short: short)
    }

    /// The resets of `providers`' limits that count down to the second at some moment from `now` on:
    /// a surface drawing those rows redraws each second from five minutes before each until it.
    func finalCountdowns(_ providers: [GlanceProvider], now: Date) -> [Date] {
        providers.flatMap { provider in provider.metrics.compactMap { metric in
            guard metric.fraction != nil, let at = metric.resetsAt, at > now else { return nil }
            return at
        } }
    }

    /// `text` without `lead` and the space after it: `5 giờ` of `Đặt lại sau 5 giờ`.
    private static func withoutLead(_ text: String, lead: String) -> String {
        guard !lead.isEmpty, text.hasPrefix(lead + " ") else { return text }
        return String(text.dropFirst(lead.count + 1))
    }
}

/// When a limit comes back, worded as the popup words it at a moment. A limit's row says it twice
/// whatever Reset Times says (`countdown`, `resetMoment`): `Đặt lại sau 2 giờ 5 phút` on its title
/// line, through the last five minutes `Đặt lại sau 04:59`, and `Đặt lại lúc 13:05 · ngày mai` beside
/// its reading. A line with room for one reset text (`line`, `span`) says it in the Reset Times
/// setting's form: Countdown `Đặt lại sau 2 giờ 5 phút`, in the last five minutes `Sắp đặt lại`;
/// Exact Time `Đặt lại lúc 13:05 ngày mai`. A document from before these words counts down to the
/// end and then says `Đang đặt lại…`, as it did.
struct GlanceResetWording {
    var exact: Bool
    var resetsIn: String
    var resetting: String
    var soon: String?
    var restoresAt: String?
    var resetMoment: String? = nil
    var absolute: GlanceDayWords?
    var days: GlanceDayWords?
    var units: GlanceUnits
    var locale: Locale
    var hour12: Bool?

    /// The last stretch before a reset that a one-line reset text calls soon and a limit's row counts
    /// down to the second (`FINAL_COUNTDOWN_SECONDS`).
    static let soonSpan: TimeInterval = 5 * 60

    /// Whether a reset at `at` is in its last five minutes at `now`, its row counting each second.
    static func isFinal(_ at: Date, now: Date) -> Bool {
        let left = at.timeIntervalSince(now)
        return left > 0 && left <= soonSpan
    }

    /// The whole seconds left of `left`, a second begun counting as a whole one, as minutes and seconds
    /// on a clock face (`clockCountdown`): `05:00`, `04:59`, `00:01`; never below `00:00`.
    static func clock(_ left: TimeInterval) -> String {
        let whole = left > 0 ? Int(left.rounded(.up)) : 0
        return String(format: "%02d:%02d", whole / 60, whole % 60)
    }

    /// The countdown on a limit's title line at `now` (`resetCountdownLabel`): `Đặt lại sau 2 giờ 5
    /// phút`, in the last five minutes `Đặt lại sau 04:59`, once the moment has passed `Sắp đặt lại`;
    /// `short` without its verb.
    func countdown(_ at: Date, now: Date, short: Bool = false) -> String {
        let left = at.timeIntervalSince(now)
        if left <= 0 { return soon ?? resetting }
        let span = left <= Self.soonSpan ? Self.clock(left) : GlanceFormat.countdown(to: at, from: now, units: units)
        return short ? span : "\(resetsIn) \(span)"
    }

    /// The exact moment beside a limit's reading (`resetMoment`): `Đặt lại lúc 13:05 · ngày mai`,
    /// `short` its clock time and day alone; nothing once it has passed. A document from before these
    /// words names it the way its Reset Times setting did.
    func resetMoment(_ at: Date, now: Date, short: Bool = false) -> String? {
        guard at > now else { return nil }
        let when = moment(at, now: now)
        if short { return when }
        if let resetMoment { return resetMoment.replacingOccurrences(of: GlanceResetRow.momentPlaceholder, with: when) }
        if exact, let absolute { return absolute.label(at, now: now, locale: locale) }
        return restore(at, now: now) ?? when
    }

    /// A limit's reset text at `now` for a reset at `at`.
    func line(_ at: Date, now: Date) -> String {
        let left = at.timeIntervalSince(now)
        if exact, let absolute {
            return left <= 0 ? (soon ?? resetting) : absolute.label(at, now: now, locale: locale)
        }
        if let soon, left <= Self.soonSpan { return soon }
        if left <= 0 { return resetting }
        return "\(resetsIn) \(GlanceFormat.countdown(to: at, from: now, units: units))"
    }

    /// The reset text without its verb, for a line without room for it: the time left, or in Exact
    /// Time the clock time with its day.
    func span(_ at: Date, now: Date) -> String {
        let left = at.timeIntervalSince(now)
        if exact {
            return left <= 0 ? (soon ?? resetting) : moment(at, now: now)
        }
        if let soon, left <= Self.soonSpan { return soon }
        if left <= 0 { return resetting }
        return GlanceFormat.countdown(to: at, from: now, units: units)
    }

    /// The line under a countdown with the moment the limit comes back (`Hồi lại lúc 13:05 · ngày
    /// mai`): in Countdown only, while the reset is ahead.
    func restore(_ at: Date, now: Date) -> String? {
        guard !exact, let restoresAt, at > now else { return nil }
        return restoresAt.replacingOccurrences(of: GlanceResetRow.momentPlaceholder, with: moment(at, now: now))
    }

    /// `at`'s clock time with its day, as the popup words a moment; the clock time alone for a
    /// document from before the day words.
    func moment(_ at: Date, now: Date) -> String {
        days?.label(at, now: now, locale: locale) ?? GlanceFormat.time(at, locale: locale, hour12: hour12)
    }

    /// Everything the wording draws for a reset at `at` at `now`, to tell when it changes.
    /// The countdown of the last five minutes counts on its own (a timer on a widget, each second on
    /// the island), so its words stand for all of it.
    fileprivate func words(_ at: Date, now: Date) -> [String] {
        let countdown = Self.isFinal(at, now: now) ? "final" : self.countdown(at, now: now)
        return [line(at, now: now), span(at, now: now), countdown, resetMoment(at, now: now) ?? ""]
    }
}

extension GlanceDocument {
    /// How often a limit's pace note is looked at for a change of its figure or verdict, which moves
    /// smoothly with the clock rather than a minute at a time.
    private static let paceProbe: TimeInterval = 15

    /// The moments after `now`, up to `end`, when the rows of `providers` read differently on their
    /// own: a limit's countdown steps (each minute in its last day, each hour before) or turns to
    /// `Sắp đặt lại`, its restore line or exact time names another day, its pace note's figure or
    /// verdict moves, a reset row's countdown steps. At most one a minute: the last of each minute, by
    /// when every change in it has happened; and besides those, five minutes before each limit's reset,
    /// when its countdown starts counting each second on its own, and the reset itself. A widget gives
    /// each its own timeline entry, so the words move with the clock as the popup's do.
    func rowTicks(_ providers: [GlanceProvider], after now: Date, until end: Date, calendar: Calendar = .current) -> [Date] {
        let wording = resetWording
        let pacing = self.pacing
        let midnight = GlanceDays.nextMidnight(after: now, calendar: calendar)
        var moments: [Date] = []
        var finals: [Date] = []
        for provider in providers {
            if let validUntil = provider.validUntil, validUntil > now {
                finals.append(validUntil)
            }
            if let row = provider.resetRow(at: now), let countdown = row.countdown, countdown.at > now {
                moments += GlanceTicks.changes(after: now, until: min(end, countdown.at), anchor: countdown.at) {
                    countdown.text(now: $0, units: labels.units)
                }
            }
            for metric in provider.metrics {
                guard let at = metric.resetsAt, at > now else { continue }
                let last = min(end, at)
                finals += [at.addingTimeInterval(-GlanceResetWording.soonSpan), at].filter { $0 > now && $0 <= end }
                moments += GlanceTicks.changes(after: now, until: last, anchor: at, also: [midnight].compactMap { $0 }) {
                    wording.words(at, now: $0).joined(separator: "\n")
                }
                if metric.pace?.used != nil {
                    moments += GlanceTicks.changes(after: now, until: last, every: Self.paceProbe) { moment in
                        let read = metric.reading(at: moment, pacing: pacing)
                        return "\(read.severity.rawValue)|\(read.note?.text ?? "")"
                    }
                }
            }
        }
        return Array(Set(GlanceTicks.lastPerMinute(moments) + finals)).sorted()
    }
}

/// Finding the moments words that move with the clock change.
enum GlanceTicks {
    /// The moments after `now`, up to `end`, when `words` changes, tried a second after each whole
    /// minute from `anchor` (a countdown's words step there: minutes, then hours or days) and at
    /// `also`.
    static func changes(after now: Date, until end: Date, anchor: Date, also: [Date] = [], words: (Date) -> String) -> [Date] {
        var candidates = also.filter { $0 > now && $0 <= end }
        var minute = (now.timeIntervalSince(anchor) / 60).rounded(.down)
        while true {
            let moment = anchor.addingTimeInterval(minute * 60 + 1)
            minute += 1
            if moment <= now { continue }
            if moment > end { break }
            candidates.append(moment)
        }
        return changed(candidates.sorted(), from: words(now), words: words)
    }

    /// The moments after `now`, up to `end`, when `words` changes, tried every `step`.
    static func changes(after now: Date, until end: Date, every step: TimeInterval, words: (Date) -> String) -> [Date] {
        var candidates: [Date] = []
        var moment = now.addingTimeInterval(step)
        while moment <= end {
            candidates.append(moment)
            moment = moment.addingTimeInterval(step)
        }
        return changed(candidates, from: words(now), words: words)
    }

    private static func changed(_ candidates: [Date], from first: String, words: (Date) -> String) -> [Date] {
        var previous = first
        var moments: [Date] = []
        for candidate in candidates {
            let current = words(candidate)
            if current != previous {
                moments.append(candidate)
                previous = current
            }
        }
        return moments
    }

    /// `moments` at most one a minute: the last of each minute.
    static func lastPerMinute(_ moments: [Date]) -> [Date] {
        let byMinute = Dictionary(grouping: moments) { ($0.timeIntervalSince1970 / 60).rounded(.down) }
        return byMinute.values.compactMap { $0.max() }.sorted()
    }
}

/// The note on a limit's title line (the popup's `PaceWarning`): `Dư ~8%`, `Còn ~40% khi đặt lại`,
/// or for a limit used up, a flame in the meter's color before `Đã hết hạn mức`; in the secondary
/// color, a size under the title as in the popup's row.
struct GlancePaceNoteView: View {
    let note: GlancePaceNote
    let severity: GlanceSeverity
    var onDark = false
    var size: CGFloat = 11
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 3) {
            if note.spent {
                Image(systemName: "flame.fill")
                    .font(.system(size: size))
                    .foregroundStyle(GlancePalette.fill(severity, onDark: onDark || colorScheme == .dark))
            }
            Text(note.text)
                .font(.glance(size: size))
                .monospacedDigit()
                .foregroundStyle(onDark ? AnyShapeStyle(GlanceRowInk.secondary(dark: true)) : AnyShapeStyle(.secondary))
                .lineLimit(1)
        }
        .fixedSize()
    }
}

/// When a surface drawing limit rows redraws: every `interval` from its start, and through the last
/// five minutes before each of `deadlines`, as each second left ends (`useFinalCountdown`), so the
/// countdown steps `05:00`, `04:59`, … `00:01` on time and turns at the deadline itself. Each moment
/// is worked out from the deadline, not from the previous one, so a late redraw lands on the right
/// second; with no deadline in its last five minutes it redraws every `interval` only.
struct GlanceCountdownSchedule: TimelineSchedule {
    var deadlines: [Date]
    var boundaries: [Date] = []
    var interval: TimeInterval = 30

    /// Past each second's turn by this much, so the time left read then has just dropped below it.
    static let settle: TimeInterval = 0.02
    /// How far ahead the next redraw must be, so a moment the schedule gave never comes back as its
    /// own next one through rounding.
    private static let margin: TimeInterval = 0.001

    func entries(from start: Date, mode: TimelineScheduleMode) -> Entries {
        Entries(cursor: start, start: start, deadlines: deadlines, boundaries: boundaries, interval: max(1, interval))
    }

    /// The moment after `moment` this schedule redraws at.
    func next(after moment: Date, start: Date) -> Date {
        Self.next(after: moment, start: start, deadlines: deadlines, boundaries: boundaries, interval: max(1, interval))
    }

    fileprivate static func next(after moment: Date, start: Date, deadlines: [Date], boundaries: [Date], interval: TimeInterval) -> Date {
        let steps = (moment.timeIntervalSince(start) / interval).rounded(.down) + 1
        var soonest = start.addingTimeInterval(steps * interval)
        for boundary in boundaries {
            let tick = boundary.addingTimeInterval(settle)
            if tick > moment.addingTimeInterval(margin), tick < soonest { soonest = tick }
        }
        for deadline in deadlines {
            let ahead = deadline.timeIntervalSince(moment) + settle
            guard ahead > margin else { continue }
            let secondsLeft = min((ahead - margin).rounded(.up) - 1, GlanceResetWording.soonSpan)
            guard secondsLeft >= 0 else { continue }
            let tick = deadline.addingTimeInterval(settle - secondsLeft)
            if tick > moment, tick < soonest { soonest = tick }
        }
        return soonest
    }

    struct Entries: Sequence, IteratorProtocol {
        var cursor: Date
        let start: Date
        let deadlines: [Date]
        let boundaries: [Date]
        let interval: TimeInterval

        mutating func next() -> Date? {
            let current = cursor
            cursor = GlanceCountdownSchedule.next(after: current, start: start, deadlines: deadlines, boundaries: boundaries, interval: interval)
            return current
        }
    }
}
