import SwiftUI
import WidgetKit

/// The desktop and Notification Center widgets. They read `glance.json`, which Quota Control writes
/// next to its settings whenever a reading changes and then asks WidgetKit to reload, so they list
/// the accounts and metrics Settings → Widget chooses, worded exactly like the popup. Three styles
/// show the limits, one the limits coming back next, two the reset tracker Settings chose (Codex's
/// or Claude's) and one an overview of both. Countdowns tick on their own; when the app is closed
/// the widgets keep their last readings and say how old they are.
@main
struct QuotaControlWidgets: WidgetBundle {
    var body: some Widget {
        QuotaDetailsWidget()
        QuotaRingsWidget()
        QuotaCompactWidget()
        QuotaOverviewWidget()
        QuotaUpcomingWidget()
        CodexResetsWidget()
        ResetCalendarWidget()
    }
}

enum QuotaWidgetStyle {
    /// Each account with its meters, headlines and reset countdowns.
    case details
    /// One round gauge per metric.
    case rings
    /// One line per metric, the most accounts at once.
    case compact
    /// The limits in brief beside the reset summary.
    case overview
    /// The limits coming back next, soonest first.
    case upcoming
    /// The reset tracker Settings chose: Codex's free-reset tracker, or Claude's.
    case codexResets
    /// That tracker's reset calendar and rhythm.
    case resetCalendar

    /// The sizes the style offers in the widget gallery.
    var families: [WidgetFamily] {
        switch self {
        case .overview, .resetCalendar: return [.systemMedium, .systemLarge, .systemExtraLarge]
        default: return [.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge]
        }
    }
}

/// One widget kind: its style, the kind string WidgetKit stores placed widgets under, and the
/// gallery wording. Kind strings never change, or placed widgets would turn blank.
private func glanceConfiguration(kind: String, style: QuotaWidgetStyle) -> some WidgetConfiguration {
    StaticConfiguration(kind: kind, provider: GlanceTimeline()) { entry in
        GlanceWidgetEntryView(entry: entry, style: style)
    }
    .configurationDisplayName(WidgetText.name(style))
    .description(WidgetText.description(style))
    .supportedFamilies(style.families)
}

struct QuotaDetailsWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlUsage", style: .details) }
}

struct QuotaRingsWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlRings", style: .rings) }
}

struct QuotaCompactWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlCompact", style: .compact) }
}

struct QuotaOverviewWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlOverview", style: .overview) }
}

struct QuotaUpcomingWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlUpcoming", style: .upcoming) }
}

struct CodexResetsWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlCodexResets", style: .codexResets) }
}

struct ResetCalendarWidget: Widget {
    var body: some WidgetConfiguration { glanceConfiguration(kind: "QuotaControlResetCalendar", style: .resetCalendar) }
}

struct GlanceEntry: TimelineEntry {
    let date: Date
    let document: GlanceDocument?
}

struct GlanceTimeline: TimelineProvider {
    /// How often the widget looks again on its own; the app also reloads it whenever readings change.
    private static let refresh: TimeInterval = 15 * 60
    private static let momentEntries = 8

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
        for moment in Self.moments(document, after: now).prefix(Self.momentEntries) {
            entries.append(GlanceEntry(date: moment, document: document))
        }
        completion(Timeline(entries: entries, policy: .after(now.addingTimeInterval(Self.refresh))))
    }

    /// The moments after `now` when something drawn changes on its own, soonest first: a limit
    /// comes back, a countdown of the reset tracker the widget chose ends or its row goes away, or
    /// the readings turn stale.
    static func moments(_ document: GlanceDocument?, after now: Date) -> [Date] {
        guard let document else { return [] }
        var moments = Set(GlanceUpcomingLimit.list(document.widget.providers, now: now).map(\.at))
        moments.formUnion(document.forWidget.resetMoments(after: now))
        let stale = document.generatedAt.addingTimeInterval(GlanceStaleness.after)
        if stale > now {
            moments.insert(stale)
        }
        return moments.sorted()
    }
}

