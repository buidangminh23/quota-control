import SwiftUI
import WidgetKit

/// Limits of one account that come back at the same moment, listed as one row: models of one plan
/// usually share their reset time, and twelve identical rows would crowd out every other account.
struct UpcomingGroup: Identifiable {
    let provider: GlanceProvider
    let metrics: [GlanceMetric]
    let at: Date

    var id: String { "\(provider.id)|\(metrics.map(\.id).joined(separator: ","))" }

    /// Every limit as a row of its own, soonest first.
    static func single(_ limits: [GlanceUpcomingLimit]) -> [UpcomingGroup] {
        limits.map { UpcomingGroup(provider: $0.provider, metrics: [$0.metric], at: $0.at) }
    }

    /// `limits` (soonest first) gathered by account and reset moment, keeping that order.
    static func grouped(_ limits: [GlanceUpcomingLimit]) -> [UpcomingGroup] {
        var groups: [UpcomingGroup] = []
        for limit in limits {
            if let index = groups.firstIndex(where: { $0.provider.id == limit.provider.id && $0.at == limit.at }) {
                let group = groups[index]
                groups[index] = UpcomingGroup(provider: group.provider, metrics: group.metrics + [limit.metric], at: group.at)
            } else {
                groups.append(UpcomingGroup(provider: limit.provider, metrics: [limit.metric], at: limit.at))
            }
        }
        return groups
    }
}

/// The heading of the list of limits coming back.
struct UpcomingHeading: View {
    let document: GlanceDocument

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: "clock.arrow.circlepath")
                .font(.system(size: WidgetScale.title, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(UpcomingText.title(document))
                .font(.system(size: WidgetScale.title, weight: .semibold))
                .lineLimit(1)
        }
    }
}

enum UpcomingText {
    static func title(_ document: GlanceDocument) -> String {
        if !document.labels.upcoming.isEmpty { return document.labels.upcoming }
        return document.isVietnamese ? "Sắp đặt lại" : "Coming back"
    }

    static func empty(_ document: GlanceDocument) -> String {
        if !document.labels.upcomingEmpty.isEmpty { return document.labels.upcomingEmpty }
        return document.isVietnamese ? "Chưa có hạn mức nào có giờ đặt lại." : "No limit has a reset time yet."
    }

    /// `+17 hạn mức khác`: limits the list left out.
    static func more(_ count: Int, document: GlanceDocument) -> String {
        "+\(count) \(document.isVietnamese ? "hạn mức khác" : "more")"
    }
}

/// The next limits to come back across the widget's accounts, soonest first, as many as fit: the
/// account and limit, its reading now, a live countdown and the clock time. Every limit gets its own
/// row when all of them fit; otherwise limits of one account due together share one.
struct UpcomingList: View {
    let document: GlanceDocument
    let now: Date
    let width: CGFloat
    var columns = 1

    var body: some View {
        let all = GlanceUpcomingLimit.list(document.widget.providers, now: now)
        let cap = document.widget.upcomingLimit
        let limits = cap > 0 ? Array(all.prefix(cap)) : all
        let beyond = all.count - limits.count
        let groups = UpcomingGroup.grouped(limits)
        if groups.isEmpty {
            Text(UpcomingText.empty(document))
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(.secondary)
                .lineLimit(3)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            let count = max(1, columns)
            let wide = columnWidth(width, count: count) >= 230
            ViewThatFits(in: .vertical) {
                if groups.count < limits.count {
                    planned(UpcomingGroup.single(limits), shown: limits.count, columns: count, wide: wide, beyond: beyond)
                }
                ForEach(Array(stride(from: groups.count, through: 1, by: -1)), id: \.self) { shown in
                    planned(groups, shown: shown, columns: count, wide: wide, beyond: beyond)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    /// `beyond`: limits past the count Settings allow, counted with the ones that did not fit.
    private func planned(_ groups: [UpcomingGroup], shown: Int, columns: Int, wide: Bool, beyond: Int) -> some View {
        let visible = Array(groups.prefix(shown))
        let perColumn = (visible.count + columns - 1) / columns
        let hidden = groups.dropFirst(shown).reduce(0) { $0 + $1.metrics.count } + beyond
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                ForEach(0..<columns, id: \.self) { column in
                    VStack(alignment: .leading, spacing: wide ? 7 : 6) {
                        ForEach(visible.dropFirst(column * perColumn).prefix(perColumn)) { group in
                            UpcomingRow(group: group, document: document, now: now, wide: wide)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            if hidden > 0 {
                MoreLine(text: UpcomingText.more(hidden, document: document))
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// One limit (or several of one account due together) coming back.
private struct UpcomingRow: View {
    let group: UpcomingGroup
    let document: GlanceDocument
    let now: Date
    let wide: Bool

    private var metric: GlanceMetric { group.metrics[0] }

    private var title: String {
        "\(accountName(group.provider, in: document.widget.providers)) · \(metric.label)"
    }

    var body: some View {
        if wide {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                mark
                Text(title)
                    .font(.system(size: WidgetScale.label))
                    .lineLimit(1)
                HiddenCount(count: group.metrics.count - 1)
                reading
                Spacer(minLength: 6)
                countdown
                clock
                    .frame(minWidth: 34, alignment: .trailing)
            }
        } else {
            VStack(alignment: .leading, spacing: 1) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    mark
                    Text(title)
                        .font(.system(size: WidgetScale.label))
                        .lineLimit(1)
                    Spacer(minLength: 3)
                    HiddenCount(count: group.metrics.count - 1)
                    reading
                }
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: 4) {
                        countdown
                        Spacer(minLength: 4)
                        clock
                    }
                    countdown
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(.leading, 14)
            }
        }
    }

    private var mark: some View {
        ProviderMark(mark: group.provider.mark)
            .foregroundStyle(group.provider.markTint)
            .frame(width: 10, height: 10)
            .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 1 }
    }

    private var reading: some View {
        Text(metric.value)
            .font(.system(size: WidgetScale.caption, weight: .semibold))
            .monospacedDigit()
            .foregroundStyle(GlancePalette.text(metric.severity, onDark: false))
            .lineLimit(1)
            .fixedSize()
    }

    private var countdown: some View {
        Text(group.at, style: .relative)
            .font(.system(size: WidgetScale.caption, weight: .medium))
            .monospacedDigit()
            .lineLimit(1)
            .fixedSize()
    }

    private var clock: some View {
        Text(ResetClock.text(group.at, now: now, document: document))
            .font(.system(size: WidgetScale.caption))
            .monospacedDigit()
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .fixedSize()
    }
}

struct UpcomingLayout: View {
    let document: GlanceDocument
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            UpcomingHeading(document: document)
            UpcomingList(document: document, now: now, width: size.width, columns: family == .systemExtraLarge ? 2 : 1)
            if family != .systemSmall {
                UpdatedFooter(document: document, now: now)
                    .padding(.top, -7)
            }
        }
    }
}
