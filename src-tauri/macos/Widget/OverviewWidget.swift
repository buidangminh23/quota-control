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

/// The accounts' limits in the compact form beside the Codex reset summary, and on the widest
/// widget the limits coming back next.
struct OverviewLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date
    let size: CGSize

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            switch family {
            case .systemMedium, .systemSmall: medium
            case .systemLarge: large
            default: extraLarge
            }
            UpdatedFooter(document: document, now: now)
        }
    }

    private var quotaIsEmpty: Bool { providers.isEmpty }

    @ViewBuilder
    private func quota(width: CGFloat, height: CGFloat, columns: Int, showsAccounts: Bool = false) -> some View {
        if quotaIsEmpty {
            Text(document.widget.empty)
                .font(.system(size: WidgetScale.caption))
                .foregroundStyle(.secondary)
                .lineLimit(4)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            CompactQuota(document: document, providers: providers, now: now, width: width, height: height, columns: columns, showsAccounts: showsAccounts)
        }
    }

    private var medium: some View {
        let reset = min(150, size.width * 0.42)
        let left = size.width - reset - WidgetScale.columnSpacing * 2 - 1
        return HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
            quota(width: left, height: size.height, columns: 1)
                .frame(width: left)
            Divider()
            ResetsSummary(document: document, now: now, style: .narrow)
                .frame(width: reset)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var large: some View {
        VStack(alignment: .leading, spacing: WidgetScale.blockSpacing) {
            quota(width: size.width, height: size.height, columns: 1)
                .frame(maxHeight: .infinity, alignment: .top)
            Divider()
            ResetsSummary(document: document, now: now, style: .band)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var extraLarge: some View {
        let reset: CGFloat = 200
        let rest = size.width - reset - WidgetScale.columnSpacing * 4 - 2
        let quotaWidth = (rest * 0.55).rounded(.down)
        let upcomingWidth = rest - quotaWidth
        return HStack(alignment: .top, spacing: WidgetScale.columnSpacing) {
            quota(width: quotaWidth, height: size.height, columns: 1, showsAccounts: true)
                .frame(width: quotaWidth)
            Divider()
            ResetsSummary(document: document, now: now, style: .column)
                .frame(width: reset)
            Divider()
            VStack(alignment: .leading, spacing: 7) {
                UpcomingHeading(document: document)
                UpcomingList(document: document, now: now, width: upcomingWidth)
            }
            .frame(width: upcomingWidth)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }
}