enum GlanceStore {
    /// `~/Library/Application Support/usage-control/widget` in the real home folder: the widget is
    /// sandboxed, and its entitlement grants read access to exactly that folder, not to the accounts
    /// beside it, and write access only to its `requests` folder (`WidgetRequests`).
    static var folderURL: URL {
        realHome
            .appendingPathComponent("Library", isDirectory: true)
            .appendingPathComponent("Application Support", isDirectory: true)
            .appendingPathComponent("usage-control", isDirectory: true)
            .appendingPathComponent("widget", isDirectory: true)
    }

    static var fileURL: URL {
        folderURL.appendingPathComponent("glance.json", isDirectory: false)
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
        case .overview: return vietnamese ? "Tổng quan" : "Overview"
        case .upcoming: return vietnamese ? "Sắp đặt lại" : "Coming Back"
        case .codexResets: return vietnamese ? "Reset Codex" : "Codex Resets"
        case .resetCalendar: return vietnamese ? "Lịch reset Codex" : "Codex Reset Calendar"
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
        case .overview:
            return vietnamese
                ? "Hạn mức các tài khoản cùng dự báo reset Codex và các hạn mức sắp đặt lại."
                : "Your limits beside the Codex reset forecast and the limits coming back next."
        case .upcoming:
            return vietnamese
                ? "Các hạn mức sắp được đặt lại, sớm nhất lên trước, kèm giờ đặt lại."
                : "The limits coming back next, soonest first, with their reset times."
        case .codexResets:
            return vietnamese
                ? "Theo dõi reset miễn phí của Codex: giờ reset đã báo, khả năng có reset và lần reset gần nhất."
                : "The Codex free-reset tracker: announced resets, the chance of one and the last one."
        case .resetCalendar:
            return vietnamese
                ? "Lịch 20 tuần reset Codex và nhịp reset theo thứ, theo giờ."
                : "Twenty weeks of Codex resets and their rhythm by weekday and hour."
        }
    }

    static var notRunning: String {
        vietnamese ? "Mở Quota Control để hiện hạn mức ở đây." : "Open Quota Control to show your limits here."
    }

    /// What a reset widget says in place of the tracker the widget chose: the Reset tab's own line
    /// while the tracker is on but has nothing yet (`failed` once it could not load), else what
    /// turns the tracker on.
    static func resetsMessage(_ document: GlanceDocument) -> (text: String, failed: Bool) {
        if let pending = document.resetsPending { return (pending.text, pending.failed == true) }
        return (resetsOff(document), false)
    }

    /// What a reset widget says while the tracker the widget chose is off.
    static func resetsOff(_ document: GlanceDocument) -> String {
        if !document.labels.resetsOff.isEmpty { return document.labels.resetsOff }
        if document.widget.resetsProvider == .claude {
            return document.isVietnamese
                ? "Bật tab Reset trong Quota Control để xem dự báo reset Claude."
                : "Turn on the Reset tab in Quota Control to see the Claude reset forecast."
        }
        return document.isVietnamese
            ? "Bật tab Reset trong Quota Control để xem dự báo reset Codex."
            : "Turn on the Reset tab in Quota Control to see the Codex reset forecast."
    }

    static func resetsTitle(_ document: GlanceDocument) -> String {
        if document.widget.resetsProvider == .claude {
            return document.resets?.title ?? document.claudeResetsTitle
        }
        return document.resets?.title ?? (document.isVietnamese ? "Reset Codex" : "Codex Resets")
    }
}

/// Reads the size WidgetKit placed the widget at and draws `GlanceWidgetView` for it.
struct GlanceWidgetEntryView: View {
    let entry: GlanceEntry
    let style: QuotaWidgetStyle
    @Environment(\.widgetFamily) private var family

    var body: some View {
        GlanceWidgetView(entry: entry, style: style, family: family)
    }
}

