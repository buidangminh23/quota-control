import SwiftUI

/// A part of the open island, in the order it is drawn.
enum IslandSection: Hashable {
    case quota
    case resets
    case upcoming

    /// The sections Settings switched on that have something to show at `now`: the quota list
    /// needs an account, the forecast needs the reset tracker, the upcoming list a limit with a
    /// reset time still ahead.
    static func visible(in document: GlanceDocument, now: Date) -> [IslandSection] {
        let sections = document.island.sections
        var result: [IslandSection] = []
        if sections.quota && !document.visibleProviders.isEmpty {
            result.append(.quota)
        }
        if sections.resets && document.resets != nil {
            result.append(.resets)
        }
        if sections.upcoming && !GlanceUpcomingLimit.list(document.providers, now: now).isEmpty {
            result.append(.upcoming)
        }
        return result
    }
}

/// How much of each section the open island draws. The island measures the budgets of
/// `ladder(accounts:)` in turn and keeps the first that fits on the screen.
struct IslandBudget: Equatable {
    /// Readings per account; `nil` picks by the number of accounts.
    var metricsPerAccount: Int?
    /// The accounts listed; the rest are counted in a `+N` line.
    var maxAccounts: Int?
    var upcoming: Int
    var calendar: Bool
    var notes: Bool

    static let full = IslandBudget(metricsPerAccount: nil, maxAccounts: nil, upcoming: 6, calendar: true, notes: true)

    /// Budgets from the roomiest to the tightest: fewer upcoming limits, then fewer readings per
    /// account and no calendar, then one reading per account without notes, then fewer accounts.
    static func ladder(accounts: Int) -> [IslandBudget] {
        var steps: [IslandBudget] = [
            .full,
            IslandBudget(metricsPerAccount: nil, maxAccounts: nil, upcoming: 4, calendar: true, notes: true),
            IslandBudget(metricsPerAccount: 2, maxAccounts: nil, upcoming: 4, calendar: false, notes: true),
            IslandBudget(metricsPerAccount: 1, maxAccounts: nil, upcoming: 3, calendar: false, notes: false),
        ]
        var shown = accounts - 1
        while shown >= 1 {
            steps.append(IslandBudget(metricsPerAccount: 1, maxAccounts: shown, upcoming: 2, calendar: false, notes: false))
            shown -= shown > 6 ? 2 : 1
        }
        return steps
    }

    /// Readings per account for `count` accounts: all four for one or two, two for up to four,
    /// one beyond that, never more than the budget allows.
    func metrics(forAccounts count: Int) -> Int {
        let automatic = count <= 2 ? 4 : (count <= 4 ? 2 : 1)
        return min(automatic, metricsPerAccount ?? automatic)
    }
}

/// Text on the black island: bright for values, dimmer for labels and captions.
enum IslandInk {
    static let label = Color.white.opacity(0.74)
    static let caption = Color.white.opacity(0.62)
    static let faint = Color.white.opacity(0.5)
    static let divider = Color.white.opacity(0.12)
    static let warning = Color(red: 1.0, green: 0.62, blue: 0.04)
    static let positive = Color(red: 0.2, green: 0.84, blue: 0.4)
    static let notice = Color(red: 1.0, green: 0.76, blue: 0.2)
}

/// The open island under the notch: the sections Settings chose (quota limits, the Codex reset
/// forecast, the next limits to come back), separated by thin rules, over the footer.
struct IslandDetails: View {
    let document: GlanceDocument
    let now: Date
    let topInset: CGFloat
    var budget: IslandBudget = .full

