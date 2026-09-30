import SwiftUI

/// What a limit's pace note and even-pace tick are worked out from (`GlancePace` in
/// `src/model/glance.ts`): `spent` for a limit used up; for a limit counting down, the share of it
/// used and its window's length in milliseconds, the window starting that long before its reset.
struct GlancePace: Decodable, Equatable {
    var spent: Bool? = nil
    var used: Double? = nil
    var period: Double? = nil
}

/// A limit's reading once its reset has passed, as the popup shows the window the moment it rolls
/// over: nothing used, no countdown, the next period's words (`Còn 100%`, `Đặt lại sau 5 giờ`).
struct GlanceAfterReset: Decodable, Equatable {
    var value: String
    var headline: String
    var fraction: Double
    var detail: String? = nil
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
    /// The metric as the popup's row reads it at `now`: once its reset has passed, the reading its
    /// window rolls over to; while it counts down, the meter's color, the note on its title line and
    /// the even-pace tick of its pace verdict at `now`, which the popup works out every time it
    /// renders.
    func reading(at now: Date, pacing: GlancePacing) -> GlanceMetric {
        var copy = self
        copy.note = nil
        copy.tick = nil
        if let resetsAt, resetsAt <= now, let after {
            copy.value = after.value
            copy.headline = after.headline
            copy.fraction = after.fraction
            copy.detail = after.detail
            copy.severity = after.severity ?? .normal
            copy.resetsAt = nil
            copy.pace = nil
            copy.after = nil
            return copy
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
    func reading(at now: Date, pacing: GlancePacing) -> GlanceProvider {
        var copy = self
        copy.metrics = metrics.map { $0.reading(at: now, pacing: pacing) }
        return copy
    }
}

extension GlanceDocument {
    /// The pace settings and words the document's limits are read with.
    var pacing: GlancePacing {
        GlancePacing(words: labels.pace, always: alwaysShowPacing == true, towardsUsed: displayMode == "used")
    }

    /// The document as the popup reads it at `now`: every limit, on the island, the widgets and
    /// beside the notch, rolled over once its reset has passed and paced at `now`. A surface draws
    /// this at each moment it draws, so its limits read as the popup's do then.
    func reading(at now: Date) -> GlanceDocument {
        let pacing = self.pacing
        var copy = self
        copy.providers = providers.map { $0.reading(at: now, pacing: pacing) }
        copy.widget.providers = widget.providers.map { $0.reading(at: now, pacing: pacing) }
        copy.island.wings = island.wings.map { $0.reading(at: now, pacing: pacing) }
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
            absolute: labels.resetAbsolute,
            days: labels.days,
            units: labels.units,
            locale: resolvedLocale,
            hour12: hour12
        )
    }

    /// The reset text of `metric`'s row at `now` as the popup's row says it, `short` without its
    /// verb; where reset times are switched off only a detail that is not one (`Chưa bắt đầu`,
    /// `Hạn mức 50 $`), which the popup's row shows in the same place.
    func resetText(for metric: GlanceMetric, now: Date, showsReset: Bool = true, short: Bool = false) -> String? {
        guard let at = metric.resetsAt else { return metric.detail }
        guard showsReset else { return nil }
        let wording = resetWording
        return short ? wording.span(at, now: now) : wording.line(at, now: now)
    }

    /// The line under `metric`'s countdown with the moment it comes back, where the popup's row has
    /// one: in Countdown, while the reset is ahead.
    func restoreText(for metric: GlanceMetric, now: Date, showsReset: Bool = true) -> String? {
        guard showsReset, let at = metric.resetsAt else { return nil }
        return resetWording.restore(at, now: now)
    }
}

/// When a limit comes back, worded as the popup's rows word it at a moment (`boundedTrailingText`,
/// `restoreText`), in the Reset Times setting's form. Countdown: `Đặt lại sau 2 giờ 5 phút` over
/// `Hồi lại lúc 13:05 · ngày mai`; Exact Time: `Đặt lại lúc 13:05 ngày mai`. In the last five minutes
/// the countdown says `Sắp đặt lại`. A document from before these words counts down to the end and
/// then says `Đang đặt lại…`, as it did.
struct GlanceResetWording {
    var exact: Bool
    var resetsIn: String
    var resetting: String
    var soon: String?
    var restoresAt: String?
    var absolute: GlanceDayWords?
    var days: GlanceDayWords?
    var units: GlanceUnits
    var locale: Locale
    var hour12: Bool?

    /// The last stretch before a reset that the popup's countdown calls soon.
    static let soonSpan: TimeInterval = 5 * 60

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
    fileprivate func words(_ at: Date, now: Date) -> [String] {
        [line(at, now: now), span(at, now: now), restore(at, now: now) ?? ""]
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
    /// when every change in it has happened. A widget gives each its own timeline entry, so the words
    /// move with the clock as the popup's do, which reads them every 30 seconds.
    func rowTicks(_ providers: [GlanceProvider], after now: Date, until end: Date, calendar: Calendar = .current) -> [Date] {
        let wording = resetWording
        let pacing = self.pacing
        let midnight = GlanceDays.nextMidnight(after: now, calendar: calendar)
        var moments: [Date] = []
        for provider in providers {
            if let row = provider.resetRow(at: now), let countdown = row.countdown, countdown.at > now {
                moments += GlanceTicks.changes(after: now, until: min(end, countdown.at), anchor: countdown.at) {
                    countdown.text(now: $0, units: labels.units)
                }
            }
            for metric in provider.metrics {
                guard let at = metric.resetsAt, at > now else { continue }
                let last = min(end, at)
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
        return GlanceTicks.lastPerMinute(moments)
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
