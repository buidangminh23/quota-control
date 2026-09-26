import SwiftUI
import WidgetKit

/// The desktop and Notification Center widgets, in three styles and four sizes each. They read
/// `glance.json`, which Quota Control writes next to its settings whenever a reading changes and then
/// asks WidgetKit to reload, so they list the accounts and metrics Settings → Widget chooses, read
/// exactly like the popup. Reset countdowns tick on their own; when the app is closed the widgets keep
/// their last readings and say how old they are.
@main
struct QuotaControlWidgets: WidgetBundle {
    var body: some Widget {
        QuotaDetailsWidget()
        QuotaRingsWidget()
        QuotaCompactWidget()
    }
}

enum QuotaWidgetStyle {
    /// Each account with its meters, headlines and reset countdowns.
    case details
    /// One round gauge per metric.
    case rings
    /// One line per metric, the most accounts at once.
    case compact
}

private let widgetFamilies: [WidgetFamily] = [.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge]

struct QuotaDetailsWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "QuotaControlUsage", provider: GlanceTimeline()) { entry in
            GlanceWidgetView(entry: entry, style: .details)
        }
        .configurationDisplayName(WidgetText.name(.details))
        .description(WidgetText.description(.details))
        .supportedFamilies(widgetFamilies)
    }
}

struct QuotaRingsWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "QuotaControlRings", provider: GlanceTimeline()) { entry in
            GlanceWidgetView(entry: entry, style: .rings)
        }
        .configurationDisplayName(WidgetText.name(.rings))
        .description(WidgetText.description(.rings))
        .supportedFamilies(widgetFamilies)
    }
}

struct QuotaCompactWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "QuotaControlCompact", provider: GlanceTimeline()) { entry in
            GlanceWidgetView(entry: entry, style: .compact)
        }
        .configurationDisplayName(WidgetText.name(.compact))
        .description(WidgetText.description(.compact))
        .supportedFamilies(widgetFamilies)
    }
}

struct GlanceEntry: TimelineEntry {
    let date: Date
    let document: GlanceDocument?
}

struct GlanceTimeline: TimelineProvider {
    /// How often the widget looks again on its own; the app also reloads it whenever readings change.
    private static let refresh: TimeInterval = 15 * 60
    private static let resetEntries = 6

    func placeholder(in context: Context) -> GlanceEntry {
        GlanceEntry(date: Date(), document: .sample)
    }

    func getSnapshot(in context: Context, completion: @escaping (GlanceEntry) -> Void) {
        let document = GlanceStore.load()
        completion(GlanceEntry(date: Date(), document: document ?? (context.isPreview ? .sample : nil)))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<GlanceEntry>) -> Void) {
        let now = Date()
        let document = GlanceStore.load()
        var entries = [GlanceEntry(date: now, document: document)]
        var moments = Set(document?.widget.providers.flatMap(\.metrics).compactMap(\.resetsAt).filter { $0 > now } ?? [])
        if let document {
            let stale = document.generatedAt.addingTimeInterval(GlanceStaleness.after)
            if stale > now {
                moments.insert(stale)
            }
        }
        for moment in moments.sorted().prefix(Self.resetEntries) {
            entries.append(GlanceEntry(date: moment, document: document))
        }
        completion(Timeline(entries: entries, policy: .after(now.addingTimeInterval(Self.refresh))))
    }
}

enum GlanceStore {
    /// `~/Library/Application Support/usage-control/widget/glance.json` in the real home folder:
    /// the widget is sandboxed, and its entitlement grants read access to exactly that folder, not
    /// to the accounts beside it.
    static var fileURL: URL {
        realHome
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("Application Support", isDirectory: true)
            .appendingPathComponent("usage-control", isDirectory: true)
            .appendingPathComponent("widget", isDirectory: true)
            .appendingPathComponent("glance.json", isDirectory: false)
    }

    private static var realHome: URL {
        if let entry = getpwuid(getuid()), let directory = entry.pointee.pw_dir {
            return URL(fileURLWithPath: String(cString: directory), isDirectory: true)
        }
        return URL(fileURLWithPath: NSHomeDirectory(), isDirectory: true)
    }

    static func load() -> GlanceDocument? {
        guard let data = try? Data(contentsOf: fileURL, options: .mappedIfSafe) else { return nil }
        return GlanceDocument.decode(data)
    }
}

enum WidgetText {
    static var vietnamese: Bool {
        Locale.preferredLanguages.first?.hasPrefix("vi") ?? false
    }