    var body: some View {
        let sections = IslandSection.visible(in: document, now: now)
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            if sections.isEmpty {
                Text(emptyText)
                    .font(.system(size: 12))
                    .foregroundStyle(IslandInk.label)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 20)
                    .padding(.top, 10)
            }
            ForEach(Array(sections.enumerated()), id: \.element) { index, section in
                if index > 0 {
                    Rectangle()
                        .fill(IslandInk.divider)
                        .frame(height: 1)
                        .padding(.horizontal, 20)
                        .padding(.vertical, 12)
                }
                content(section)
                    .padding(.horizontal, 20)
                    .padding(.top, index == 0 ? 10 : 0)
            }
            footer
        }
    }

    @ViewBuilder
    private func content(_ section: IslandSection) -> some View {
        switch section {
        case .quota:
            IslandQuotaSection(document: document, now: now, budget: budget)
        case .resets:
            if let resets = document.resets {
                IslandResetsSection(resets: resets, labels: document.labels, now: now, budget: budget)
            }
        case .upcoming:
            IslandUpcomingSection(document: document, now: now, count: budget.upcoming)
        }
    }

    private var emptyText: String {
        let sections = document.island.sections
        if sections.resets, document.resets == nil, !sections.quota, !document.labels.resetsOff.isEmpty {
            return document.labels.resetsOff
        }
        if sections.upcoming, !sections.quota, !sections.resets, !document.labels.upcomingEmpty.isEmpty {
            return document.labels.upcomingEmpty
        }
        return document.island.empty ?? document.labels.empty
    }

    private var footer: some View {
        HStack(spacing: 8) {
            Text("\(document.labels.updated) \(GlanceFormat.time(document.generatedAt, locale: document.resolvedLocale, hour12: document.hour12))")
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(document.labels.open)
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .font(.system(size: 10.5))
        .foregroundStyle(IslandInk.faint)
        .padding(.horizontal, 20)
        .padding(.top, 14)
        .padding(.bottom, 14)
    }
}

// MARK: Quota

/// The island's accounts with their meters and countdowns: one column for up to three accounts,
/// two balanced columns beyond that, fewer readings per account as the list grows. Readings and
/// accounts left out are counted, never dropped silently.
struct IslandQuotaSection: View {
    let document: GlanceDocument
    let now: Date
    let budget: IslandBudget

    var body: some View {
        let all = document.visibleProviders
        let shown = Array(all.prefix(budget.maxAccounts ?? all.count))
        let perAccount = budget.metrics(forAccounts: all.count)
        VStack(alignment: .leading, spacing: 10) {
            if shown.count > 3 {
                let split = Self.balancedSplit(shown, perAccount: perAccount, shows: document.island.shows)
                HStack(alignment: .top, spacing: 18) {
                    column(Array(shown[..<split]), perAccount: perAccount)
                    column(Array(shown[split...]), perAccount: perAccount)
                }
            } else {
                column(shown, perAccount: perAccount)
            }
            if all.count > shown.count {
                IslandMoreLine(text: more(all.count - shown.count))
            }
        }
    }

    private func more(_ count: Int) -> String {
        let word = document.labels.more
        return word.isEmpty ? "+\(count)" : "+\(count) \(word)"
    }

