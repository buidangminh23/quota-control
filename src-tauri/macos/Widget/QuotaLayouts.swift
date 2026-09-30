import SwiftUI
import WidgetKit

/// How many rows each account gets when `total` rows are dealt round-robin: every account its first
/// row, then every account with more its second, and so on, so no account crowds out the others.
/// `sizes` holds how many rows each account could show.
func dealRows(_ sizes: [Int], total: Int) -> [Int] {
    var counts = Array(repeating: 0, count: sizes.count)
    var left = total
    var round = 0
    let most = sizes.max() ?? 0
    while left > 0, round < most {
        for index in sizes.indices where left > 0 && sizes[index] > round {
            counts[index] += 1
            left -= 1
        }
        round += 1
    }
    return counts
}

/// How much of each account a plan draws besides its rows, richest first.
enum QuotaDetail: Int, Hashable {
    /// Every detail and, under each countdown in a wide column, the moment the limit comes back.
    case restore
    /// The email under the heading and every row's reset countdown.
    case full
    /// No email line.
    case noAccounts
    /// No email line and no reset countdown lines.
    case bare

    var showsAccounts: Bool { self == .restore || self == .full }
}

/// One way to fit the accounts: how much of each is drawn, how many metrics each shown account
/// keeps, or one line per account.
struct QuotaPlan: Hashable {
    let counts: [Int]
    let detail: QuotaDetail
    let condensed: Bool

    var accounts: Int { counts.count }

    /// Richest first: every account with as many rows as could be dealt, one row fewer at a time,
    /// first with every detail, then without emails, then without reset lines; then one line per
    /// account, then fewer accounts. An account's rows are the one its card starts with at `now`
    /// (its reset row), then its metrics, or one for the notice saying why it has none. `most` caps
    /// the rows tried, so a tall list does not try hundreds of plans that cannot fit. The lines with
    /// the moment each limit comes back (`restore`) are tried with every row only: rows come before
    /// them.
    static func candidates(_ providers: [GlanceProvider], now: Date, most: Int, details: [QuotaDetail] = [.full, .noAccounts, .bare]) -> [QuotaPlan] {
        let sizes = providers.map { $0.rowCount(at: now) }
        let total = sizes.reduce(0, +)
        let top = max(providers.count, min(total, most))
        var plans: [QuotaPlan] = []
        for detail in details {
            for rows in stride(from: top, through: providers.count, by: -1) {
                if detail == .restore, rows < top { break }
                plans.append(QuotaPlan(counts: dealRows(sizes, total: rows), detail: detail, condensed: false))
            }
        }
        for accounts in stride(from: providers.count, through: 1, by: -1) {
            plans.append(QuotaPlan(counts: Array(repeating: 1, count: accounts), detail: .bare, condensed: true))
        }
        return plans
    }

    /// The most rows `height` could hold in `columns` columns if every row took `row` points.
    static func mostRows(height: CGFloat, columns: Int, row: CGFloat) -> Int {
        max(1, Int(height / row)) * max(columns, 1)
    }
}

/// How many columns of accounts a size holds.
func quotaColumns(_ family: WidgetFamily, style: QuotaWidgetStyle, accounts: Int) -> Int {
    let most: Int
    switch family {
    case .systemMedium: most = 2
    case .systemLarge: most = accounts > 3 ? 2 : 1
    case .systemExtraLarge: most = style == .compact ? 3 : 2
    default: most = 1
    }
    return max(1, min(most, accounts))
}

/// The width of one of `count` columns across `width`.
func columnWidth(_ width: CGFloat, count: Int) -> CGFloat {
    (width - CGFloat(count - 1) * WidgetScale.columnSpacing) / CGFloat(max(count, 1))
}