    static func name(_ style: QuotaWidgetStyle) -> String {
        switch style {
        case .details: return vietnamese ? "Chi tiết" : "Details"
        case .rings: return vietnamese ? "Vòng tròn" : "Rings"
        case .compact: return vietnamese ? "Gọn" : "Compact"
        }
    }

    static func description(_ style: QuotaWidgetStyle) -> String {
        switch style {
        case .details:
            return vietnamese
                ? "Hạn mức từng tài khoản Claude, Codex… với thanh mức dùng và giờ đặt lại."
                : "Each Claude, Codex… account's limits with usage bars and reset times."
        case .rings:
            return vietnamese ? "Mỗi chỉ số một vòng tròn phần trăm." : "A percentage ring for every metric."
        case .compact:
            return vietnamese ? "Mỗi chỉ số một dòng, xem được nhiều tài khoản nhất." : "One line per metric, the most accounts at once."
        }
    }

    static var notRunning: String {
        vietnamese ? "Mở Quota Control để hiện hạn mức ở đây." : "Open Quota Control to show your limits here."
    }
}

struct GlanceWidgetView: View {
    let entry: GlanceEntry
    let style: QuotaWidgetStyle
    @Environment(\.widgetFamily) private var family

    var body: some View {
        Group {
            if let document = entry.document {
                let providers = document.widget.visibleProviders
                if providers.isEmpty {
                    WidgetMessage(text: document.widget.empty)
                } else {
                    content(document, providers: providers)
                        .environment(\.locale, document.resolvedLocale)
                }
            } else {
                WidgetMessage(text: WidgetText.notRunning)
            }
        }
        .containerBackground(.background, for: .widget)
    }

    @ViewBuilder
    private func content(_ document: GlanceDocument, providers: [GlanceProvider]) -> some View {
        switch style {
        case .details:
            DetailsLayout(document: document, providers: providers, family: family, now: entry.date)
        case .rings:
            RingsLayout(document: document, providers: providers, family: family, now: entry.date)
        case .compact:
            CompactLayout(document: document, providers: providers, family: family, now: entry.date)
        }
    }
}

/// How many columns of accounts a size holds.
private func accountColumns(_ family: WidgetFamily, style: QuotaWidgetStyle) -> Int {
    switch family {
    case .systemMedium: return 2
    case .systemExtraLarge: return style == .compact ? 3 : 2
    default: return 1
    }
}

/// One way to fit the accounts: how many of them, how many metrics each, and whether each account
/// shrinks to a single line.
private struct FitPlan: Hashable {
    let accounts: Int
    let metrics: Int
    let condensed: Bool

    /// Densest last: every account with up to `most` metrics, fewer metrics each, one line per
    /// account, then fewer accounts.
    static func candidates(accounts: Int, most: Int) -> [FitPlan] {
        let full = (1...max(1, most)).reversed().map { FitPlan(accounts: accounts, metrics: $0, condensed: false) }
        let lines = stride(from: accounts, through: 1, by: -1).map { FitPlan(accounts: $0, metrics: 1, condensed: true) }
        return full + lines
    }
}

/// `providers` dealt into `count` columns left to right, so the first accounts lead every column.
private func columns(_ providers: [GlanceProvider], count: Int) -> [[GlanceProvider]] {
    guard count > 1 else { return [providers] }
    return (0..<count).map { column in
        stride(from: column, to: providers.count, by: count).map { providers[$0] }
    }
}

// MARK: Details

private struct DetailsLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ViewThatFits(in: .vertical) {
                ForEach(FitPlan.candidates(accounts: providers.count, most: 4), id: \.self) { plan in
                    planned(plan)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            UpdatedFooter(document: document, now: now)
                .padding(.top, 4)
        }
    }

    private func planned(_ plan: FitPlan) -> some View {
        let shown = Array(providers.prefix(plan.accounts))
        let count = accountColumns(family, style: .details)
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 14) {
                ForEach(Array(columns(shown, count: count).enumerated()), id: \.offset) { _, column in
                    VStack(alignment: .leading, spacing: plan.condensed ? 7 : 10) {
                        ForEach(column) { provider in
                            if plan.condensed {
                                CondensedAccount(provider: provider, document: document)
                            } else {
                                DetailedAccount(provider: provider, document: document, now: now, metrics: plan.metrics)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            MoreAccounts(hidden: providers.count - shown.count, labels: document.labels)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// An account's header with its first `metrics` meters, or the notice saying why it has none.
private struct DetailedAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument
    let now: Date
    let metrics: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            GlanceProviderHeader(provider: provider, shows: document.widget.shows, size: 12)
            if provider.metrics.isEmpty {
                GlanceNoticeRow(text: provider.notice ?? document.labels.noData, size: 10)
            }
            ForEach(provider.metrics.prefix(metrics)) { metric in
                WidgetMetricRow(metric: metric, labels: document.labels, now: now, showsReset: document.widget.shows.resets)
            }
        }
    }
}

/// An account in one line: the mark and name, the first reading and its meter.
private struct CondensedAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 4) {
                GlanceProviderHeader(provider: provider, shows: GlanceShows(account: false, plan: false, resets: false), size: 11)
                Spacer(minLength: 2)
                if let metric = provider.metrics.first {
                    Text(metric.value)
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(GlancePalette.text(metric.severity, onDark: false))
                } else {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.system(size: 10))
                        .foregroundStyle(Color.orange)
                }
            }
            if let fraction = provider.metrics.first?.fraction, let metric = provider.metrics.first {
                GlanceMeter(fraction: fraction, severity: metric.severity, height: 4)
            }
        }
    }
}

