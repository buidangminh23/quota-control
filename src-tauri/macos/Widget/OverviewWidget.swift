import SwiftUI
import WidgetKit

/// How much of the reset tracker an overview has room for.
enum ResetsSummaryStyle {
    /// A narrow column: the announced reset or the chance of one, and the time since the last.
    case narrow
    /// A band across the widget: the announced or last reset beside the three chances.
    case band
    /// A tall column: the announced reset, the last one and the three chances.
    case column
}

/// The reset tracker in brief, or the words saying it is off.
struct ResetsSummary: View {
    let document: GlanceDocument
    let now: Date
    let style: ResetsSummaryStyle

    private var units: GlanceUnits { document.labels.units }

    var body: some View {
        if let resets = document.resets {
            summary(resets)
        } else {
            VStack(alignment: .leading, spacing: 5) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Image(systemName: "arrow.counterclockwise.circle")
                        .font(.system(size: WidgetScale.title, weight: .semibold))
                        .foregroundStyle(.secondary)
                    Text(WidgetText.resetsTitle(document))
                        .font(.system(size: WidgetScale.title, weight: .semibold))
                        .lineLimit(1)
                }
                Text(WidgetText.resetsOff(document))
                    .font(.system(size: WidgetScale.caption))
                    .foregroundStyle(.secondary)
                    .lineLimit(4)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    @ViewBuilder
    private func summary(_ resets: GlanceResets) -> some View {
        let upcoming = resets.upcoming(at: now)
        switch style {
        case .narrow:
            VStack(alignment: .leading, spacing: 6) {
                ResetsHeader(resets: resets)
                if let upcoming {
                    AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13)
                } else {
                    ChanceHero(resets: resets)
                }
                Spacer(minLength: 0)
                if let latest = resets.latest {
                    latest.since.live(now: now, units: units)
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        case .band:
            VStack(alignment: .leading, spacing: 7) {
                ResetsHeader(resets: resets, showsSource: true)
                HStack(alignment: .top, spacing: WidgetScale.columnSpacing + 4) {
                    VStack(alignment: .leading, spacing: 7) {
                        if let upcoming {
                            AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13.5)
                        }
                        if let latest = resets.latest {
                            LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now, valueSize: upcoming == nil ? 13.5 : WidgetScale.caption, showsWhen: upcoming == nil)
                                .foregroundStyle(upcoming == nil ? Color.primary : Color.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    ChanceBars(resets: resets, spacing: 5)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        case .column:
            VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
                ResetsHeader(resets: resets)
                if let upcoming {
                    AnnouncedResetBlock(upcoming: upcoming, units: units, now: now, valueSize: 13.5)
                }
                if let latest = resets.latest {
                    LatestResetBlock(latest: latest, tint: resets.tint, units: units, now: now)
                }
                ChanceBars(resets: resets, spacing: 5)
                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }
}

/// The parts Settings chose for the Overview (the accounts' limits in the compact form, the summary
/// of the reset tracker the widget chose, the limits coming back next), laid out for the widget's
/// size: side by side on the wide sizes, stacked on the large one. A part left out gives its room to
/// the others.
struct OverviewLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    private var parts: [GlanceView] { document.widget.tabs }

    var body: some View {
        if parts.contains(.resets), let resets = document.resets {
            ResetWidgetPager(
                document: document, resets: resets, family: family, now: now, size: size,
                namespace: "overview", prefixPages: parts.filter { $0 != .resets }.map { part in
                    AnyView(pane(part, width: size.width, style: .band, showsAccounts: true)
                        .frame(width: size.width, height: max(40, size.height - 46), alignment: .topLeading))
                }
            )
        } else {
            VStack(alignment: .leading, spacing: 0) {
                switch family {
                case .systemMedium, .systemSmall: medium
                case .systemLarge: large
                default: extraLarge
                }
                UpdatedFooter(document: document, now: now)
            }
        }
    }

    @ViewBuilder
    private func quota(width: CGFloat, height: CGFloat, showsAccounts: Bool = false) -> some View {
        if providers.isEmpty {
            Text(document.widget.empty)
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(.secondary)
                .lineLimit(4)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            CompactQuota(document: document, providers: providers, now: now, width: width, height: height, columns: 1, showsAccounts: showsAccounts)
        }
    }

    private func upcoming(width: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            UpcomingHeading(document: document)
            UpcomingList(document: document, now: now, width: width)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    @ViewBuilder
    private func pane(_ part: GlanceView, width: CGFloat, style: ResetsSummaryStyle, showsAccounts: Bool = false) -> some View {
        switch part {
        case .quota:
            quota(width: width, height: size.height, showsAccounts: showsAccounts)
        case .resets:
            ResetsSummary(document: document, now: now, style: style)
        case .upcoming:
            upcoming(width: width)
        }
    }

    @ViewBuilder
    private var medium: some View {
        if parts.count > 2 {
            let width = max(1, (size.width - WidgetScale.columnSpacing * 2 - 1) / 2)
            HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
                pane(parts[0], width: width, style: .narrow)
                    .frame(width: width)
                Divider()
                VStack(alignment: .leading, spacing: 7) {
                    ForEach(Array(parts.dropFirst().enumerated()), id: \.element) { index, part in
                        if index > 0 { Divider() }
                        brief(part)
                            .frame(maxHeight: .infinity, alignment: .topLeading)
                    }
                }
                .frame(width: width)
            }
            .frame(maxHeight: .infinity, alignment: .topLeading)
        } else {
            columns(parts, style: .narrow)
        }
    }

    private var large: some View {
        GeometryReader { proxy in
            let gap = WidgetScale.blockSpacing
            let height = max(1, (proxy.size.height - CGFloat(max(parts.count - 1, 0)) * (gap * 2 + 1)) / CGFloat(max(parts.count, 1)))
            VStack(alignment: .leading, spacing: gap) {
                ForEach(Array(parts.enumerated()), id: \.element) { index, part in
                    if index > 0 { Divider() }
                    Group {
                        if part == .resets && parts.count > 1 {
                            brief(part)
                        } else {
                            pane(part, width: proxy.size.width, style: parts.count == 1 ? .column : .band, showsAccounts: parts.count == 1)
                        }
                    }
                    .frame(height: height, alignment: .topLeading)
                }
            }
        }
    }

    @ViewBuilder
    private func brief(_ part: GlanceView) -> some View {
        switch part {
        case .quota:
            quota(width: size.width / 2, height: max(1, (size.height - WidgetScale.footerHeight) / 2))
        case .resets:
            if let resets = document.resets {
                VStack(alignment: .leading, spacing: 3) {
                    ResetsHeader(resets: resets)
                    if let upcoming = resets.upcoming(at: now) {
                        upcoming.liveValue(now: now, units: document.labels.units)
                            .font(.system(size: WidgetScale.value, weight: .semibold))
                            .monospacedDigit()
                            .lineLimit(2)
                    } else if let chance = resets.chance(days: 1) ?? resets.forecast.first {
                        Text("\(chance.label) · \(chance.percent)%")
                            .font(.system(size: WidgetScale.value, weight: .semibold))
                            .lineLimit(2)
                    } else if let latest = resets.latest {
                        latest.since.live(now: now, units: document.labels.units)
                            .font(.system(size: WidgetScale.caption))
                            .lineLimit(2)
                    }
                }
            } else {
                Text(WidgetText.resetsOff(document))
                    .font(.system(size: WidgetScale.caption))
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
            }
        case .upcoming:
            let all = GlanceUpcomingLimit.list(document.widget.providers, now: now)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 3) {
                    UpcomingHeading(document: document)
                    Spacer(minLength: 0)
                    HiddenCount(count: all.count - 1)
                }
                if let next = all.first {
                    Text("\(accountName(next.provider, in: document.widget.providers)) · \(next.metric.label) · \(Text(next.at, style: .relative))")
                        .font(.system(size: WidgetScale.caption, weight: .medium))
                        .lineLimit(2)
                        .truncationMode(.middle)
                } else {
                    Text(UpcomingText.empty(document))
                        .font(.system(size: WidgetScale.caption))
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
            }
        }
    }

    private var extraLarge: some View {
        columns(parts, style: .column, showsAccounts: true)
    }

    /// Parts side by side: the reset summary keeps a fixed column, the limits and the limits
    /// coming back share the rest (the limits a little more).
    private func columns(_ shown: [GlanceView], style: ResetsSummaryStyle, showsAccounts: Bool = false) -> some View {
        let dividers = CGFloat(max(shown.count - 1, 0))
        let available = size.width - dividers * (WidgetScale.columnSpacing * 2 + 1)
        let resetWidth: CGFloat = shown.count == 1 ? available : (style == .narrow ? min(150, available * 0.5) : min(220, available / CGFloat(shown.count)))
        let rest = shown.contains(.resets) ? available - resetWidth : available
        let flexible = shown.filter { $0 != .resets }
        let width: (GlanceView) -> CGFloat = { part in
            if part == .resets { return resetWidth }
            if flexible.count < 2 { return rest }
            return part == .quota ? (rest * 0.55).rounded(.down) : rest - (rest * 0.55).rounded(.down)
        }
        return HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
            ForEach(Array(shown.enumerated()), id: \.element) { index, part in
                if index > 0 {
                    Divider()
                }
                pane(part, width: width(part), style: shown.count == 1 && part == .resets ? .column : style, showsAccounts: showsAccounts)
                    .frame(width: width(part))
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }
}
