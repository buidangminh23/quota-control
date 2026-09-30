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

    /// Whether the view says, as the popup's tab does, that its data is on its way or could not
    /// load: the reset view while its tracker is on but has nothing yet.
    func isPending(in document: GlanceDocument) -> Bool {
        self == .resets && document.resets == nil && document.resetsPending != nil
    }
}

/// The island's last picked tab, kept in the app's defaults so a restart or an update reopens it;
/// a tab that is no longer chosen falls back to the first one with something to show.
enum IslandTabMemory {
    private static let key = "island.selectedTab"

    static func load(_ defaults: UserDefaults = .standard) -> GlanceView? {
        defaults.string(forKey: key).flatMap(GlanceView.init(rawValue:))
    }

    static func save(_ tab: GlanceView?, _ defaults: UserDefaults = .standard) {
        if let tab {
            defaults.set(tab.rawValue, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }
}

/// What the open island draws: behind a tab bar, the one tab picked (the first with something to
/// show until one is clicked), in full; stacked, every chosen view that has something to show or
/// says why it has nothing yet, the reset view first so its forecast is not below every account.
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
        let shown = chosen.filter { $0.hasContent(in: document, now: now) || $0.isPending(in: document) }
        return IslandPlan(tabs: [], sections: shown.filter { $0 == .resets } + shown.filter { $0 != .resets }, selected: nil)
    }

    /// Whether the open island has anything at all to show.
    static func hasContent(_ document: GlanceDocument, now: Date) -> Bool {
        document.island.tabs.contains { $0.hasContent(in: document, now: now) || $0.isPending(in: document) }
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

/// Text on the open island's themed panels, in the app's theme as the popup draws it: on the dark
/// panel the island's own inks, on the light panel the popup's light label, secondary and tertiary.
struct IslandPanelInk {
    let dark: Bool

    init(_ scheme: ColorScheme) {
        dark = scheme == .dark
    }

    var primary: Color { dark ? Color.white : Color.black.opacity(0.88) }
    var label: Color { dark ? IslandInk.label : Color.black.opacity(0.62) }
    var caption: Color { dark ? IslandInk.caption : Color.black.opacity(0.62) }
    var faint: Color { dark ? IslandInk.faint : Color.black.opacity(0.56) }
    var warning: Color { dark ? IslandInk.warning : GlanceHeaderInk.warning(dark: false) }

    /// A mark's color on the panel, as the popup picks it for its theme.
    func mark(_ provider: GlanceProvider) -> Color {
        dark ? provider.islandMarkColor : provider.markColor(in: .light)
    }
}

/// The open island's sizes and spacing, as the popup's Density setting sets them: Compact steps each
/// down by the popup's compact change (`tokens.css`), keeping small text at the popup's 10-point
/// caption size.
struct IslandDensity {
    var name: CGFloat
    var mark: CGFloat
    var plan: CGFloat
    var label: CGFloat
    var support: CGFloat
    var note: CGFloat
    var caption: CGFloat
    var meter: CGFloat
    var quotaGap: CGFloat
    var accountGap: CGFloat
    var rowGap: CGFloat
    var lineGap: CGFloat
    var upcomingGap: CGFloat
    var upcomingText: CGFloat
    var upcomingLine: CGFloat
    var upcomingMark: CGFloat
    var panelPadding: CGFloat
    var sectionGap: CGFloat
    var cardGap: CGFloat
    var resetRow: GlanceResetRowSizes

    static let regular = IslandDensity(
        name: 13, mark: 14, plan: 10.5, label: 12, support: 12, note: 11, caption: 10, meter: 4,
        quotaGap: 10, accountGap: 13, rowGap: 9, lineGap: 4, upcomingGap: 8, upcomingText: 11.5, upcomingLine: 10.5, upcomingMark: 12,
        panelPadding: 8, sectionGap: 10, cardGap: 12, resetRow: .island
    )

    static let compact = IslandDensity(
        name: 12, mark: 12, plan: 10, label: 11, support: 11, note: 10, caption: 10, meter: 3,
        quotaGap: 5, accountGap: 7, rowGap: 4, lineGap: 3, upcomingGap: 6, upcomingText: 10.5, upcomingLine: 10, upcomingMark: 11,
        panelPadding: 6, sectionGap: 6, cardGap: 6, resetRow: .islandCompact
    )

    static func of(_ document: GlanceDocument) -> IslandDensity {
        document.isCompact ? .compact : .regular
    }
}

/// The open island's sections sit on the popup's page in the app's theme, as the reset view always
/// has: the popup's white or dark page, its ink, rounded inside the black island.
struct IslandThemedPanel: ViewModifier {
    let scheme: ColorScheme
    let padding: CGFloat

    func body(content: Content) -> some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(GlanceResetPalette(scheme: scheme).background)
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .environment(\.colorScheme, scheme)
    }
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
    /// The reset view's folds, open or closed; the history and the limit changes fold after their
    /// first rows, as in the tab.
    var resetFolds = GlanceResetFolds(foldsLists: true)
    /// A click on a fold of the reset view; the measuring copy leaves it out.
    var onResetFold: ((GlanceResetFold) -> Void)?
    @Environment(\.colorScheme) private var systemScheme

    /// The app's theme, or the Mac's appearance while it follows the Mac, as the popup draws in.
    private var scheme: ColorScheme { document.forcedScheme ?? systemScheme }

    private var density: IslandDensity { .of(document) }

    /// The width inside a section's panel: the island's side margins and the panel's own padding
    /// taken off, and the scroller's room while the island scrolls.
    private var panelWidth: CGFloat {
        max(1, availableWidth - 40 - density.panelPadding * 2 - NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy))
    }