/// A widget of any style at any size, drawn from the entry's document; the size is passed in so
/// every family can be rendered outside WidgetKit.
struct GlanceWidgetView: View {
    let entry: GlanceEntry
    let style: QuotaWidgetStyle
    let family: WidgetFamily
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        GeometryReader { proxy in
            content(size: proxy.size)
                .frame(width: proxy.size.width, height: proxy.size.height, alignment: .topLeading)
        }
        .containerBackground(containerFill, for: .widget)
        .environment(\.colorScheme, themedScheme ?? colorScheme)
    }

    /// The reset widgets draw in the app's theme, like the Reset tab and the island. Only in full
    /// color: dimmed on the desktop, the system draws every widget its own way, and dark ink forced
    /// by a light theme would fade out there.
    private var themedScheme: ColorScheme? {
        guard renderingMode == .fullColor else { return nil }
        switch style {
        case .overview, .codexResets, .resetCalendar:
            let theme = entry.document?.forWidget.resets?.theme
            return theme == "light" ? .light : theme == "dark" ? .dark : nil
        default:
            return nil
        }
    }

    /// A themed widget's background follows its theme, with the island's fill behind the tracker,
    /// so what is drawn straight on it (a section's name, the page buttons, the update time) stays
    /// readable when the app's theme and the Mac's appearance differ.
    private var containerFill: AnyShapeStyle {
        themedScheme.map { AnyShapeStyle(GlanceResetPalette(scheme: $0).background) } ?? AnyShapeStyle(.background)
    }

    @ViewBuilder
    private func content(size: CGSize) -> some View {
        if let document = entry.document {
            layout(document, size: size)
                .environment(\.locale, document.resolvedLocale)
        } else {
            WidgetMessage(text: WidgetText.notRunning)
        }
    }

    @ViewBuilder
    private func layout(_ full: GlanceDocument, size: CGSize) -> some View {
        let document = full.forWidget
        let providers = document.widget.visibleProviders
        let now = entry.date
        switch style {
        case .details, .rings, .compact:
            if providers.isEmpty {
                WidgetMessage(text: document.widget.empty)
            } else if style == .details {
                DetailsLayout(document: document, providers: providers, family: family, now: now, size: size)
            } else if style == .rings {
                RingsLayout(document: document, providers: providers, family: family, now: now, size: size)
            } else {
                CompactLayout(document: document, providers: providers, family: family, now: now, size: size)
            }
        case .overview:
            OverviewLayout(document: document, providers: providers, family: family, now: now, size: size)
        case .upcoming:
            UpcomingLayout(document: document, family: family, now: now, size: size)
        case .codexResets:
            if let resets = document.resets {
                CodexResetsLayout(document: document, resets: resets, family: family, now: now, size: size)
            } else {
                let message = WidgetText.resetsMessage(document)
                WidgetMessage(text: message.text, symbol: "arrow.counterclockwise.circle", failed: message.failed)
            }
        case .resetCalendar:
            if let resets = document.resets, let calendar = resets.calendar {
                ResetCalendarLayout(document: document, resets: resets, calendar: calendar, family: family, now: now, size: size)
            } else if let resets = document.resets {
                CodexResetsLayout(document: document, resets: resets, family: family, now: now, size: size)
            } else {
                let message = WidgetText.resetsMessage(document)
                WidgetMessage(text: message.text, symbol: "calendar", failed: message.failed)
            }
        }
    }
}

