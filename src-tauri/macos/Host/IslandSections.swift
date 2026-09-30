import AppKit
import SwiftUI

extension GlanceView {
    /// Whether the view has something to draw at `now`: the quota list needs an account, the
    /// forecast needs the reset tracker, the upcoming list a limit with a reset time still ahead.
    func hasContent(in document: GlanceDocument, now: Date) -> Bool {
        switch self {
        case .quota: return !document.visibleProviders.isEmpty
        case .resets: return document.resets != nil
        case .upcoming: return !GlanceUpcomingLimit.list(document.providers, now: now).isEmpty
        }
    }
}

/// What the open island draws: behind a tab bar, the one tab picked (the first with something to
/// show until one is clicked), in full; stacked, every chosen view that has something to show.
struct IslandPlan: Equatable {
    /// The tab bar, empty when there is none.
    var tabs: [GlanceView]
    /// The views drawn, top to bottom.
    var sections: [GlanceView]
    var selected: GlanceView?

    static func make(_ document: GlanceDocument, now: Date, selected: GlanceView?) -> IslandPlan {
        let chosen = document.island.tabs
        if document.island.arrangement == .tabs, chosen.count > 1 {
            let active = selected.flatMap { chosen.contains($0) ? $0 : nil }
                ?? chosen.first { $0.hasContent(in: document, now: now) }
                ?? chosen[0]
            return IslandPlan(tabs: chosen, sections: [active], selected: active)
        }
        return IslandPlan(tabs: [], sections: chosen.filter { $0.hasContent(in: document, now: now) }, selected: nil)
    }

    /// Whether the open island has anything at all to show.
    static func hasContent(_ document: GlanceDocument, now: Date) -> Bool {
        document.island.tabs.contains { $0.hasContent(in: document, now: now) }
    }
}

/// How big the open island draws the reset calendar.
enum IslandCalendarSize: Equatable {
    /// Every week as a grid, weekdays down the side and months across the top.
    case grid
    /// The last four weeks in one row.
    case strip
    case none
}

struct IslandBudget: Equatable {
    /// Readings per account; `nil` shows every reading.
    var metricsPerAccount: Int?
    /// The accounts listed; the rest are counted in a `+N` line.
    var maxAccounts: Int?
    var upcoming: Int
    var calendar: IslandCalendarSize
    var rhythm: Bool
    var notes: Bool

    static let full = IslandBudget(metricsPerAccount: nil, maxAccounts: nil, upcoming: .max, calendar: .grid, rhythm: true, notes: true)

    var perAccount: Int { metricsPerAccount ?? Int.max }

    /// The limits coming back listed: what fits, never more than Settings allow (`0` for no cap).
    func upcoming(limit: Int) -> Int {
        limit > 0 ? min(upcoming, limit) : upcoming
    }
}

/// Where each tab of the open island's tab bar sits, so a click can pick it.
struct IslandTabFrames: PreferenceKey {
    static let space = "island"
    static var defaultValue: [GlanceView: CGRect] = [:]