/// A meter row whose countdown WidgetKit keeps current between reloads.
private struct WidgetMetricRow: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date
    var showsReset = true

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(metric.label)
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer(minLength: 2)
                Text(metric.headline)
                    .font(.system(size: 11.5, weight: .semibold))
                    .monospacedDigit()
                    .foregroundStyle(GlancePalette.text(metric.severity, onDark: false))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            if let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, height: 4)
            }
            if showsReset {
                ResetLine(metric: metric, labels: labels, now: now)
                    .font(.system(size: 9.5))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// When a metric comes back, as text WidgetKit counts down by itself.
private struct ResetLine: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date

    var body: some View {
        Group {
            if let resetsAt = metric.resetsAt {
                if resetsAt > now {
                    Text("\(labels.resetsIn) \(Text(resetsAt, style: .relative))")
                        .monospacedDigit()
                } else {
                    Text(labels.resetting)
                }
            } else if let detail = metric.detail {
                Text(detail)
            }
        }
        .lineLimit(1)
    }
}

// MARK: Rings

/// One ring: a metric of an account, or an account that has none and says why.
private struct RingTile: Identifiable {
    let id: String
    let provider: GlanceProvider
    let metric: GlanceMetric?
}

private struct RingGrid: Hashable {
    let columns: Int
    let rows: Int
    let ring: CGFloat
}

private struct RingsLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date

    private var tiles: [RingTile] {
        providers.flatMap { provider -> [RingTile] in
            if provider.metrics.isEmpty {
                return [RingTile(id: provider.id, provider: provider, metric: nil)]
            }
            return provider.metrics.map { RingTile(id: "\(provider.id)|\($0.id)", provider: provider, metric: $0) }
        }
    }

    /// Bigger rings while everything fits, smaller ones as the tiles grow.
    private var grids: [RingGrid] {
        switch family {
        case .systemSmall:
            return [RingGrid(columns: 1, rows: 1, ring: 92), RingGrid(columns: 2, rows: 1, ring: 60), RingGrid(columns: 2, rows: 2, ring: 50)]
        case .systemMedium:
            return [RingGrid(columns: 3, rows: 1, ring: 80), RingGrid(columns: 4, rows: 1, ring: 66), RingGrid(columns: 5, rows: 2, ring: 42)]
        case .systemExtraLarge:
            return [RingGrid(columns: 5, rows: 2, ring: 92), RingGrid(columns: 6, rows: 3, ring: 70), RingGrid(columns: 8, rows: 4, ring: 52)]
        default:
            return [RingGrid(columns: 2, rows: 2, ring: 100), RingGrid(columns: 3, rows: 3, ring: 70), RingGrid(columns: 4, rows: 4, ring: 52)]
        }
    }

    var body: some View {
        let tiles = self.tiles
        let grid = grids.first { $0.columns * $0.rows >= tiles.count } ?? grids[grids.count - 1]
        let shown = Array(tiles.prefix(grid.columns * grid.rows))
        let multiple = Set(providers.map(\.id)).count > 1
        VStack(spacing: 0) {
            Grid(horizontalSpacing: 8, verticalSpacing: 8) {
                ForEach(0..<rows(shown.count, columns: grid.columns), id: \.self) { row in
                    GridRow {
                        ForEach(0..<grid.columns, id: \.self) { column in
                            let index = row * grid.columns + column
                            if index < shown.count {
                                RingTileView(tile: shown[index], document: document, now: now, ring: grid.ring, namesAccount: multiple)
                            } else {
                                Color.clear.frame(width: grid.ring, height: grid.ring)
                            }
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            if tiles.count > shown.count {
                MoreAccounts(hidden: Set(tiles.dropFirst(shown.count).map(\.provider.id)).count, labels: document.labels, metricsOnly: true)
            }
            if family != .systemSmall {
                UpdatedFooter(document: document, now: now)
                    .padding(.top, 4)
            }
        }
    }

    private func rows(_ count: Int, columns: Int) -> Int {
        max(1, (count + columns - 1) / columns)
    }
}

private struct RingTileView: View {
    let tile: RingTile
    let document: GlanceDocument
    let now: Date
    let ring: CGFloat
    let namesAccount: Bool

    var body: some View {
        VStack(spacing: 3) {
            GlanceRing(fraction: tile.metric?.fraction, severity: tile.metric?.severity ?? .none, lineWidth: max(3, ring / 14)) {
                VStack(spacing: ring > 60 ? 2 : 0) {
                    ProviderMark(mark: tile.provider.mark)
                        .foregroundStyle(markColor)
                        .frame(width: ring * 0.22, height: ring * 0.22)
                    if let metric = tile.metric {
                        Text(metric.value)
                            .font(.system(size: max(9, ring * 0.2), weight: .bold))
                            .monospacedDigit()
                            .foregroundStyle(GlancePalette.text(metric.severity, onDark: false))
                            .lineLimit(1)
                            .minimumScaleFactor(0.5)
                    } else {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: max(8, ring * 0.16)))
                            .foregroundStyle(Color.orange)
                    }
                }
                .padding(ring * 0.14)
            }
            .frame(width: ring, height: ring)
            Text(caption)
                .font(.system(size: ring > 60 ? 10 : 9, weight: .medium))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .frame(maxWidth: ring + 16)
            if ring > 60, document.widget.shows.resets, let metric = tile.metric {
                ResetLine(metric: metric, labels: document.labels, now: now)
                    .font(.system(size: 8.5))
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: ring + 24)
            }
        }
    }

    private var caption: String {
        guard let metric = tile.metric else { return tile.provider.notice ?? document.labels.noData }
        return namesAccount ? "\(tile.provider.name) · \(metric.label)" : metric.label
    }

    private var markColor: Color {
        tile.provider.color.uppercased() == "#FFFFFF" ? .primary : tile.provider.tint
    }
}