    var body: some View {
        let plan = IslandPlan.make(document, now: now, selected: selected)
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            if let active = plan.selected, !plan.tabs.isEmpty {
                IslandTabBar(tabs: plan.tabs, selected: active, labels: document.labels.tabs, resets: document.resets)
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
                if section.hasContent(in: document, now: now) {
                    content(section, named: plan.tabs.isEmpty)
                        .modifier(IslandThemedPanel(scheme: scheme, padding: density.panelPadding))
                        .padding(.horizontal, 20)
                        .padding(.top, index == 0 ? 10 : density.sectionGap)
                } else {
                    emptyLine(emptyText(section))
                }
            }
        }
        .padding(.bottom, 2)
    }

    /// A view's content; `named`, the reset view names its tracker itself, as no tab bar does.
    @ViewBuilder
    private func content(_ section: GlanceView, named: Bool) -> some View {
        switch section {
        case .quota:
            IslandQuotaSection(document: document, now: now, budget: budget, availableWidth: panelWidth)
        case .resets:
            if let resets = document.resets {
                IslandResetsSection(
                    resets: resets, labels: document.labels, now: now, budget: budget,
                    availableWidth: panelWidth, spacing: density.cardGap,
                    folds: resetFolds, onFold: onResetFold, showsHeading: named
                )
            }
        case .upcoming:
            IslandUpcomingSection(document: document, now: now, count: budget.upcoming(limit: document.island.upcomingLimit))
        }
    }

    private func emptyLine(_ line: (text: String, failed: Bool)) -> some View {
        Text(line.text)
            .font(.system(size: 12))
            .foregroundStyle(line.failed ? IslandInk.warning : IslandInk.label)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 20)
            .padding(.top, 10)
    }

    /// Why a view (or, `nil`, the whole island) has nothing to show; the reset view says what the
    /// Reset tab says while its tracker loads, in the notice color once it could not load.
    private func emptyText(_ section: GlanceView?) -> (text: String, failed: Bool) {
        let fallback = document.island.empty ?? document.labels.empty
        switch section ?? (document.island.tabs.count == 1 ? document.island.tabs[0] : nil) {
        case .resets:
            if let pending = document.resetsPending { return (pending.text, pending.failed == true) }
            return (document.labels.resetsOff.isEmpty ? fallback : document.labels.resetsOff, false)
        case .upcoming:
            return (document.labels.upcomingEmpty.isEmpty ? fallback : document.labels.upcomingEmpty, false)
        case .quota, .none:
            return (fallback, false)
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

/// The open island's tabs as a segmented bar; the picked one lit, the reset tab led by its
/// tracker's mark as the Reset tab's switch shows it. A click lands through the island's own click
/// handling, which finds the tab by the frames reported here.
struct IslandTabBar: View {
    let tabs: [GlanceView]
    let selected: GlanceView
    let labels: GlanceTabLabels
    /// The tracker the reset tab shows, whose mark leads its name.
    var resets: GlanceResets? = nil

    var body: some View {
        HStack(spacing: 2) {
            ForEach(tabs, id: \.self) { tab in
                let on = tab == selected
                HStack(spacing: 4) {
                    if tab == .resets, let resets {
                        ProviderMark(mark: resets.mark)
                            .foregroundStyle(resets.tint)
                            .frame(width: 11, height: 11)
                    }
                    Text(labels.name(tab))
                        .font(.system(size: 11.5, weight: on ? .semibold : .medium))
                        .foregroundStyle(on ? Color.white : IslandInk.caption)
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)
                }
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

/// The island's accounts as the popup reads them at `now`: each limit rolled over once its reset has
/// passed and paced at `now` (`GlanceDocument.reading(at:)`).
struct IslandQuotaSection: View {
    let document: GlanceDocument
    let now: Date
    let budget: IslandBudget
    var availableWidth: CGFloat = 340

    var body: some View {
        let document = self.document.reading(at: now)
        let all = document.visibleProviders
        let shown = Array(all.prefix(budget.maxAccounts ?? all.count))
        let perAccount = budget.perAccount
        let restores = document.island.shows.resets && document.labels.restoresAt != nil && !document.resetWording.exact
        VStack(alignment: .leading, spacing: IslandDensity.of(document).quotaGap) {
            if shown.count > 1 && availableWidth >= 620 {
                let split = Self.balancedSplit(shown, perAccount: perAccount, shows: document.island.shows, restores: restores, now: now)
                HStack(alignment: .top, spacing: 18) {
                    column(Array(shown[..<split]), document: document, perAccount: perAccount)
                    column(Array(shown[split...]), document: document, perAccount: perAccount)
                }
            } else {
                column(shown, document: document, perAccount: perAccount)
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

    private func column(_ providers: [GlanceProvider], document: GlanceDocument, perAccount: Int) -> some View {
        VStack(alignment: .leading, spacing: IslandDensity.of(document).accountGap) {
            ForEach(providers) { provider in
                IslandAccount(provider: provider, document: document, now: now, perAccount: perAccount)
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    /// Where the second column starts: keeps the accounts in reading order, top to bottom and
    /// left to right, with the two columns as close in height as the accounts allow. `restores`: a
    /// limit counting down has the line with the moment it comes back under it.
    static func balancedSplit(_ providers: [GlanceProvider], perAccount: Int, shows: GlanceShows, restores: Bool = false, now: Date) -> Int {
        let weights = providers.map { weight($0, perAccount: perAccount, shows: shows, restores: restores, now: now) }
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

    /// The rough height of one account, in lines: the plan period's corner takes the email's line
    /// when there is none; a limit's headline shares its line with the reset countdown, and the
    /// moment it comes back takes a smaller line under them.
    private static func weight(_ provider: GlanceProvider, perAccount: Int, shows: GlanceShows, restores: Bool, now: Date) -> CGFloat {
        var lines: CGFloat = 1.3
        if (shows.account && provider.account != nil) || provider.term != nil { lines += 0.9 }
        if let row = provider.resetRow(at: now) { lines += row.note == nil ? 2.1 : 3 }
        if provider.metrics.isEmpty { return lines + 1.2 }
        for metric in provider.metrics.prefix(perAccount) {
            lines += 1.2
            if metric.fraction != nil { lines += 1.5 }
            if restores, metric.resetsAt != nil { lines += 0.8 }
        }
        if provider.metrics.count > perAccount { lines += 1 }
        return lines + 0.9
    }
}

/// One account: its header, the row its card starts with, and its first readings, with a `+N` for
/// the readings left out.
struct IslandAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument
    let now: Date
    let perAccount: Int
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let density = IslandDensity.of(document)
        VStack(alignment: .leading, spacing: density.rowGap) {
            IslandAccountHeader(provider: provider, shows: document.island.shows, words: document.labels.planTerm, now: now, density: density)
            if let row = provider.resetRow(at: now) {
                IslandResetRow(row: row, document: document, now: now)
            }
            if provider.metrics.isEmpty {
                GlanceNoticeRow(text: provider.notice ?? document.labels.noData, onDark: scheme == .dark, size: density.note)
            }
            ForEach(provider.metrics.prefix(perAccount)) { metric in
                IslandMetricRow(metric: metric, document: document, now: now, showsReset: document.island.shows.resets)
            }
            if provider.metrics.count > perAccount {
                IslandMoreLine(text: "+\(provider.metrics.count - perAccount)")
            }
        }
    }
}

/// The row a Codex or Claude account starts with, on the island: hovering it shows the post and how
/// its time was read, and while the popup's row opens the Reset tab, pressing it opens the popup on
/// that tracker's Reset tab.
struct IslandResetRow: View {
    let row: GlanceResetRow
    let document: GlanceDocument
    let now: Date

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let content = GlanceResetRowView(row: row, document: document, now: now, sizes: IslandDensity.of(document).resetRow, onDark: scheme == .dark)
        if row.opens == true {
            Button {
                IslandActions.shared.send(.openResets(row.tracker))
            } label: {
                content.contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(row.details)
        } else {
            content.help(row.details)
        }
    }
}

extension GlanceProvider {
    /// The mark's color on the island's black, as the popup's dark theme draws it: the brand color,
    /// or the secondary text color for a brand without a color or without a mark.
    var islandMarkColor: Color { hasBrandTint ? tint : IslandInk.caption }
}

/// An account's header as the popup's card header draws it in the dark theme: the mark in its
/// brand color, the name, the plan in plain secondary text, `Dữ liệu cũ` and the warning triangle
/// (its reason on hover), the email under, and the plan period in the right corner.
struct IslandAccountHeader: View {
    let provider: GlanceProvider
    let shows: GlanceShows
    /// The plan period's words, sent while an account has one.
    let words: GlancePlanTermWords?
    let now: Date
    var density = IslandDensity.regular
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let ink = IslandPanelInk(scheme)
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    ProviderMark(mark: provider.mark, brand: provider.brand)
                        .foregroundStyle(ink.mark(provider))
                        .frame(width: density.mark, height: density.mark)
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Text(provider.name)
                            .font(.system(size: density.name, weight: .semibold))
                            .foregroundStyle(ink.primary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        if shows.plan, let plan = provider.plan {
                            Text(plan)
                                .font(.system(size: density.plan))
                                .foregroundStyle(ink.caption)
                                .lineLimit(1)
                                .layoutPriority(1)
                        }
                        if let outdated = provider.outdated {
                            Text(outdated)
                                .font(.system(size: density.plan))
                                .foregroundStyle(ink.faint)
                                .lineLimit(1)
                        }
                    }
                    if let problem = provider.problem {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: 10))
                            .foregroundStyle(ink.warning)
                            .help(problem)
                            .accessibilityLabel(problem)
                    }
                }
                if shows.account, let account = provider.account {
                    Text(account)
                        .font(.system(size: density.plan))
                        .foregroundStyle(ink.caption)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .padding(.leading, density.mark + 6)
                }
            }
            if let words, let lines = provider.term?.lines(now: now, words: words) {
                Spacer(minLength: 6)
                GlancePlanTermCorner(left: lines.left, day: lines.day, soon: lines.soon, onDark: ink.dark, size: density.plan)
            }
        }
    }
}

/// One reading as the popup's row reads it (`metric` read at `now`). A limit: its title in bold with
/// its pace note at the other end, over the meter, which alone carries the pace color and the
/// even-pace tick; then its headline in the text color with when it comes back on the right in the
/// secondary color, in the Reset Times setting's form, and under a countdown the moment it comes
/// back. Where reset times are switched off, a detail that is not one still shows. A metric without a
/// limit: its title with its value on the right, after the expiry dot of a reset credit.
struct IslandMetricRow: View {
    let metric: GlanceMetric
    let document: GlanceDocument
    let now: Date
    var showsReset = true
    @Environment(\.colorScheme) private var scheme

    private var density: IslandDensity { .of(document) }
    private var ink: IslandPanelInk { IslandPanelInk(scheme) }

    var body: some View {
        if let fraction = metric.fraction {
            VStack(alignment: .leading, spacing: density.lineGap) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    title
                    if let note = metric.note {
                        Spacer(minLength: 6)
                        GlancePaceNoteView(note: note, severity: metric.severity, onDark: ink.dark, size: density.note)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: ink.dark, height: density.meter, tick: metric.tick)
                VStack(alignment: .trailing, spacing: 2) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        headline
                        if let reset = document.resetText(for: metric, now: now, showsReset: showsReset) {
                            Spacer(minLength: 8)
                            Text(reset)
                                .font(.system(size: density.support))
                                .foregroundStyle(ink.caption)
                                .monospacedDigit()
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if let restore = document.restoreText(for: metric, now: now, showsReset: showsReset) {
                        Text(restore)
                            .font(.system(size: density.caption))
                            .foregroundStyle(ink.faint)
                            .monospacedDigit()
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
            }
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                title
                Spacer(minLength: 4)
                GlanceValueWithDot(metric: metric, now: now, onDark: ink.dark) { headline }
            }
        }
    }

    private var title: some View {
        Text(metric.label)
            .font(.system(size: density.label, weight: .semibold))
            .foregroundStyle(ink.primary)
            .lineLimit(1)
            .truncationMode(.tail)
    }

    private var headline: some View {
        Text(metric.headline)
            .font(.system(size: density.label))
            .foregroundStyle(ink.primary)
            .monospacedDigit()
            .lineLimit(1)
            .fixedSize()
    }
}

/// A count of what the island left out: `+2`, `+3 tài khoản khác`.
struct IslandMoreLine: View {
    let text: String
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        Text(text)
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(IslandPanelInk(scheme).faint)
            .lineLimit(1)
    }
}

// MARK: Reset tracker

/// The reset tracker's cards; `IslandDetails` puts them on the themed panel with the other views.
struct IslandResetsSection: View {
    let resets: GlanceResets
    let labels: GlanceLabels
    let now: Date
    let budget: IslandBudget
    var availableWidth: CGFloat = 340
    var spacing: CGFloat = 12
    var folds = GlanceResetFolds(foldsLists: true)
    var onFold: ((GlanceResetFold) -> Void)?
    /// Names the tracker above the cards, for an island without a tab bar to name it.
    var showsHeading = false

    var body: some View {
        GlanceResetContent(resets: resets, units: labels.units, now: now, availableWidth: availableWidth, folds: folds, onFold: onFold, showsHeading: showsHeading, spacing: spacing)
    }
}

// MARK: Upcoming limits

/// The next limits to come back across the island's accounts, soonest first, each as the popup's
/// card reads it: the account's mark, its name (with its email when another account shares it) and
/// the limit, the headline on the right; under them when it comes back as the row says it in the
/// Reset Times setting's form, with the clock time and its day beside a countdown.
struct IslandUpcomingSection: View {
    let document: GlanceDocument
    let now: Date
    let count: Int
    @Environment(\.colorScheme) private var scheme

    private var density: IslandDensity { .of(document) }
    private var ink: IslandPanelInk { IslandPanelInk(scheme) }

    var body: some View {
        let wording = document.resetWording
        let limits = Array(GlanceUpcomingLimit.list(document.providers, now: now).prefix(max(count, 1)))
        VStack(alignment: .leading, spacing: density.upcomingGap) {
            if !document.labels.upcoming.isEmpty {
                Text(document.labels.upcoming)
                    .font(.system(size: density.upcomingText - 0.5, weight: .medium))
                    .foregroundStyle(ink.label)
                    .lineLimit(1)
            }
            ForEach(limits) { limit in
                row(limit, wording: wording)
            }
        }
    }

    private func row(_ limit: GlanceUpcomingLimit, wording: GlanceResetWording) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline, spacing: 7) {
                ProviderMark(mark: limit.provider.mark, brand: limit.provider.brand)
                    .foregroundStyle(ink.mark(limit.provider))
                    .frame(width: density.upcomingMark, height: density.upcomingMark)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
                Text("\(accountName(limit.provider, in: document.providers)) · \(limit.metric.label)")
                    .font(.system(size: density.upcomingText))
                    .foregroundStyle(ink.dark ? Color.white.opacity(0.88) : Color.black.opacity(0.88))
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 8)
                Text(limit.metric.headline)
                    .font(.system(size: density.upcomingText, weight: .semibold))
                    .foregroundStyle(ink.primary)
                    .monospacedDigit()
                    .lineLimit(1)
                    .fixedSize()
            }
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(wording.line(limit.at, now: now))
                    .foregroundStyle(ink.label)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 8)
                if !wording.exact {
                    Text(document.dayLabel(limit.at, now: now))
                        .foregroundStyle(ink.caption)
                        .lineLimit(1)
                        .fixedSize()
                }
            }
            .font(.system(size: density.upcomingLine))
            .monospacedDigit()
            .padding(.leading, density.upcomingMark + 7)
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
                ProviderMark(mark: mark, brand: alert.brand)
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