    private func column(_ providers: [GlanceProvider], perAccount: Int) -> some View {
        VStack(alignment: .leading, spacing: 13) {
            ForEach(providers) { provider in
                IslandAccount(provider: provider, document: document, now: now, perAccount: perAccount)
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    /// Where the second column starts: keeps the accounts in reading order, top to bottom and
    /// left to right, with the two columns as close in height as the accounts allow.
    static func balancedSplit(_ providers: [GlanceProvider], perAccount: Int, shows: GlanceShows) -> Int {
        let weights = providers.map { weight($0, perAccount: perAccount, shows: shows) }
        let total = weights.reduce(0, +)
        var best = (index: 1, tallest: CGFloat.infinity)
        var left: CGFloat = 0
        for index in 1..<providers.count {
            left += weights[index - 1]
            let tallest = max(left, total - left)
            if tallest < best.tallest {
                best = (index, tallest)
            }
        }
        return best.index
    }

    /// The rough height of one account, in lines.
    private static func weight(_ provider: GlanceProvider, perAccount: Int, shows: GlanceShows) -> CGFloat {
        var lines: CGFloat = 1.3
        if shows.account && provider.account != nil { lines += 0.9 }
        if provider.metrics.isEmpty { return lines + 1.2 }
        for metric in provider.metrics.prefix(perAccount) {
            lines += 1.2
            if metric.fraction != nil { lines += 0.5 }
            if shows.resets && (metric.resetsAt != nil || metric.detail != nil) { lines += 1 }
        }
        if provider.metrics.count > perAccount { lines += 1 }
        return lines + 0.9
    }
}

/// One account: its header and its first readings, with a `+N` for the readings left out.
struct IslandAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument
    let now: Date
    let perAccount: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            IslandAccountHeader(provider: provider, shows: document.island.shows)
            if provider.metrics.isEmpty {
                GlanceNoticeRow(text: provider.notice ?? document.labels.noData, onDark: true, size: 11)
            }
            ForEach(provider.metrics.prefix(perAccount)) { metric in
                IslandMetricRow(metric: metric, labels: document.labels, now: now, showsReset: document.island.shows.resets)
            }
            if provider.metrics.count > perAccount {
                IslandMoreLine(text: "+\(provider.metrics.count - perAccount)")
            }
        }
    }
}

/// An account's mark in its brand color beside its name and plan, with its email under.
struct IslandAccountHeader: View {
    let provider: GlanceProvider
    let shows: GlanceShows

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                ProviderMark(mark: provider.mark)
                    .foregroundStyle(provider.tint)
                    .frame(width: 14, height: 14)
                Text(provider.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.white)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if shows.plan, let plan = provider.plan {
                    GlancePlanBadge(text: plan, onDark: true, size: 13)
                }
            }
            if shows.account, let account = provider.account {
                Text(account)
                    .font(.system(size: 10.5))
                    .foregroundStyle(IslandInk.caption)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, 20)
            }
        }
    }
}

/// One reading: its label and headline over the meter, and when it comes back beneath.
struct IslandMetricRow: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date
    var showsReset = true

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(metric.label)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(IslandInk.label)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 4)
                Text(metric.headline)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(GlancePalette.text(metric.severity, onDark: true))
                    .monospacedDigit()
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            if let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: true, height: 4)
            }
            if showsReset, metric.resetsAt != nil || metric.detail != nil {
                GlanceResetText(metric: metric, labels: labels, now: now)
                    .font(.system(size: 10.5))
                    .foregroundStyle(IslandInk.caption)
            }
        }
    }
}

/// A count of what the island left out: `+2`, `+3 tài khoản khác`.
struct IslandMoreLine: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(IslandInk.faint)
            .lineLimit(1)
    }
}

// MARK: Codex resets

/// The Codex free-reset tracker: the announced reset (or the time since the last one), the chance
/// of a reset over the next one, three and seven days, and the last four weeks of resets.
struct IslandResetsSection: View {
    let resets: GlanceResets
    let labels: GlanceLabels
    let now: Date
    let budget: IslandBudget

    var body: some View {
        VStack(alignment: .leading, spacing: 11) {
            header
            if let upcoming = resets.upcoming(at: now) {
                announced(upcoming)
            } else if let latest = resets.latest {
                last(latest)
            }
            if !resets.forecast.isEmpty {
                chances
            }
            if budget.calendar, let calendar = resets.calendar, calendar.weeks > 0 {
                IslandResetStrip(calendar: calendar, tint: resets.tint)
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                ProviderMark(mark: resets.mark)
                    .foregroundStyle(resets.tint)
                    .frame(width: 14, height: 14)
                Text(resets.title)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.white)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text(resets.source)
                    .font(.system(size: 10))
                    .foregroundStyle(IslandInk.faint)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            if let stale = resets.stale {
                GlanceNoticeRow(text: stale, onDark: true, size: 10.5)
            }
        }
    }