// MARK: Compact

private struct CompactLayout: View {
    let document: GlanceDocument
    let providers: [GlanceProvider]
    let family: WidgetFamily
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ViewThatFits(in: .vertical) {
                ForEach(FitPlan.candidates(accounts: providers.count, most: 6), id: \.self) { plan in
                    planned(plan)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            if family != .systemSmall {
                UpdatedFooter(document: document, now: now)
                    .padding(.top, 4)
            }
        }
    }

    private func planned(_ plan: FitPlan) -> some View {
        let shown = Array(providers.prefix(plan.accounts))
        let count = accountColumns(family, style: .compact)
        return VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .top, spacing: 14) {
                ForEach(Array(columns(shown, count: count).enumerated()), id: \.offset) { _, column in
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(column) { provider in
                            CompactAccount(provider: provider, document: document, now: now, metrics: plan.condensed ? 1 : plan.metrics, headed: !plan.condensed)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .topLeading)
                }
            }
            MoreAccounts(hidden: providers.count - shown.count, labels: document.labels)
        }
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// An account as short lines: an optional header, then one line per metric with a thin meter.
private struct CompactAccount: View {
    let provider: GlanceProvider
    let document: GlanceDocument
    let now: Date
    let metrics: Int
    let headed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if headed {
                GlanceProviderHeader(provider: provider, shows: document.widget.shows, size: 11)
            }
            if provider.metrics.isEmpty {
                HStack(spacing: 4) {
                    if !headed { mark }
                    GlanceNoticeRow(text: headed ? (provider.notice ?? document.labels.noData) : provider.name, size: 9.5)
                }
            }
            ForEach(provider.metrics.prefix(metrics)) { metric in
                HStack(spacing: 5) {
                    if !headed { mark }
                    Text(headed ? metric.label : "\(provider.name) · \(metric.label)")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .layoutPriority(1)
                    if let fraction = metric.fraction {
                        GlanceMeter(fraction: fraction, severity: metric.severity, height: 3)
                            .frame(minWidth: 18, maxWidth: 60)
                    } else {
                        Spacer(minLength: 2)
                    }
                    Text(metric.value)
                        .font(.system(size: 10.5, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(GlancePalette.text(metric.severity, onDark: false))
                        .lineLimit(1)
                        .fixedSize()
                    if headed, document.widget.shows.resets, metric.resetsAt != nil {
                        ResetLine(metric: metric, labels: document.labels, now: now)
                            .font(.system(size: 8.5))
                            .foregroundStyle(.tertiary)
                            .fixedSize()
                    }
                }
            }
        }
    }