/// The accounts of `plan` placed in `columns` columns, each in turn going to the column with the
/// fewest rows so far, so the columns end near the same height and each reads top to bottom.
func columnIndices(_ plan: QuotaPlan, columns: Int) -> [[Int]] {
    var placed = Array(repeating: [Int](), count: max(columns, 1))
    var loads = Array(repeating: 0.0, count: max(columns, 1))
    for index in 0..<plan.accounts {
        let column = loads.indices.min { loads[$0] < loads[$1] || (loads[$0] == loads[$1] && $0 < $1) } ?? 0
        placed[column].append(index)
        loads[column] += Double(plan.condensed ? 1 : plan.counts[index]) + 1.5
    }
    return placed
}

/// When a metric comes back, as the popup's row words it at the entry's moment in the Reset Times
/// setting's form (`Đặt lại sau 2 giờ 5 phút`, `Đặt lại lúc 13:05 ngày mai`, `Sắp đặt lại`), without
/// its verb when `short`; or the metric's own detail. The timeline has an entry for each minute the
/// words change, so they move with the clock as the popup's do.
struct ResetText: View {
    let text: String

    init?(metric: GlanceMetric, document: GlanceDocument, now: Date, showsReset: Bool = true, short: Bool = false) {
        guard let text = document.resetText(for: metric, now: now, showsReset: showsReset, short: short) else { return nil }
        self.text = text
    }

    var body: some View {
        Text(text)
            .monospacedDigit()
            .lineLimit(1)
            .truncationMode(.tail)
    }
}

extension GlanceResetRowSizes {
    /// An account's reset row on a widget, at the sizes of its metric rows.
    static let widget = GlanceResetRowSizes(avatar: 12, title: WidgetScale.label, value: WidgetScale.label, caption: WidgetScale.caption)
    /// The same row in a compact list, beside its one-line metrics.
    static let widgetLine = GlanceResetRowSizes(avatar: 10, title: WidgetScale.label, value: WidgetScale.label, caption: WidgetScale.caption)
}

/// The row a Codex or Claude account starts with, on a widget, its countdown worded at the entry's
/// moment: pressing it asks the app to show that tracker's Reset tab, as the popup's row does while
/// that tab is on. A compact list keeps its first line.
struct WidgetResetRow: View {
    let row: GlanceResetRow
    let document: GlanceDocument
    let now: Date
    var compact = false

    var body: some View {
        let content = GlanceResetRowView(
            row: row, document: document, now: now, sizes: compact ? .widgetLine : .widget, showsCaption: !compact
        )
        if row.opens == true {
            Button(intent: PressGlanceAction(.openResets(row.tracker), step: .press)) { content }
                .buttonStyle(.plain)
        } else {
            content
        }
    }
}

// MARK: Details