extension GlanceDocument {
    /// What the widget gallery shows before the app has written any readings: two accounts and a
    /// reset tracker with its chances, calendar and rhythm, worded in the Mac's language.
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
                : GlanceUnits(day: "d", hour: "h", minute: "m"),
            resetsOff: vietnamese
                ? "Bật tab Reset trong Quota Control để xem dự báo reset Codex."
                : "Turn on the Reset tab in Quota Control to see the Codex reset forecast.",
            upcoming: vietnamese ? "Sắp đặt lại" : "Coming back",
            upcomingEmpty: vietnamese ? "Chưa có hạn mức nào có giờ đặt lại." : "No limit has a reset time yet."
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
        let session = vietnamese ? "Phiên 5h" : "5h Session"
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
            resets: sampleResets(now: now, vietnamese: vietnamese),
            alert: nil
        )
    }

    /// A plausible reset tracker for the gallery: a chance forecast rather than an announcement, so
    /// the preview never reads like real news.
    private static func sampleResets(now: Date, vietnamese: Bool) -> GlanceResets {
        var calendar = Calendar(identifier: .gregorian)
        calendar.locale = Locale(identifier: vietnamese ? "vi_VN" : "en_US")
        calendar.firstWeekday = 2
        let today = calendar.startOfDay(for: now)
        let weekday = (calendar.component(.weekday, from: today) + 5) % 7
        let firstMonday = calendar.date(byAdding: .day, value: -(weekday + 19 * 7), to: today) ?? today
        let pattern = Array("..r....r...r.....r.r..r....b..r...rr....r.....r.r...b..r....r..r.....r.r..r...r...r..b...r....r.r...r.....rr...r....b..r..r...r....r...r..r")
        let todayIndex = 19 * 7 + weekday
        let cells = String((0..<140).map { index -> Character in
            if index > todayIndex { return "-" }
            if index == todayIndex { return "." }
            return pattern[index % pattern.count]
        })
        var months: [GlanceResetMonth] = []
        var lastMonth = -1
        for week in 0..<20 {
            guard let date = calendar.date(byAdding: .day, value: week * 7, to: firstMonday) else { continue }
            let month = calendar.component(.month, from: date)
            if month != lastMonth {
                let symbol = vietnamese ? "Th\(month)" : calendar.shortMonthSymbols[month - 1]
                months.append(GlanceResetMonth(week: week, label: symbol))
                lastMonth = month
            }
        }
        let lastReset = now.addingTimeInterval(-1.7 * 86_400)
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: vietnamese ? "vi_VN" : "en_US")
        formatter.setLocalizedDateFormatFromTemplate("EEEdMHHmm")
        let weekdays = vietnamese ? ["T2", "T3", "T4", "T5", "T6", "T7", "CN"] : ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        return GlanceResets(
            title: vietnamese ? "Reset Codex" : "Codex Resets",
            source: vietnamese ? "Theo codex-resets.com" : "From codex-resets.com",
            brand: "codex",
            color: "#10A37F",
            latest: GlanceLatestReset(
                at: lastReset,
                kind: "regular",
                label: vietnamese ? "Lần gần nhất" : "Last reset",
                kindLabel: vietnamese ? "Reset thường" : "Regular reset",
                since: GlanceCountdown(at: lastReset, text: vietnamese ? "Đã {d} chưa có reset" : "{d} since the last reset", since: true),
                when: formatter.string(from: lastReset)
            ),
            forecastTitle: vietnamese ? "Khả năng có reset" : "Chance of a reset",
            forecast: [
                GlanceResetChance(days: 1, percent: 22, label: vietnamese ? "24 giờ tới" : "Next 24 hours"),
                GlanceResetChance(days: 3, percent: 52, label: vietnamese ? "3 ngày tới" : "Next 3 days"),
                GlanceResetChance(days: 7, percent: 82, label: vietnamese ? "7 ngày tới" : "Next 7 days"),
            ],
            forecastNote: vietnamese ? "Ước tính từ nhịp reset gần đây." : "Estimated from the recent rhythm of resets.",
            calendar: GlanceResetCalendar(
                title: vietnamese ? "20 tuần gần đây" : "Last 20 weeks",
                weeks: 20,
                cells: cells,
                today: todayIndex,
                weekdays: weekdays,
                months: months,
                legend: GlanceResetLegend(
                    regular: vietnamese ? "Reset thường" : "Regular",
                    banked: vietnamese ? "Lượt để dành" : "Banked",
                    today: vietnamese ? "Hôm nay" : "Today"
                )
            ),
            rhythm: GlanceResetRhythm(
                title: vietnamese ? "Nhịp reset" : "Reset rhythm",
                total: 54,
                weekdayTitle: vietnamese ? "Theo thứ" : "By weekday",
                weekdays: zip(weekdays, [4, 11, 9, 8, 6, 7, 9]).map { GlanceResetBucket(label: $0, count: $1) },
                hourTitle: vietnamese ? "Theo giờ trong ngày" : "By hour of day",
                hours: zip(["0–4", "4–8", "8–12", "12–16", "16–20", "20–24"], [12, 18, 13, 6, 1, 4]).map { GlanceResetBucket(label: $0, count: $1) }
            )
        )
    }
}