    static func reduce(value: inout [GlanceView: CGRect], nextValue: () -> [GlanceView: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}

struct IslandFooterFrame: PreferenceKey {
    static var defaultValue: CGRect = .zero

    static func reduce(value: inout CGRect, nextValue: () -> CGRect) {
        value = nextValue()
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

/// The open island under the notch: the tab bar and the picked tab, or the chosen views stacked
/// and separated by thin rules, over the footer.
struct IslandDetails: View {
    let document: GlanceDocument
    let now: Date
    let topInset: CGFloat
    var budget: IslandBudget = .full
    var selected: GlanceView?
    var availableWidth: CGFloat = IslandGeometry.expandedWidth
    var viewportHeight: CGFloat?
    /// The reset view's folds, open or closed; the history folds after its first rows, as in the tab.
    var resetFolds = GlanceResetFolds(foldsHistory: true)
    /// A click on a fold of the reset view; the measuring copy leaves it out.
    var onResetFold: ((GlanceResetFold) -> Void)?

    var body: some View {
        let plan = IslandPlan.make(document, now: now, selected: selected)
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            if let active = plan.selected, !plan.tabs.isEmpty {
                IslandTabBar(tabs: plan.tabs, selected: active, labels: document.labels.tabs)
                    .padding(.horizontal, 16)
                    .padding(.top, 8)
                    .padding(.bottom, 4)
            }
            if viewportHeight != nil {
                ScrollView(.vertical) {
                    sections(plan)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(maxHeight: .infinity)
            } else {
                sections(plan)
            }
            footer
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(width: availableWidth, height: viewportHeight, alignment: .top)
    }

    private func sections(_ plan: IslandPlan) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if plan.sections.isEmpty {
                emptyLine(emptyText(nil))
            }
            ForEach(Array(plan.sections.enumerated()), id: \.element) { index, section in
                if index > 0 {
                    Rectangle()
                        .fill(IslandInk.divider)
                        .frame(height: 1)
                        .padding(.horizontal, 20)
                        .padding(.vertical, 12)
                }
                if section.hasContent(in: document, now: now) {
                    content(section)
                        .padding(.horizontal, 20)
                        .padding(.top, index == 0 ? 10 : 0)
                } else {
                    emptyLine(emptyText(section))
                }
            }
        }
        .padding(.bottom, 2)
    }

    @ViewBuilder
    private func content(_ section: GlanceView) -> some View {
        switch section {
        case .quota:
            IslandQuotaSection(document: document, now: now, budget: budget, availableWidth: max(1, availableWidth - 40))
        case .resets:
            if let resets = document.resets {
                IslandResetsSection(
                    resets: resets, labels: document.labels, now: now, budget: budget,
                    availableWidth: max(1, availableWidth - 40 - NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy)),
                    folds: resetFolds, onFold: onResetFold
                )
            }
        case .upcoming:
            IslandUpcomingSection(document: document, now: now, count: budget.upcoming(limit: document.island.upcomingLimit))
        }
    }

    private func emptyLine(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 12))
            .foregroundStyle(IslandInk.label)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 20)
            .padding(.top, 10)
    }

    /// Why a view (or, `nil`, the whole island) has nothing to show.
    private func emptyText(_ section: GlanceView?) -> String {
        let fallback = document.island.empty ?? document.labels.empty
        switch section ?? (document.island.tabs.count == 1 ? document.island.tabs[0] : nil) {
        case .resets:
            return document.labels.resetsOff.isEmpty ? fallback : document.labels.resetsOff
        case .upcoming:
            return document.labels.upcomingEmpty.isEmpty ? fallback : document.labels.upcomingEmpty
        case .quota, .none:
            return fallback
        }
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
        .contentShape(Rectangle())
        .background(
            GeometryReader { proxy in
                Color.clear.preference(
                    key: IslandFooterFrame.self,
                    value: proxy.frame(in: .named(IslandTabFrames.space))
                )
            }
        )
    }
}

/// The open island's tabs as a segmented bar; the picked one lit. A click lands through the
/// island's own click handling, which finds the tab by the frames reported here.
struct IslandTabBar: View {
    let tabs: [GlanceView]
    let selected: GlanceView
    let labels: GlanceTabLabels

    var body: some View {
        HStack(spacing: 2) {
            ForEach(tabs, id: \.self) { tab in
                let on = tab == selected
                Text(labels.name(tab))
                    .font(.system(size: 11.5, weight: on ? .semibold : .medium))
                    .foregroundStyle(on ? Color.white : IslandInk.caption)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
                    .padding(.horizontal, 8)
                    .frame(maxWidth: .infinity)
                    .frame(height: 24)
                    .background(
                        Capsule()
                            .fill(Color.white.opacity(on ? 0.17 : 0))
                    )
                    .contentShape(Capsule())
                    .background(
                        GeometryReader { proxy in
                            Color.clear.preference(
                                key: IslandTabFrames.self,
                                value: [tab: proxy.frame(in: .named(IslandTabFrames.space))]
                            )
                        }
                    )
            }
        }
        .padding(3)
        .background(
            Capsule()
                .fill(Color.white.opacity(0.07))
        )
    }
}

// MARK: Quota

struct IslandQuotaSection: View {
    let document: GlanceDocument
    let now: Date
    let budget: IslandBudget
    var availableWidth: CGFloat = 340

    var body: some View {
        let all = document.visibleProviders
        let shown = Array(all.prefix(budget.maxAccounts ?? all.count))
        let perAccount = budget.perAccount
        VStack(alignment: .leading, spacing: 10) {
            if shown.count > 1 && availableWidth >= 620 {
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

// MARK: Reset tracker

struct IslandResetsSection: View {
    @Environment(\.colorScheme) private var systemScheme
    let resets: GlanceResets
    let labels: GlanceLabels
    let now: Date
    let budget: IslandBudget
    var availableWidth: CGFloat = 340
    var folds = GlanceResetFolds(foldsHistory: true)
    var onFold: ((GlanceResetFold) -> Void)?

    var body: some View {
        let scheme = resets.theme == "dark" ? ColorScheme.dark : resets.theme == "light" ? .light : systemScheme
        GlanceResetContent(resets: resets, units: labels.units, now: now, availableWidth: max(1, availableWidth - 16), folds: folds, onFold: onFold)
            .padding(8)
            .background(GlanceResetPalette(scheme: scheme).background)
            .clipShape(RoundedRectangle(cornerRadius: 14))
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