struct DetailsLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    var body: some View {
        let columns = quotaColumns(family, style: .details, accounts: providers.count)
        let inline = columnWidth(size.width, count: columns) >= 280
        let most = QuotaPlan.mostRows(height: size.height, columns: columns, row: 20)
        let restores = inline && document.widget.shows.resets && providers.contains { provider in
            provider.metrics.contains { document.restoreText(for: $0, now: now) != nil }
        }
        let details: [QuotaDetail] = restores ? [.restore, .full, .noAccounts, .bare] : [.full, .noAccounts, .bare]
        VStack(alignment: .leading, spacing: 0) {
            ViewThatFits(in: .vertical) {
                ForEach(Array(QuotaPlan.candidates(providers, now: now, most: most, details: details).enumerated()), id: \.offset) { _, plan in
                    planned(plan, columns: columns, inline: inline)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            UpdatedFooter(document: document, now: now)
        }
    }

    private func planned(_ plan: QuotaPlan, columns: Int, inline: Bool) -> some View {
        let shown = Array(providers.prefix(plan.accounts))
        let hidden = Array(providers.dropFirst(plan.accounts))
        return VStack(alignment: .leading, spacing: 7) {
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                ForEach(Array(columnIndices(plan, columns: columns).enumerated()), id: \.offset) { _, indices in
                    VStack(alignment: .leading, spacing: plan.condensed ? 8 : 10) {
                        ForEach(indices, id: \.self) { index in
                            if plan.condensed {
                                CondensedAccount(provider: shown[index], document: document, now: now)
                            } else {
                                DetailedAccount(provider: shown[index], count: plan.counts[index], detail: plan.detail, document: document, now: now, inline: inline)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            if !hidden.isEmpty {
                MoreLine(text: MoreText.accounts(hidden, labels: document.labels))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// An account's header with its first `count` rows (the row its card starts with, then its meters),
/// a `+N` for the metrics left out, or the line saying why it has none. A wide column has room for
/// the plan period's corner and for every countdown in full words.
private struct DetailedAccount: View {
    let provider: GlanceProvider
    let count: Int
    let detail: QuotaDetail
    let document: GlanceDocument
    let now: Date
    let inline: Bool

    var body: some View {
        let shows = document.widget.shows
        let row = count > 0 ? provider.resetRow(at: now) : nil
        let metrics = Array(provider.metrics.prefix(max(0, count - (row == nil ? 0 : 1))))
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .top, spacing: 4) {
                GlanceProviderHeader(
                    provider: provider,
                    shows: GlanceShows(account: shows.account && detail.showsAccounts, plan: shows.plan, resets: shows.resets),
                    size: WidgetScale.mark,
                    term: inline ? document.termContext(now: now) : nil
                )
                .frame(maxWidth: .infinity, alignment: .leading)
                HiddenCount(count: provider.metrics.count - metrics.count)
            }
            if let row {
                WidgetResetRow(row: row, document: document, now: now)
            }
            if provider.metrics.isEmpty {
                GlanceNoticeRow(text: provider.notice ?? document.labels.noData, size: WidgetScale.caption)
            }
            ForEach(metrics) { metric in
                QuotaMetricRow(
                    metric: metric, document: document, now: now, showsReset: shows.resets, inline: inline,
                    dense: detail == .bare, restores: detail == .restore
                )
            }
        }
    }
}

extension GlanceDocument {
    /// What a header needs to word the plan period's corner at `now`, when any account has one.
    func termContext(now: Date) -> GlanceTermContext? {
        labels.planTerm.map { GlanceTermContext(words: $0, now: now) }
    }
}

/// A metric as the popup's row reads it (`metric` read at the entry's moment). A limit in a wide
/// column: its title in bold with its pace note at the other end, over the meter, which alone carries
/// the pace color and the even-pace tick; then its headline in the text color with its reset text on
/// the right, in the Reset Times setting's form, and with `restores` the moment it comes back under
/// them. A narrow column has no room for the headline and the reset text on one line, so there the
/// headline sits on the title's line (with the pace note where it fits) and the reset text under the
/// meter, left out in the densest plan. Where reset times are switched off, a detail that is not one
/// still shows. A metric without a limit: its title with its value on the right, after the expiry
/// dot of a reset credit.
private struct QuotaMetricRow: View {
    let metric: GlanceMetric
    let document: GlanceDocument
    let now: Date
    var showsReset = true
    var inline = false
    var dense = false
    var restores = false
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let reset = ResetText(metric: metric, document: document, now: now, showsReset: showsReset)
        if let fraction = metric.fraction, inline {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    title
                    if let note = metric.note {
                        Spacer(minLength: 4)
                        noteView(note)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                GlanceMeter(fraction: fraction, severity: metric.severity, height: 4, tick: metric.tick)
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    headline(noted: metric.note != nil)
                    if let reset {
                        Spacer(minLength: 4)
                        reset
                            .font(.system(size: WidgetScale.value))
                            .foregroundStyle(.secondary)
                    }
                }
                if restores, let restore = document.restoreText(for: metric, now: now, showsReset: showsReset) {
                    Text(restore)
                        .font(.system(size: WidgetScale.caption))
                        .monospacedDigit()
                        .foregroundStyle(GlanceRowInk.tertiary(dark: colorScheme == .dark))
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: 3) {
                ViewThatFits(in: .horizontal) {
                    if let note = metric.note {
                        titleLine(note: note)
                    }
                    titleLine(note: nil)
                }
                if let fraction = metric.fraction {
                    GlanceMeter(fraction: fraction, severity: metric.severity, height: 4, tick: metric.tick)
                    if let reset, !dense {
                        reset
                            .font(.system(size: WidgetScale.caption))
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    /// The title with, on the right, the pace note where it fits, then the headline.
    private func titleLine(note: GlancePaceNote?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            title
            Spacer(minLength: 4)
            if let note {
                noteView(note)
            }
            GlanceValueWithDot(metric: metric, now: now, dotSize: 5) { headline(noted: note != nil) }
        }
    }

    private func noteView(_ note: GlancePaceNote) -> some View {
        GlancePaceNoteView(note: note, severity: metric.severity, size: WidgetScale.caption)
    }

    private var title: some View {
        Text(metric.label)
            .font(.system(size: WidgetScale.label, weight: .semibold))
            .foregroundStyle(.primary)
            .lineLimit(1)
    }

    /// The headline, after the mark a limit close to or out of its limit keeps where the widget is
    /// drawn without color, unless its pace note (`noted`) already says so in words.
    private func headline(noted: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 3) {
            if !noted {
                ColorlessSeverityMark(severity: metric.severity, size: WidgetScale.value)
            }
            Text(metric.headline)
                .font(.system(size: WidgetScale.value))
                .monospacedDigit()
                .foregroundStyle(.primary)
                .lineLimit(1)
        }
        .fixedSize()
    }
}

/// An account in one line: the mark and name, the first reading and its meter, or what it says in
/// place of readings.
private struct CondensedAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                GlanceProviderHeader(provider: provider, shows: GlanceShows(account: false, plan: false, resets: false), size: WidgetScale.mark)
                Spacer(minLength: 4)
                if let metric = provider.metrics.first {
                    GlanceValueWithDot(metric: metric, now: now, dotSize: 5) {
                        ColorlessSeverityMark(severity: metric.severity, size: WidgetScale.value)
                        Text(metric.value)
                            .font(.system(size: WidgetScale.value, weight: .semibold))
                            .monospacedDigit()
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                    }
                    .fixedSize()
                } else {
                    Text(provider.notice ?? document.labels.noData)
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            if let metric = provider.metrics.first, let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, height: 4, tick: metric.tick)
            }
        }
    }
}

// MARK: Compact

struct CompactLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            CompactQuota(
                document: document,
                providers: providers,
                now: now,
                width: size.width,
                height: size.height,
                columns: quotaColumns(family, style: .compact, accounts: providers.count),
                showsAccounts: family == .systemLarge || family == .systemExtraLarge
            )
            if family != .systemSmall {
                UpdatedFooter(document: document, now: now)
            }
        }
    }
}

/// The accounts as short lines, as many as fit the height offered: a header per account, then one
/// line per metric; one line per account when that is all that fits.
struct CompactQuota: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let now: Date
    let width: CGFloat
    let height: CGFloat
    let columns: Int
    var showsAccounts = false

    var body: some View {
        let count = max(1, min(columns, providers.count))
        let column = columnWidth(width, count: count)
        let style = CompactRowStyle(wide: column >= 200, resets: column >= 280 && document.widget.shows.resets, terms: column >= 280)
        let most = QuotaPlan.mostRows(height: height, columns: count, row: 14)
        ViewThatFits(in: .vertical) {
            ForEach(Array(QuotaPlan.candidates(providers, now: now, most: most, details: [.full, .noAccounts]).enumerated()), id: \.offset) { _, plan in
                planned(plan, columns: count, style: style)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func planned(_ plan: QuotaPlan, columns: Int, style: CompactRowStyle) -> some View {
        let shown = Array(providers.prefix(plan.accounts))
        let hidden = Array(providers.dropFirst(plan.accounts))
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                ForEach(Array(columnIndices(plan, columns: columns).enumerated()), id: \.offset) { _, indices in
                    VStack(alignment: .leading, spacing: plan.condensed ? 4 : 7) {
                        ForEach(indices, id: \.self) { index in
                            CompactAccount(
                                provider: shown[index],
                                count: plan.counts[index],
                                headed: !plan.condensed,
                                showsAccount: plan.detail == .full && (showsAccounts || sharesHeading(shown[index], in: providers)),
                                style: style,
                                document: document,
                                now: now
                            )
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            if !hidden.isEmpty {
                MoreLine(text: MoreText.accounts(hidden, labels: document.labels))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// How much a compact line holds in its column.
struct CompactRowStyle {
    /// A meter and the reading beside the name instead of a hairline under it.
    let wide: Bool
    /// The reset countdown at the end of the line.
    let resets: Bool
    /// The plan period's corner in an account's header.
    var terms = false
}

/// An account as short lines: its header, then the row its card starts with in one line, then one
/// line per metric; or, headless, one line naming the account beside its first metric.
private struct CompactAccount: View {
    let provider: GlanceProvider
    let count: Int
    let headed: Bool
    let showsAccount: Bool
    let style: CompactRowStyle
    let document: GlanceDocument
    let now: Date
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let row = headed && count > 0 ? provider.resetRow(at: now) : nil
        let metrics = Array(provider.metrics.prefix(max(0, count - (row == nil ? 0 : 1))))
        VStack(alignment: .leading, spacing: 4) {
            if headed {
                HStack(alignment: .top, spacing: 4) {
                    GlanceProviderHeader(
                        provider: provider,
                        shows: GlanceShows(account: showsAccount && document.widget.shows.account, plan: document.widget.shows.plan, resets: false),
                        size: WidgetScale.mark,
                        term: style.terms ? document.termContext(now: now) : nil
                    )
                    .frame(maxWidth: .infinity, alignment: .leading)
                    HiddenCount(count: provider.metrics.count - metrics.count)
                }
            }
            if let row {
                WidgetResetRow(row: row, document: document, now: now, compact: true)
            }
            if provider.metrics.isEmpty {
                if headed {
                    GlanceNoticeRow(text: provider.notice ?? document.labels.noData, size: WidgetScale.caption)
                } else {
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        mark
                        Text(provider.name)
                            .font(.system(size: WidgetScale.label, weight: .semibold))
                            .lineLimit(1)
                        problemMark
                        Spacer(minLength: 4)
                        Text(provider.notice ?? document.labels.noData)
                            .font(.system(size: WidgetScale.caption))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
            }
            ForEach(metrics) { metric in
                line(metric)
            }
        }
    }

    @ViewBuilder
    private func line(_ metric: GlanceMetric) -> some View {
        let title = headed ? metric.label : "\(provider.name) · \(metric.label)"
        if style.wide {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if !headed { mark }
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(title)
                        .font(.system(size: WidgetScale.label, weight: headed ? .semibold : .regular))
                        .foregroundStyle(headed ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                        .lineLimit(1)
                    if !headed { problemMark }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Group {
                    if let fraction = metric.fraction {
                        GlanceMeter(fraction: fraction, severity: metric.severity, height: 4, tick: metric.tick)
                    } else {
                        Color.clear.frame(height: 4)
                    }
                }
                .frame(width: 44)
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] + 3 }
                value(metric)
                    .frame(minWidth: 40, alignment: .trailing)
                if style.resets {
                    Group {
                        if let reset = ResetText(metric: metric, document: document, now: now, short: true) {
                            reset
                        }
                    }
                    .font(.system(size: WidgetScale.caption))
                    .foregroundStyle(.secondary)
                    .frame(width: 84, alignment: .trailing)
                }
            }
        } else if !headed {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    mark
                    Text(provider.name)
                        .font(.system(size: WidgetScale.label, weight: .semibold))
                        .lineLimit(1)
                    problemMark
                    Spacer(minLength: 4)
                    value(metric)
                }
                Text(metric.label)
                    .font(.system(size: WidgetScale.caption))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .padding(.leading, 14)
                if let fraction = metric.fraction {
                    GlanceMeter(fraction: fraction, severity: metric.severity, height: 2.5, tick: metric.tick)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(title)
                        .font(.system(size: WidgetScale.label, weight: headed ? .semibold : .regular))
                        .foregroundStyle(headed ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    value(metric)
                }
                if let fraction = metric.fraction {
                    GlanceMeter(fraction: fraction, severity: metric.severity, height: 2.5, tick: metric.tick)
                }
            }
        }
    }

    private func value(_ metric: GlanceMetric) -> some View {
        GlanceValueWithDot(metric: metric, now: now, dotSize: 5) {
            ColorlessSeverityMark(severity: metric.severity, size: WidgetScale.value)
            Text(metric.value)
                .font(.system(size: WidgetScale.value, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(.primary)
                .lineLimit(1)
        }
        .fixedSize()
    }

    private var mark: some View {
        ProviderMarkView(provider: provider)
            .frame(width: 10, height: 10)
            .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
    }

    /// The header's warning triangle, beside the name on a line that stands in for the header.
    @ViewBuilder
    private var problemMark: some View {
        if let problem = provider.problem {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(GlanceHeaderInk.warning(dark: colorScheme == .dark))
                .accessibilityLabel(problem)
        }
    }
}

// MARK: Rings

/// One ring: a metric of an account, or an account that has none and says why.
struct RingTile: Identifiable {
    let id: String
    let provider: GlanceProvider
    let metric: GlanceMetric?
}

/// The grid of rings that shows the most tiles at a readable size: its tile count, columns, ring
/// diameter and whether each ring has room for its reset countdown.
struct RingFit {
    let tiles: Int
    let columns: Int
    let ring: CGFloat
    let cell: CGFloat
    let resets: Bool

    static let gap: CGFloat = 8
    /// Two caption lines under a ring.
    static let captionHeight: CGFloat = 26
    static let resetHeight: CGFloat = 12
    /// A reset countdown only fits under a ring this wide.
    static let resetRing: CGFloat = 64

    static func bounds(_ family: WidgetFamily) -> (least: CGFloat, most: CGFloat) {
        switch family {
        case .systemSmall: return (56, 96)
        case .systemMedium: return (50, 84)
        case .systemLarge: return (46, 110)
        default: return (50, 110)
        }
    }

    /// The most tiles (up to `count`) that fit `size` with rings at least the family's smallest,
    /// then the biggest rings for that many, in as few rows as that size allows. When some are left
    /// out, a `MoreLine` takes a line.
    static func best(count: Int, in size: CGSize, family: WidgetFamily, resets: Bool) -> RingFit {
        let (least, most) = bounds(family)
        for tiles in stride(from: count, through: 1, by: -1) {
            let height = size.height - (tiles < count ? WidgetScale.moreHeight + 4 : 0)
            var best: RingFit?
            for columns in 1...tiles {
                let rows = (tiles + columns - 1) / columns
                let cell = (size.width - CGFloat(columns - 1) * gap) / CGFloat(columns)
                let perRow = (height - CGFloat(rows - 1) * gap) / CGFloat(rows)
                var ring = min(cell - 6, perRow - captionHeight)
                var fitsReset = false
                if resets {
                    let smaller = min(cell - 6, perRow - captionHeight - resetHeight)
                    if smaller >= resetRing {
                        ring = smaller
                        fitsReset = true
                    }
                }
                ring = min(ring, most)
                if ring >= least, ring >= (best?.ring ?? 0) {
                    best = RingFit(tiles: tiles, columns: columns, ring: ring.rounded(.down), cell: cell, resets: fitsReset)
                }
            }
            if let best { return best }
        }
        return RingFit(tiles: 1, columns: 1, ring: least, cell: size.width, resets: false)
    }
}

struct RingsLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    private var sources: [[RingTile]] {
        providers.map { provider in
            if provider.metrics.isEmpty {
                return [RingTile(id: provider.id, provider: provider, metric: nil)]
            }
            return provider.metrics.map { RingTile(id: "\(provider.id)|\($0.id)", provider: provider, metric: $0) }
        }
    }

    var body: some View {
        let sources = self.sources
        let total = sources.reduce(0) { $0 + $1.count }
        let footer = family == .systemSmall ? 0 : WidgetScale.footerHeight
        let fit = RingFit.best(
            count: total,
            in: CGSize(width: size.width, height: size.height - footer),
            family: family,
            resets: document.widget.shows.resets
        )
        let counts = dealRows(sources.map(\.count), total: fit.tiles)
        let shown = zip(sources, counts).flatMap { Array($0.prefix($1)) }
        let hidden = zip(providers, zip(sources, counts)).map { ($0, $1.0.count - $1.1) }.filter { $0.1 > 0 }
        let rows = (shown.count + fit.columns - 1) / fit.columns
        VStack(spacing: 0) {
            VStack(spacing: RingFit.gap) {
                ForEach(0..<rows, id: \.self) { row in
                    HStack(alignment: .top, spacing: RingFit.gap) {
                        ForEach(0..<fit.columns, id: \.self) { column in
                            let index = row * fit.columns + column
                            if index < shown.count {
                                RingTileView(tile: shown[index], document: document, providers: providers, now: now, fit: fit)
                            } else {
                                Color.clear.frame(width: fit.cell, height: 1)
                            }
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if !hidden.isEmpty {
                MoreLine(text: MoreText.metrics(hidden, document: document))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.top, 4)
            }
            if family != .systemSmall {
                UpdatedFooter(document: document, now: now)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

private struct RingTileView: View {
    let tile: RingTile
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let now: Date
    let fit: RingFit
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let ring = fit.ring
        VStack(spacing: 3) {
            GlanceRing(fraction: tile.metric?.fraction, severity: tile.metric?.severity ?? .none, lineWidth: max(3.5, ring / 13)) {
                VStack(spacing: ring > 60 ? 2 : 1) {
                    ProviderMarkView(provider: tile.provider)
                        .frame(width: ring * 0.22, height: ring * 0.22)
                    if let metric = tile.metric {
                        GlanceValueWithDot(metric: metric, now: now, dotSize: max(4, ring * 0.07)) {
                            ColorlessSeverityMark(severity: metric.severity, size: max(10, ring * 0.16))
                            Text(metric.value)
                                .font(.system(size: max(10, ring * 0.2), weight: .bold))
                                .monospacedDigit()
                                .foregroundStyle(.primary)
                                .lineLimit(1)
                                .minimumScaleFactor(0.6)
                        }
                    } else if tile.provider.problem != nil {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: max(9, ring * 0.16)))
                            .foregroundStyle(GlanceHeaderInk.warning(dark: colorScheme == .dark))
                    } else {
                        Text(Self.noReading)
                            .font(.system(size: max(10, ring * 0.2), weight: .bold))
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(ring * 0.15)
            }
            .frame(width: ring, height: ring)
            Text(caption)
                .font(.system(size: WidgetScale.footnote, weight: .medium))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .frame(width: fit.cell, height: RingFit.captionHeight - 4, alignment: .top)
            if fit.resets, document.widget.shows.resets, let metric = tile.metric,
               let reset = ResetText(metric: metric, document: document, now: now, short: true) {
                reset
                    .font(.system(size: WidgetScale.footnote - 0.5))
                    .foregroundStyle(.tertiary)
                    .frame(width: fit.cell)
            }
        }
        .frame(width: fit.cell)
    }

    /// What the popup's rows read without a reading.
    private static let noReading = "—"

    private var caption: String {
        guard let metric = tile.metric else {
            return "\(tile.provider.name): \(tile.provider.notice ?? document.labels.noData)"
        }
        if sharesHeading(tile.provider, in: providers) {
            return "\(accountName(tile.provider, in: providers)) · \(metric.label)"
        }
        return metric.label
    }
}