    private func announced(_ upcoming: GlanceUpcomingReset) -> some View {
        let lines = upcoming.lines(now: now, units: labels.units)
        let tone = upcoming.tone == .positive ? IslandInk.positive : IslandInk.notice
        return HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 3) {
                Text(upcoming.title)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(tone)
                    .lineLimit(1)
                Text(lines.value)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(lines.awaiting ? IslandInk.label : Color.white)
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                if !lines.caption.isEmpty {
                    Text(lines.caption)
                        .font(.system(size: 10.5))
                        .foregroundStyle(IslandInk.caption)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if budget.notes, let note = upcoming.note {
                    Text(note)
                        .font(.system(size: 10.5))
                        .foregroundStyle(IslandInk.faint)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(.leading, 12)
            .background(alignment: .leading) {
                Capsule()
                    .fill(tone)
                    .frame(width: 3)
                    .padding(.vertical, 2)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 8)
        .padding(.horizontal, 10)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.white.opacity(0.07))
        )
    }

    private func last(_ latest: GlanceLatestReset) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(latest.label)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(IslandInk.label)
                    .lineLimit(1)
                Spacer(minLength: 6)
                Text("\(latest.kindLabel) · \(latest.when)")
                    .font(.system(size: 10.5))
                    .foregroundStyle(IslandInk.caption)
                    .lineLimit(1)
            }
            Text(latest.since.text(now: now, units: labels.units))
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.white)
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
    }

    private var chances: some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(resets.forecastTitle)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(IslandInk.label)
                .lineLimit(1)
            HStack(alignment: .top, spacing: 0) {
                ForEach(resets.forecast) { chance in
                    VStack(spacing: 5) {
                        IslandChanceRing(fraction: chance.fraction, tint: resets.tint) {
                            Text("\(chance.percent)%")
                                .font(.system(size: 11, weight: .semibold))
                                .foregroundStyle(Color.white)
                                .monospacedDigit()
                                .lineLimit(1)
                                .minimumScaleFactor(0.7)
                        }
                        .frame(width: 38, height: 38)
                        Text(chance.label)
                            .font(.system(size: 10.5))
                            .foregroundStyle(IslandInk.caption)
                            .lineLimit(1)
                            .minimumScaleFactor(0.85)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            if budget.notes, !resets.forecastNote.isEmpty {
                Text(resets.forecastNote)
                    .font(.system(size: 10))
                    .foregroundStyle(IslandInk.faint)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A chance as a ring in the tracker's color, with the percentage inside.
struct IslandChanceRing<Center: View>: View {
    let fraction: Double
    let tint: Color
    var lineWidth: CGFloat = 3.5
    @ViewBuilder var center: () -> Center

    var body: some View {
        ZStack {
            Circle()
                .stroke(Color.white.opacity(0.14), lineWidth: lineWidth)
            if fraction > 0 {
                Circle()
                    .trim(from: 0, to: min(max(fraction, 0.03), 1))
                    .stroke(tint, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            center()
                .padding(lineWidth + 1)
        }
        .padding(lineWidth / 2)
    }
}

/// The last four weeks of resets in one row, a week per group, Monday first, with a legend.
struct IslandResetStrip: View {
    let calendar: GlanceResetCalendar
    let tint: Color

    static let weeks = 4
    private static let cell: CGFloat = 8

    var body: some View {
        let rows = calendar.weekRows(last: Self.weeks)
        let first = max(calendar.weeks - rows.count, 0) * 7
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                ForEach(Array(rows.enumerated()), id: \.offset) { week, cells in
                    HStack(spacing: 2) {
                        ForEach(Array(cells.enumerated()), id: \.offset) { day, cell in
                            square(cell, today: first + week * 7 + day == calendar.today)
                        }
                    }
                }
            }
            HStack(spacing: 10) {
                legend(color: tint, text: calendar.legend.regular)
                if rows.joined().contains(.banked) {
                    legend(color: IslandInk.warning, text: calendar.legend.banked)
                }
                HStack(spacing: 4) {
                    RoundedRectangle(cornerRadius: 2, style: .continuous)
                        .strokeBorder(Color.white.opacity(0.85), lineWidth: 1)
                        .frame(width: Self.cell, height: Self.cell)
                    Text(calendar.legend.today)
                }
            }
            .font(.system(size: 10))
            .foregroundStyle(IslandInk.caption)
            .lineLimit(1)
        }
    }

    private func square(_ cell: GlanceResetCalendar.Cell, today: Bool) -> some View {
        RoundedRectangle(cornerRadius: 2, style: .continuous)
            .fill(color(cell))
            .frame(width: Self.cell, height: Self.cell)
            .overlay(
                RoundedRectangle(cornerRadius: 2, style: .continuous)
                    .strokeBorder(Color.white.opacity(today ? 0.85 : 0), lineWidth: 1)
            )
    }

    private func color(_ cell: GlanceResetCalendar.Cell) -> Color {
        switch cell {
        case .regular: return tint
        case .banked: return IslandInk.warning
        case .none: return Color.white.opacity(0.12)
        case .future: return Color.white.opacity(0.04)
        }
    }

    private func legend(color: Color, text: String) -> some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 2, style: .continuous)
                .fill(color)
                .frame(width: Self.cell, height: Self.cell)
            Text(text)
        }
    }
}

// MARK: Upcoming limits

/// The next limits to come back across the island's accounts, soonest first: the account's mark,
/// its name and the reading, the time left and the clock time.
struct IslandUpcomingSection: View {
    let document: GlanceDocument
    let now: Date
    let count: Int

    var body: some View {
        let limits = Array(GlanceUpcomingLimit.list(document.providers, now: now).prefix(max(count, 1)))
        VStack(alignment: .leading, spacing: 7) {
            if !document.labels.upcoming.isEmpty {
                Text(document.labels.upcoming)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(IslandInk.label)
                    .lineLimit(1)
            }
            ForEach(limits) { limit in
                row(limit)
            }
        }
    }

    private func row(_ limit: GlanceUpcomingLimit) -> some View {
        HStack(spacing: 7) {
            ProviderMark(mark: limit.provider.mark)
                .foregroundStyle(limit.provider.tint)
                .frame(width: 12, height: 12)
            Text("\(limit.provider.name) · \(limit.metric.label)")
                .font(.system(size: 11.5))
                .foregroundStyle(Color.white.opacity(0.88))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 8)
            Text(GlanceFormat.span(from: now, to: limit.at, units: document.labels.units))
                .font(.system(size: 11.5, weight: .semibold))
                .foregroundStyle(Color.white)
                .monospacedDigit()
                .lineLimit(1)
                .layoutPriority(1)
            Text(GlanceFormat.time(limit.at, locale: document.resolvedLocale, hour12: document.hour12))
                .font(.system(size: 10.5))
                .foregroundStyle(IslandInk.caption)
                .monospacedDigit()
                .lineLimit(1)
                .frame(minWidth: 36, alignment: .trailing)
                .layoutPriority(1)
        }
    }
}

// MARK: Alert

/// A short notice: a limit about to run out, or one that came back.
struct IslandAlertView: View {
    let alert: GlanceAlert
    let mark: GlanceMark?
    let tint: Color
    let topInset: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            HStack(alignment: .top, spacing: 12) {
                ProviderMark(mark: mark)
                    .foregroundStyle(tint)
                    .frame(width: 22, height: 22)
                VStack(alignment: .leading, spacing: 3) {
                    Text(alert.title)
                        .font(.system(size: 13.5, weight: .semibold))
                        .foregroundStyle(GlancePalette.text(alert.severity, onDark: true))
                        .fixedSize(horizontal: false, vertical: true)
                    Text(alert.body)
                        .font(.system(size: 12))
                        .foregroundStyle(Color.white.opacity(0.78))
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 20)
            .padding(.top, 10)
            .padding(.bottom, 16)
        }
    }
}