    private var mark: some View {
        ProviderMark(mark: provider.mark)
            .foregroundStyle(provider.color.uppercased() == "#FFFFFF" ? .primary : provider.tint)
            .frame(width: 10, height: 10)
    }
}

// MARK: Shared parts

/// `+2 tài khoản khác`: the accounts a layout had to leave out.
private struct MoreAccounts: View {
    let hidden: Int
    let labels: GlanceLabels
    var metricsOnly = false

    var body: some View {
        if hidden > 0 {
            Text(metricsOnly ? "+\(hidden)" : "+\(hidden) \(labels.more)")
                .font(.system(size: 9.5, weight: .medium))
                .foregroundStyle(.secondary)
        }
    }
}

private struct UpdatedFooter: View {
    let document: GlanceDocument
    let now: Date

    var body: some View {
        let stale = now.timeIntervalSince(document.generatedAt) > GlanceStaleness.after
        HStack(spacing: 3) {
            if stale {
                Image(systemName: "exclamationmark.circle")
            }
            Text("\(document.labels.updated) \(GlanceFormat.time(document.generatedAt, locale: document.resolvedLocale, hour12: document.hour12))")
        }
        .font(.system(size: 9))
        .foregroundStyle(stale ? Color.orange : Color.secondary)
        .lineLimit(1)
    }
}

/// Readings older than this come from an app that stopped refreshing. The timeline adds an entry at
/// that moment, so the marker appears on time even when nothing reloads the widget.
enum GlanceStaleness {
    static let after: TimeInterval = 20 * 60
}

private struct WidgetMessage: View {
    let text: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "gauge.with.dots.needle.33percent")
                .font(.system(size: 22, weight: .medium))
                .foregroundStyle(.secondary)
            Text(text)
                .font(.system(size: 11))
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

extension GlanceDocument {
    /// What the widget gallery shows before the app has written any readings.
    static var sample: GlanceDocument {
        let vietnamese = WidgetText.vietnamese
        let now = Date()
        let labels = GlanceLabels(
            title: "Quota Control",
            empty: vietnamese ? "Gắn sao chỉ số trong Quota Control để hiện ở đây." : "Star metrics in Quota Control to show them here.",
            updated: vietnamese ? "Cập nhật" : "Updated",
            resetsIn: vietnamese ? "Đặt lại sau" : "Resets in",
            resetting: vietnamese ? "Đang đặt lại…" : "Resetting…",
            open: vietnamese ? "Mở Quota Control" : "Open Quota Control",
            notRunning: WidgetText.notRunning,
            noData: vietnamese ? "Chưa có số liệu" : "No data yet",
            more: vietnamese ? "tài khoản khác" : "more",
            units: vietnamese
                ? GlanceUnits(day: " ngày", hour: " giờ", minute: " phút")
                : GlanceUnits(day: "d", hour: "h", minute: "m")
        )
        func metric(_ id: String, _ label: String, used: Double, hours: Double) -> GlanceMetric {
            let left = Int(((1 - used) * 100).rounded())
            return GlanceMetric(
                id: id,
                label: label,
                value: "\(left)%",
                headline: vietnamese ? "Còn \(left)%" : "\(left)% left",
                fraction: 1 - used,
                severity: used >= 0.8 ? .warning : .normal,
                resetsAt: now.addingTimeInterval(hours * 3600),
                detail: nil
            )
        }
        let session = vietnamese ? "Phiên" : "Session"
        let weekly = vietnamese ? "Tuần" : "Weekly"
        let providers = [
            GlanceProvider(id: "claude", name: "Claude", account: nil, plan: "Max", notice: nil, brand: "claude", color: "#DE7356", mark: nil, metrics: [
                metric("claude.session", session, used: 0.42, hours: 2.2),
                metric("claude.weekly", weekly, used: 0.18, hours: 70),
            ]),
            GlanceProvider(id: "codex", name: "Codex", account: nil, plan: "Pro", notice: nil, brand: "codex", color: "#10A37F", mark: nil, metrics: [
                metric("codex.session", session, used: 0.83, hours: 1.1),
                metric("codex.weekly", weekly, used: 0.35, hours: 96),
            ]),
        ]
        return GlanceDocument(
            version: supportedVersion,
            generatedAt: now,
            locale: vietnamese ? "vi_VN" : "en_US",
            hour12: nil,
            labels: labels,
            providers: providers,
            island: GlanceIsland(enabled: true),
            widget: GlanceWidgetContent(providers: providers, shows: .all, empty: labels.empty),
            alert: nil
        )
    }
}
