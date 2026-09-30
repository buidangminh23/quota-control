import AppKit
import SwiftUI

/// The capsule meter (the popup's `Meter`): a faint track and a flat fill in the pace color. Any
/// non-zero fill is at least as wide as the bar is tall, so 1-2% never disappears.
struct GlanceMeter: View {
    let fraction: Double
    let severity: GlanceSeverity
    var onDark = false
    var height: CGFloat = 5

    var body: some View {
        GeometryReader { proxy in
            let width = proxy.size.width
            let clamped = min(max(fraction, 0), 1)
            let fill = clamped > 0 ? max(width * clamped, height) : 0
            ZStack(alignment: .leading) {
                Capsule().fill(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12))
                if fill > 0 {
                    Capsule()
                        .fill(GlancePalette.fill(severity, onDark: onDark))
                        .frame(width: min(fill, width))
                }
            }
        }
        .frame(height: height)
    }
}

/// A round gauge: a faint track and an arc in the pace color starting at twelve o'clock, with
/// whatever `center` draws inside. A metric without a limit draws the track alone.
struct GlanceRing<Center: View>: View {
    let fraction: Double?
    let severity: GlanceSeverity
    var onDark = false
    var lineWidth: CGFloat = 4
    @ViewBuilder var center: () -> Center

    var body: some View {
        ZStack {
            Circle()
                .stroke(onDark ? Color.white.opacity(0.16) : Color.primary.opacity(0.12), lineWidth: lineWidth)
            if let fraction, fraction > 0 {
                Circle()
                    .trim(from: 0, to: min(max(fraction, 0.03), 1))
                    .stroke(
                        GlancePalette.fill(severity, onDark: onDark),
                        style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)
                    )
                    .rotationEffect(.degrees(-90))
            }
            center()
        }
        .padding(lineWidth / 2)
    }
}

/// When the metric comes back: a countdown against `now`, the reset moment itself, or the metric's
/// own detail text.
struct GlanceResetText: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date

    var body: some View {
        Text(text)
            .monospacedDigit()
            .lineLimit(1)
    }

    private var text: String {
        if let resetsAt = metric.resetsAt {
            if resetsAt <= now { return labels.resetting }
            return "\(labels.resetsIn) \(GlanceFormat.countdown(to: resetsAt, from: now, units: labels.units))"
        }
        return metric.detail ?? ""
    }
}

/// One metric: its label and headline over the meter, and the reset countdown beneath.
struct GlanceMetricRow: View {
    let metric: GlanceMetric
    let labels: GlanceLabels
    let now: Date
    var onDark = false
    var compact = false
    var showsReset = true

    var body: some View {
        VStack(alignment: .leading, spacing: compact ? 2 : 3) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(metric.label)
                    .font(.system(size: compact ? 10.5 : 11.5, weight: .medium))
                    .foregroundStyle(onDark ? Color.white.opacity(0.72) : Color.secondary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text(metric.headline)
                    .font(.system(size: compact ? 11 : 12.5, weight: .semibold))
                    .foregroundStyle(GlancePalette.text(metric.severity, onDark: onDark))
                    .monospacedDigit()
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            if let fraction = metric.fraction {
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: onDark, height: compact ? 4 : 5)
            }
            if showsReset, metric.resetsAt != nil || metric.detail != nil {
                GlanceResetText(metric: metric, labels: labels, now: now)
                    .font(.system(size: compact ? 9.5 : 10.5))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary.opacity(0.9))
            }
        }
    }
}

/// A provider's mark in its brand color beside its name and plan, with the account's email under.
struct GlanceProviderHeader: View {
    let provider: GlanceProvider
    var shows: GlanceShows = .all
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 5) {
                ProviderMark(mark: provider.mark)
                    .foregroundStyle(onDark ? provider.tint : markColor)
                    .frame(width: size, height: size)
                Text(provider.name)
                    .font(.system(size: size - 1, weight: .semibold))
                    .foregroundStyle(onDark ? Color.white : Color.primary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if shows.plan, let plan = provider.plan {
                    GlancePlanBadge(text: plan, onDark: onDark, size: size)
                }
            }
            if shows.account, let account = provider.account {
                Text(account)
                    .font(.system(size: max(size - 3.5, 8.5)))
                    .foregroundStyle(onDark ? Color.white.opacity(0.55) : Color.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.leading, size + 5)
            }
        }
    }

    private var markColor: Color {
        provider.color.uppercased() == "#FFFFFF" ? .primary : provider.tint
    }
}

/// The plan (`Pro`, `Max 5x`) in a small rounded tag.
struct GlancePlanBadge: View {
    let text: String
    var onDark = false
    var size: CGFloat = 13

    var body: some View {
        Text(text)
            .font(.system(size: max(size - 4, 8), weight: .semibold))
            .foregroundStyle(onDark ? Color.white.opacity(0.7) : Color.secondary)
            .lineLimit(1)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(
                RoundedRectangle(cornerRadius: 3, style: .continuous)
                    .fill(onDark ? Color.white.opacity(0.12) : Color.primary.opacity(0.08))
            )
            .fixedSize()
    }
}

/// Why an account shows no readings: signed out, session expired, no data yet.
struct GlanceNoticeRow: View {
    let text: String
    var onDark = false
    var size: CGFloat = 10.5

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: size - 1))
            Text(text)
                .font(.system(size: size))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .foregroundStyle(onDark ? Color(red: 1.0, green: 0.62, blue: 0.04) : Color.orange)
    }
}


/// How a line of a reset card reads, in the Reset tab's type styles.
enum GlanceResetTextStyle {
    /// 11pt in the label color, whole: a quoted announcement, which is cut short already.
    case body
    /// A post's words on a card, cut after four lines like the Reset tab's.
    case post
    /// A post's words in a history row: the secondary color, cut after three lines.
    case rowPost
    /// 10pt in the secondary color: a card's meta lines, a note, an explanation.
    case secondary
    /// 11pt in the secondary color: when the feed was read, the line above the source.
    case status
    /// 11pt semibold: the current wait.
    case heading
    /// 20pt bold: a chance.
    case value
}

/// Words on a reset card that move with the clock, worded when the card is drawn at the moment
/// the surface passes down (`glanceResetNow`: the island's clock, a widget entry's date). The cards,
/// and the pages a widget cuts them into, then stay the same from one minute to the next, and a
/// widget can give each change of the words a timeline entry of its own (`changes`).
enum GlanceResetLiveText: Equatable {
    /// A countdown's words: `Còn 3 giờ`, `Tải 5 phút trước`, `Vừa tải`.
    case countdown(GlanceCountdown, GlanceUnits)
    /// How long ago, in the Reset tab's relative words: `12 phút trước`, `3 ngày trước`.
    case ago(Date, locale: String)
    /// A countdown's words, then fixed ones: a scheduled card's `Thông báo 2 giờ trước · Chưa nói giờ cụ thể`.
    case countdownThen(GlanceCountdown, GlanceUnits, String)
    /// The time left to a stated time, then, once it has passed, how long ago it was.
    case dueThenOverdue(GlanceCountdown, GlanceCountdown?, GlanceUnits)

    func text(at now: Date) -> String {
        switch self {
        case let .countdown(countdown, units):
            return countdown.text(now: now, units: units)
        case let .ago(at, locale):
            return GlanceResetLatestPresentation.ago(since: at, now: now, locale: locale)
        case let .countdownThen(countdown, units, rest):
            return countdown.text(now: now, units: units) + " · " + rest
        case let .dueThenOverdue(due, overdue, units):
            if due.at <= now, let overdue { return overdue.text(now: now, units: units) }
            return due.text(now: now, units: units)
        }
    }

    /// The moment the words count from or to.
    private var anchor: Date {
        switch self {
        case let .countdown(countdown, _), let .countdownThen(countdown, _, _), let .dueThenOverdue(countdown, _, _):
            return countdown.at
        case let .ago(at, _):
            return at
        }
    }

    /// The moments after `now`, up to `end`, when the words change. Every change falls a whole
    /// number of minutes from `anchor` (the words count minutes, then hours or days), so the words
    /// are tried a second after each of those minutes and kept where they differ.
    func changes(after now: Date, until end: Date) -> [Date] {
        var moments: [Date] = []
        var previous = text(at: now)
        var minute = (now.timeIntervalSince(anchor) / 60).rounded(.down)
        while true {
            let moment = anchor.addingTimeInterval(minute * 60 + 1)
            minute += 1
            if moment <= now { continue }
            if moment > end { break }
            let words = text(at: moment)
            if words != previous {
                moments.append(moment)
                previous = words
            }
        }
        return moments
    }
}

private struct GlanceResetNowKey: EnvironmentKey {
    static var defaultValue: Date { Date() }
}

extension EnvironmentValues {
    /// The moment reset cards word their moving words at (`GlanceResetLiveText`).
    var glanceResetNow: Date {
        get { self[GlanceResetNowKey.self] }
        set { self[GlanceResetNowKey.self] = newValue }
    }
}

enum GlanceResetElement {
    case text(String, GlanceResetTextStyle)
    /// Words that move with the clock, in a text style.
    case live(GlanceResetLiveText, GlanceResetTextStyle)
    /// Who posted, as a card names them: the picture (or the initial) and the handle.
    case author(GlanceResetAuthor, String)
    /// The latest reset's time since, big on a yellow tag.
    case badge(GlanceResetLiveText)
    case chances([GlanceResetForecastChance])
    case meter(Double)
    case calendar(GlanceResetCalendar, Range<Int>, Range<Int>)
    /// The calendar's key, the entries listed with their colors; a widget page too short for the
    /// whole key gets one entry at a time.
    case legend(GlanceResetLegend, [GlanceResetLegend.Item])
    case rhythm(String, [GlanceResetBucket])
    /// One statistic: its name on the left, the value on the right.
    case stat(String, String)
    /// The post on X, worded like the Reset tab's link to it.
    case link(String, String)
    case divider
    /// An announcement quoted in a box, like the message under the Reset tab's latest reset.
    case message([GlanceResetElement])
    /// A history row, kept together on a widget page: its head line, the words, the notes under.
    case row([GlanceResetElement])
    /// A history row's first line: who posted, the kind, whether the site reviewed it, when, the post.
    case rowHead(GlanceResetRowHead)
    /// The attribution under the cards, with the link to the site.
    case source(String, String)
    /// A part the Reset tab folds: its words, and whether it is open.
    case fold(GlanceResetFold, String, Bool)
}

/// The first line of a history row, as the Reset tab draws it.
struct GlanceResetRowHead {
    var author: GlanceResetAuthor?
    /// The poster's picture; empty for an author the tracker has none of, drawn as an initial.
    var avatar: String
    var kind: String
    var kindLabel: String
    /// `Chưa kiểm chứng`, while the site has not reviewed the entry (Claude).
    var provisional: String?
    var when: String
    var url: String?
    /// What the post's link says to VoiceOver.
    var linkLabel: String
}

/// The parts of the reset cards the Reset tab folds: the history after its first rows, behind
/// "Xem thêm N", and how the numbers are worked out, behind "Cách tính".
enum GlanceResetFold: String {
    case history
    case method
}

/// Which folds a surface draws open. The island folds the history like the Reset tab does; a
/// widget pages through every row instead, so the history folds only where `foldsHistory` says.
struct GlanceResetFolds: Equatable {
    /// The history rows the Reset tab lists before its "Xem thêm N" button.
    static let historyPreview = 8

    var foldsHistory = false
    var historyOpen = false
    var methodOpen = false

    func isOpen(_ fold: GlanceResetFold) -> Bool {
        switch fold {
        case .history: return historyOpen
        case .method: return methodOpen
        }
    }

    mutating func toggle(_ fold: GlanceResetFold) {
        switch fold {
        case .history: historyOpen.toggle()
        case .method: methodOpen.toggle()
        }
    }
}

/// How a surface makes a fold's row clickable: the island flips its own state with a button, a
/// widget runs an App Intent. Without one, the row is drawn and does nothing.
struct GlanceResetFoldAction {
    let wrap: (_ fold: GlanceResetFold, _ open: Bool, _ label: AnyView) -> AnyView
}

private struct GlanceResetFoldActionKey: EnvironmentKey {
    static let defaultValue: GlanceResetFoldAction? = nil
}

extension EnvironmentValues {
    var glanceResetFoldAction: GlanceResetFoldAction? {
        get { self[GlanceResetFoldActionKey.self] }
        set { self[GlanceResetFoldActionKey.self] = newValue }
    }
}

/// The few words the reset cards add around the document's own, as the Reset tab words them
/// (`openPost`, `showMore` and `showLess` in src/i18n/insightsVi.ts and insightsEn.ts; a test
/// there keeps the two in step), so the document need not carry them.
struct GlanceResetWords {
    let vietnamese: Bool

    init(locale: String) {
        vietnamese = locale.lowercased().hasPrefix("vi")
    }

    var openPost: String { vietnamese ? "Mở bài trên X" : "Open the post on X" }

    func showMore(_ count: Int) -> String { vietnamese ? "Xem thêm \(count)" : "Show \(count) more" }

    var showLess: String { vietnamese ? "Thu gọn" : "Show less" }
}

/// How a card sits on the page: filled like the Reset tab's cards, a list whose rows run between
/// hairlines from edge to edge (`inset` above the first and below the last), or plain lines under
/// the cards like the tab's notes and footnotes.
enum GlanceResetCardLook {
    case card
    case list(inset: CGFloat)
    case plain
}

struct GlanceResetCardData: Identifiable {
    let id: String
    var title: String
    var accent: Color?
    var look: GlanceResetCardLook = .card
    var elements: [GlanceResetElement]
}

struct GlanceResetPalette {
    let scheme: ColorScheme
    /// What the tracker's cards sit on, like the Reset tab's page.
    var background: Color { Color(glanceHex: scheme == .dark ? "#1e1e1e" : "#ffffff")! }
    var card: Color { Color(glanceHex: scheme == .dark ? "#2a2a2b" : "#f4f4f5")! }
    var blue: Color { Color(glanceHex: scheme == .dark ? "#0a84ff" : "#007aff")! }
    var yellow: Color { Color(glanceHex: scheme == .dark ? "#ffd60a" : "#f5b800")! }
    /// What a quoted announcement sits on inside a card, the popup's `--uc-quinary`.
    var message: Color { scheme == .dark ? Color.white.opacity(0.07) : Color.black.opacity(0.05) }
    /// The popup's `--uc-quaternary`: a badge's fill, an initial's circle, the ring round a picture.
    var quaternary: Color { scheme == .dark ? Color.white.opacity(0.12) : Color.black.opacity(0.08) }
    /// The popup's `--uc-tertiary`: the source line, the rhythm's counts and labels.
    var tertiary: Color { scheme == .dark ? Color.white.opacity(0.5) : Color.black.opacity(0.56) }
    /// The popup's `--uc-separator`: the hairline between a list's rows.
    var separator: Color { scheme == .dark ? Color.white.opacity(0.1) : Color.black.opacity(0.1) }
    /// The popup's `--uc-orange` and `--uc-notice-text`, for the "not reviewed" badge.
    var notice: Color { Color(glanceHex: scheme == .dark ? "#ff9f0a" : "#ff9500")! }
    var noticeText: Color { Color(glanceHex: scheme == .dark ? "#ff9f0a" : "#b25900")! }
    static let orange = Color(glanceHex: "#ff9500")!
}

struct GlanceResetCardView: View {
    let card: GlanceResetCardData
    let availableWidth: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    private var grouped: Bool {
        ["forecast", "calendar", "rhythm", "stats", "history"].contains { card.id.hasPrefix($0) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if grouped && !card.title.isEmpty {
                Text(card.title).font(.system(size: 10, weight: .semibold)).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 8)
            }
            switch card.look {
            case .card:
                filled(VStack(alignment: .leading, spacing: 8) {
                    if !grouped && !card.title.isEmpty {
                        Text(card.id.hasPrefix("latest") ? card.title.uppercased() : card.title)
                            .font(.system(size: card.id.hasPrefix("latest") ? 10 : 14, weight: card.id.hasPrefix("latest") ? .semibold : .bold))
                            .tracking(card.id.hasPrefix("latest") ? 0.6 : 0)
                            .foregroundStyle(card.id.hasPrefix("latest") ? Color.secondary : Color.primary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    ForEach(Array(card.elements.enumerated()), id: \.offset) { _, element in
                        GlanceResetElementView(element: element, availableWidth: max(1, availableWidth - 24))
                    }
                }
                .padding(12))
            case let .list(inset):
                filled(VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(card.elements.enumerated()), id: \.offset) { _, element in
                        let padding = Self.listPadding(element)
                        GlanceResetElementView(element: element, availableWidth: max(1, availableWidth - padding.leading - padding.trailing))
                            .padding(padding)
                    }
                }
                .padding(.vertical, inset))
            case .plain:
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(card.elements.enumerated()), id: \.offset) { _, element in
                        GlanceResetElementView(element: element, availableWidth: max(1, availableWidth - 16))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 8)
            }
        }
        .frame(width: availableWidth, alignment: .leading)
    }

    private func filled<Content: View>(_ content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(GlanceResetPalette(scheme: colorScheme).card))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(card.accent ?? Color.primary.opacity(0.09), lineWidth: card.accent == nil ? 0.5 : 2))
    }

    /// A list row's room, the Reset tab's: history rows 9pt by 12pt, a statistic 5pt by 12pt, the
    /// "Xem thêm" button a little more below than above; hairlines run edge to edge.
    private static func listPadding(_ element: GlanceResetElement) -> EdgeInsets {
        switch element {
        case .divider: return EdgeInsets()
        case .row: return EdgeInsets(top: 9, leading: 12, bottom: 9, trailing: 12)
        case .stat: return EdgeInsets(top: 5, leading: 12, bottom: 5, trailing: 12)
        case .fold: return EdgeInsets(top: 6, leading: 12, bottom: 8, trailing: 12)
        default: return EdgeInsets(top: 4, leading: 12, bottom: 4, trailing: 12)
        }
    }
}

/// A poster's round picture or, for an author the tracker has no picture of, the initial in a faint
/// circle, like the Reset tab's `ResetAuthorAvatar`.
struct GlanceResetAvatar: View {
    let handle: String
    let picture: String
    let size: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    private static let cache = NSCache<NSString, NSImage>()

    var body: some View {
        let palette = GlanceResetPalette(scheme: colorScheme)
        Group {
            if let image = Self.image(picture) {
                Image(nsImage: image).resizable().scaledToFill()
            } else {
                Text(initial)
                    .font(.system(size: (size * 0.55).rounded(), weight: .bold))
                    .foregroundStyle(.secondary)
                    .frame(width: size, height: size)
                    .background(palette.quaternary)
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .background(Circle().fill(palette.quaternary).padding(-1))
    }

    private var initial: String {
        String(handle.drop { $0 == "@" }.prefix(1)).uppercased()
    }

    /// A `data:image/…;base64,` picture, decoded once per distinct picture.
    static func image(_ value: String) -> NSImage? {
        guard !value.isEmpty else { return nil }
        let key = value as NSString
        if let image = cache.object(forKey: key) { return image }
        guard let comma = value.firstIndex(of: ","), value.hasPrefix("data:image/"),
              let data = Data(base64Encoded: String(value[value.index(after: comma)...])),
              let image = NSImage(data: data) else { return nil }
        cache.setObject(image, forKey: key)
        return image
    }
}

struct GlanceResetElementView: View {
    let element: GlanceResetElement
    let availableWidth: CGFloat
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.glanceResetFoldAction) private var foldAction
    @Environment(\.glanceResetNow) private var now
    private var palette: GlanceResetPalette { GlanceResetPalette(scheme: colorScheme) }

    @ViewBuilder
    var body: some View {
        switch element {
        case let .text(text, style):
            styled(text, style)
        case let .live(text, style):
            styled(text.text(at: now), style)
        case let .author(author, avatar):
            HStack(spacing: 6) {
                GlanceResetAvatar(handle: author.handle, picture: avatar, size: 22)
                Text(author.handle)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        case let .badge(text):
            Text(text.text(at: now))
                .font(.system(size: 24, weight: .heavy))
                .foregroundStyle(Color(red: 0.11, green: 0.11, blue: 0.12))
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 10).padding(.vertical, 2)
                .background(RoundedRectangle(cornerRadius: 6).fill(palette.yellow))
        case let .chances(chances):
            HStack(alignment: .top, spacing: 12) {
                ForEach(chances) { chance in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(chance.percent).font(.system(size: 20, weight: .bold)).monospacedDigit()
                        meter(chance.fraction)
                        Text(chance.label).font(.system(size: 10)).foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        case let .meter(fraction):
            meter(fraction)
        case let .calendar(calendar, weeks, days):
            calendarGrid(calendar, weeks: weeks, days: days)
        case let .legend(legend, items):
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) { legendItems(legend, items) }
                VStack(alignment: .leading, spacing: 5) { legendItems(legend, items) }
            }
            .font(.system(size: 10.5)).foregroundStyle(.secondary)
        case let .rhythm(title, buckets):
            rhythm(title, buckets)
        case let .stat(label, value):
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Text(label).foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                    Text(value).fontWeight(.semibold).monospacedDigit()
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text(label).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                    Text(value).fontWeight(.semibold).monospacedDigit().fixedSize(horizontal: false, vertical: true)
                }
            }
            .font(.system(size: 11))
        case .divider:
            Rectangle().fill(palette.separator).frame(height: 0.5)
        case let .link(url, label):
            if let destination = Self.web(url) {
                Link(destination: destination) {
                    HStack(spacing: 3) {
                        Text(label).font(.system(size: 11, weight: .medium))
                        Image(systemName: "arrow.up.right.square").font(.system(size: 10))
                    }
                    .foregroundStyle(palette.blue)
                }
                .accessibilityLabel(label)
            }
        case let .message(lines):
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    GlanceResetElementView(element: line, availableWidth: max(1, availableWidth - 20))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(palette.message))
        case let .row(lines):
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    GlanceResetElementView(element: line, availableWidth: availableWidth)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        case let .rowHead(head):
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 6) {
                    rowAuthor(head)
                    rowBadges(head)
                    rowTime(head)
                    Spacer(minLength: 0)
                    rowLink(head)
                }
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        rowAuthor(head)
                        rowBadges(head)
                    }
                    rowTimeAndLink(head)
                }
                VStack(alignment: .leading, spacing: 2) {
                    if head.author != nil {
                        HStack(spacing: 6) { rowAuthor(head) }
                    }
                    HStack(spacing: 6) { rowBadges(head) }
                    rowTimeAndLink(head)
                }
            }
        case let .source(text, url):
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 4) {
                    sourceText(text)
                    sourceLink(url)
                }
                VStack(alignment: .leading, spacing: 4) {
                    sourceText(text).fixedSize(horizontal: false, vertical: true)
                    sourceLink(url)
                }
            }
        case let .fold(fold, label, open):
            if let foldAction {
                foldAction.wrap(fold, open, AnyView(foldLabel(fold, label, open)))
            } else {
                foldLabel(fold, label, open)
            }
        }
    }

    private func styled(_ text: String, _ style: GlanceResetTextStyle) -> some View {
        Text(text)
            .font(.system(size: Self.size(style), weight: style == .value ? .bold : style == .heading ? .semibold : .regular))
            .foregroundStyle(style == .secondary || style == .rowPost || style == .status ? Color.secondary : Color.primary)
            .lineLimit(style == .post ? 4 : style == .rowPost ? 3 : nil)
            .fixedSize(horizontal: false, vertical: true)
    }

    private static func size(_ style: GlanceResetTextStyle) -> CGFloat {
        switch style {
        case .value: return 20
        case .secondary: return 10
        case .body, .post, .rowPost, .heading, .status: return 11
        }
    }

    private static func web(_ url: String) -> URL? {
        guard let destination = URL(string: url), ["https", "http"].contains(destination.scheme ?? "") else { return nil }
        return destination
    }

    /// The attribution, which like the Reset tab's source line puts its link after the words, or
    /// under them once the words take more than a line.
    private func sourceText(_ text: String) -> some View {
        Text(text).font(.system(size: 10)).foregroundStyle(palette.tertiary)
    }

    @ViewBuilder
    private func sourceLink(_ url: String) -> some View {
        if let destination = Self.web(url) {
            Link(destination: destination) {
                Image(systemName: "arrow.up.right.square").font(.system(size: 10)).foregroundStyle(palette.blue)
            }
            .accessibilityLabel(url)
        }
    }

    private func meter(_ fraction: Double) -> some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Color.primary.opacity(0.1))
                Capsule().fill(palette.blue).frame(width: proxy.size.width * min(max(fraction, 0), 1))
            }
        }.frame(height: 5)
    }

    /// Bars per weekday or per four-hour block, as the Reset tab draws them: an empty slot has no
    /// count over it, and the busiest one's bar and count stand out.
    private func rhythm(_ title: String, _ buckets: [GlanceResetBucket]) -> some View {
        let peak = max(1, buckets.map(\.count).max() ?? 0)
        return VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.system(size: 10)).foregroundStyle(.secondary)
            HStack(alignment: .bottom, spacing: 4) {
                ForEach(Array(buckets.enumerated()), id: \.offset) { _, bucket in
                    let isPeak = bucket.count == peak
                    VStack(spacing: 2) {
                        Text(bucket.count > 0 ? "\(bucket.count)" : " ")
                            .font(.system(size: 8.5, weight: isPeak ? .bold : .regular))
                            .monospacedDigit()
                            .foregroundStyle(isPeak ? Color.primary : palette.tertiary)
                            .lineLimit(1)
                            .frame(height: 11)
                        UnevenRoundedRectangle(topLeadingRadius: 2, topTrailingRadius: 2)
                            .fill(palette.blue.opacity(isPeak ? 1 : 0.45))
                            .frame(height: max(1, 30 * CGFloat(bucket.count) / CGFloat(peak)))
                            .frame(height: 30, alignment: .bottom)
                        Text(bucket.label)
                            .font(.system(size: 8.5))
                            .foregroundStyle(palette.tertiary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
        }
    }

    @ViewBuilder
    private func rowAuthor(_ head: GlanceResetRowHead) -> some View {
        if let author = head.author {
            GlanceResetAvatar(handle: author.handle, picture: head.avatar, size: 18)
            if head.avatar.isEmpty || GlanceResetAvatar.image(head.avatar) == nil {
                Text(author.handle)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    @ViewBuilder
    private func rowBadges(_ head: GlanceResetRowHead) -> some View {
        if head.kind == "banked" {
            chip(head.kindLabel, fill: palette.blue.opacity(0.16), ink: palette.blue)
        } else {
            chip(head.kindLabel, fill: palette.quaternary, ink: Color.secondary)
        }
        if let provisional = head.provisional {
            chip(provisional, fill: palette.notice.opacity(0.18), ink: palette.noticeText)
        }
    }

    private func rowTime(_ head: GlanceResetRowHead) -> some View {
        Text(head.when)
            .font(.system(size: 10))
            .foregroundStyle(.secondary)
            .monospacedDigit()
            .lineLimit(1)
    }

    private func rowTimeAndLink(_ head: GlanceResetRowHead) -> some View {
        HStack(spacing: 6) {
            rowTime(head)
            Spacer(minLength: 0)
            rowLink(head)
        }
    }

    @ViewBuilder
    private func rowLink(_ head: GlanceResetRowHead) -> some View {
        if let url = head.url, let destination = Self.web(url) {
            Link(destination: destination) {
                Image(systemName: "arrow.up.right.square").font(.system(size: 10)).foregroundStyle(palette.blue)
            }
            .accessibilityLabel(head.linkLabel)
        }
    }

    private func chip(_ text: String, fill: Color, ink: Color) -> some View {
        Text(text)
            .font(.system(size: 9.5, weight: .semibold))
            .foregroundStyle(ink)
            .lineLimit(1)
            .padding(.horizontal, 5)
            .frame(height: 15)
            .background(RoundedRectangle(cornerRadius: 4, style: .continuous).fill(fill))
            .fixedSize()
    }

    /// A fold's row: the history's "Xem thêm N" in the accent color, or "Cách tính" with its chevron.
    @ViewBuilder
    private func foldLabel(_ fold: GlanceResetFold, _ label: String, _ open: Bool) -> some View {
        switch fold {
        case .history:
            Text(label)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(palette.blue)
                .contentShape(Rectangle())
        case .method:
            HStack(spacing: 4) {
                Text(label).font(.system(size: 10, weight: .semibold))
                Image(systemName: open ? "chevron.up" : "chevron.down").font(.system(size: 7.5, weight: .bold))
            }
            .foregroundStyle(.secondary)
            .contentShape(Rectangle())
        }
    }

    private func legendItems(_ legend: GlanceResetLegend, _ items: [GlanceResetLegend.Item]) -> some View {
        ForEach(items, id: \.self) { item in
            switch item {
            case .regular:
                HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).fill(palette.blue).frame(width: 9, height: 9); Text(legend.regular) }
            case .banked:
                HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).fill(Color.orange).frame(width: 9, height: 9); Text(legend.banked) }
            case .today:
                HStack(spacing: 4) { RoundedRectangle(cornerRadius: 2).strokeBorder(Color.primary, lineWidth: 1).frame(width: 9, height: 9); Text(legend.today) }
            }
        }
    }

    private func calendarGrid(_ calendar: GlanceResetCalendar, weeks: Range<Int>, days: Range<Int>) -> some View {
        let rows = calendar.weekRows()
        let pitch = max(3, (availableWidth - 22) / CGFloat(max(weeks.count, 1)))
        let side = min(16, pitch - 2)
        return VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 0) {
                Color.clear.frame(width: 22, height: 11)
                ForEach(Array(weeks), id: \.self) { week in
                    Text(calendar.months.first(where: { $0.week == week })?.label ?? "")
                        .font(.system(size: 8.5)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: true, vertical: false)
                        .frame(width: pitch, alignment: .leading)
                }
            }
            ForEach(Array(days), id: \.self) { day in
                HStack(spacing: 0) {
                    Text(day < calendar.weekdays.count ? calendar.weekdays[day] : "")
                        .font(.system(size: 8.5)).foregroundStyle(.secondary).frame(width: 22, alignment: .leading)
                    ForEach(Array(weeks), id: \.self) { week in
                        let value = week < rows.count && day < rows[week].count ? rows[week][day] : .future
                        RoundedRectangle(cornerRadius: 2)
                            .fill(value == .regular ? palette.blue : value == .banked ? GlanceResetPalette.orange : Color.primary.opacity(value == .future ? 0.035 : 0.1))
                            .overlay(RoundedRectangle(cornerRadius: 2).strokeBorder(Color.primary.opacity(week * 7 + day == calendar.today ? 1 : 0), lineWidth: 1.5))
                            .frame(width: max(1, side), height: max(1, side))
                            .frame(width: pitch, height: max(1, side))
                    }
                }
            }
        }
    }
}

/// Whose resets a reset view shows, named at its top as the Reset tab's switch names them: the
/// tracker's mark in its color beside its title.
struct GlanceResetHeading: View {
    let resets: GlanceResets
    var markSize: CGFloat = 14
    var fontSize: CGFloat = 12

    var body: some View {
        HStack(spacing: 6) {
            ProviderMark(mark: resets.mark)
                .foregroundStyle(resets.markTint)
                .frame(width: markSize, height: markSize)
            Text(resets.title)
                .font(.system(size: fontSize, weight: .semibold))
                .foregroundStyle(Color.primary)
                .lineLimit(1)
        }
        .accessibilityElement(children: .combine)
    }
}

struct GlanceResetContent: View {
    @Environment(\.colorScheme) private var colorScheme
    let resets: GlanceResets
    let units: GlanceUnits
    let now: Date
    let availableWidth: CGFloat
    /// Which folds are open, and whether the history folds at all (the island's own state).
    var folds = GlanceResetFolds()
    /// Opens or closes a fold; without it the folds' rows are drawn and do nothing.
    var onFold: ((GlanceResetFold) -> Void)? = nil
    /// Names the tracker above the cards, where nothing else around the view does.
    var showsHeading = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if showsHeading {
                GlanceResetHeading(resets: resets)
            }
            ForEach(GlanceResetCards.make(resets: resets, units: units, now: now, folds: folds)) { card in
                GlanceResetCardView(card: card, availableWidth: availableWidth)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.glanceResetFoldAction, onFold.map { toggle in
            GlanceResetFoldAction { fold, _, label in
                AnyView(Button { toggle(fold) } label: { label }.buttonStyle(.plain))
            }
        })
        .environment(\.glanceResetNow, now)
        .environment(\.colorScheme, resets.theme == "dark" ? .dark : resets.theme == "light" ? .light : colorScheme)
    }
}


enum GlanceResetCards {
    static func make(resets: GlanceResets, units: GlanceUnits, now: Date, folds: GlanceResetFolds = GlanceResetFolds()) -> [GlanceResetCardData] {
        var cards: [GlanceResetCardData] = []
        func add(_ id: String, _ title: String, _ elements: [GlanceResetElement], accent: Color? = nil, look: GlanceResetCardLook = .card) {
            cards.append(GlanceResetCardData(id: id, title: title, accent: accent, look: look, elements: elements))
        }
        if let stale = resets.stale {
            add("stale", "", [.text(stale, .secondary)], look: .plain)
        }
        if let presentation = resets.presentation {
            let words = GlanceResetWords(locale: presentation.locale)
            if let latest = presentation.latest {
                let author = latest.author.map { GlanceResetElement.author($0, presentation.avatar(for: $0)) }
                var elements: [GlanceResetElement] = []
                if latest.excerpt == nil, let author { elements.append(author) }
                elements += [.badge(.ago(latest.at, locale: presentation.locale)), .text(latest.meta, .secondary)]
                if let excerpt = latest.excerpt {
                    var lines: [GlanceResetElement] = author.map { [$0] } ?? []
                    lines.append(.text(excerpt, .body))
                    if let observed = latest.observed { lines.append(.text(observed, .secondary)) }
                    if let url = latest.url { lines.append(.link(url, words.openPost)) }
                    elements.append(.message(lines))
                }
                elements += (latest.notes ?? []).map { .text($0, .secondary) }
                add("latest", latest.title, elements)
            }
            let quoted = presentation.latest?.excerpt != nil
            for status in presentation.statuses(at: now) {
                let repeats = quoted && status.sameAsLatest == true
                var elements: [GlanceResetElement] = []
                let metadata = status.liveMetadata(units: units)
                if status.kind == "watch", metadata.count > 1 { elements.append(metadata[0]) }
                if let author = status.author, !repeats { elements.append(.author(author, presentation.avatar(for: author))) }
                if let excerpt = status.excerpt, !repeats { elements.append(.text(excerpt, .post)) }
                elements += status.kind == "watch" && metadata.count > 1 ? Array(metadata.dropFirst()) : metadata
                if let due = status.liveDue(units: units) { elements.append(due) }
                if let url = status.url { elements.append(.link(url, words.openPost)) }
                add(status.id, status.title, elements, accent: accent(status.kind))
            }
            let forecast = presentation.forecast
            var elements: [GlanceResetElement] = []
            if !forecast.chances.isEmpty { elements.append(.chances(forecast.chances)) }
            if let wait = forecast.wait { elements += [.divider, .text(wait, .heading)] }
            if let fraction = forecast.waitFraction { elements.append(.meter(fraction)) }
            for text in [forecast.median, forecast.sampleNote, forecast.reliability, forecast.disclaimer, forecast.unavailable].compactMap({ $0 }) {
                elements.append(.text(text, .secondary))
            }
            if !elements.isEmpty { add("forecast", forecast.title, elements) }
        } else {
            if let latest = resets.latest {
                add("latest", latest.label, [.badge(.countdown(latest.since, units)), .text(latest.when, .secondary)])
            }
            if let upcoming = resets.upcoming(at: now) {
                let lines = upcoming.lines(now: now, units: units)
                let value: GlanceResetElement = upcoming.countdown.map { .live(.countdown($0, units), .heading) } ?? .text(lines.value, .heading)
                var elements: [GlanceResetElement] = [value, .text(lines.caption, .secondary)]
                if let note = upcoming.note { elements.append(.text(note, .secondary)) }
                add("upcoming", upcoming.title, elements, accent: upcoming.tone == .positive ? .green : GlanceResetPalette.orange)
            }
            var elements: [GlanceResetElement] = []
            if !resets.forecast.isEmpty {
                elements.append(.chances(resets.forecast.map { GlanceResetForecastChance(days: $0.days, label: $0.label, percent: "\($0.percent)%", fraction: $0.fraction) }))
            }
            for text in [resets.wait, resets.median, resets.forecastNote.isEmpty ? nil : resets.forecastNote].compactMap({ $0 }) {
                elements.append(.text(text, .secondary))
            }
            if !elements.isEmpty { add("forecast", resets.forecastTitle, elements) }
        }
        if let calendar = resets.calendar {
            add("calendar", calendar.title, [.calendar(calendar, 0..<calendar.weekRows().count, 0..<7), .legend(calendar.legend, GlanceResetLegend.Item.allCases)])
        }
        if let rhythm = resets.rhythm {
            var elements: [GlanceResetElement] = [.rhythm(rhythm.weekdayTitle, rhythm.weekdays), .rhythm(rhythm.hourTitle, rhythm.hours)]
            if let text = resets.presentation?.patternNote, !text.isEmpty { elements.append(.text(text, .secondary)) }
            add("rhythm", rhythm.title, elements)
        }
        if let presentation = resets.presentation {
            let words = GlanceResetWords(locale: presentation.locale)
            if !presentation.stats.isEmpty {
                add("stats", presentation.statsTitle, presentation.stats.map { .stat($0.label, $0.value) }, look: .list(inset: 4))
            }
            if !presentation.history.isEmpty {
                add("history", presentation.historyTitle, history(presentation, folds: folds, words: words), look: .list(inset: 0))
            }
            if let fetched = presentation.fetched {
                add("fetched", "", [.live(.countdown(fetched, units), .status)], look: .plain)
            }
            add("source", "", [.source(presentation.source, resets.site ?? "https://codex-resets.com")], look: .plain)
            if !presentation.method.isEmpty {
                var elements: [GlanceResetElement] = [.fold(.method, presentation.methodTitle, folds.methodOpen)]
                if folds.methodOpen { elements += presentation.method.map { .text($0, .secondary) } }
                add("method", "", elements, look: .plain)
            }
        }
        return cards
    }

    /// The moments after `now`, up to `end`, when a moving word on `resets`' cards changes, at most
    /// one a minute: the last of each minute, by when every change in it has happened. A widget
    /// gives each its own timeline entry, so the words move with the clock like the popup's.
    static func ticks(resets: GlanceResets, units: GlanceUnits, after now: Date, until end: Date) -> [Date] {
        var moments: [Date] = []
        func collect(_ elements: [GlanceResetElement]) {
            for element in elements {
                switch element {
                case let .live(text, _), let .badge(text):
                    moments += text.changes(after: now, until: end)
                case let .message(lines), let .row(lines):
                    collect(lines)
                default:
                    break
                }
            }
        }
        collect(make(resets: resets, units: units, now: now).flatMap(\.elements))
        let byMinute = Dictionary(grouping: moments) { ($0.timeIntervalSince1970 / 60).rounded(.down) }
        return byMinute.values.compactMap { $0.max() }.sorted()
    }

    /// The Reset tab's history as one list: a row per reset between hairlines, its head line with
    /// who posted, the kind and when, then its words and notes. Where the history folds, the first
    /// rows come before a button for the rest, as in the tab.
    private static func history(_ presentation: GlanceResetPresentation, folds: GlanceResetFolds, words: GlanceResetWords) -> [GlanceResetElement] {
        let preview = GlanceResetFolds.historyPreview
        let folding = folds.foldsHistory && presentation.history.count > preview
        let shown = folding && !folds.historyOpen ? Array(presentation.history.prefix(preview)) : presentation.history
        var elements: [GlanceResetElement] = []
        for item in shown {
            if !elements.isEmpty { elements.append(.divider) }
            let head = GlanceResetRowHead(
                author: item.author,
                avatar: item.author.map { presentation.avatar(for: $0) } ?? "",
                kind: item.kind,
                kindLabel: item.kindLabel,
                provisional: item.provisional,
                when: item.when,
                url: item.url,
                linkLabel: words.openPost
            )
            var lines: [GlanceResetElement] = [.rowHead(head), .text(item.excerpt, .rowPost)]
            lines += [item.scope, item.observed].compactMap { $0 }.map { .text($0, .secondary) }
            elements.append(.row(lines))
        }
        if folding {
            let label = folds.historyOpen ? words.showLess : words.showMore(presentation.history.count - preview)
            elements.append(.fold(.history, label, folds.historyOpen))
        }
        return elements
    }

    /// A status card's border: orange for a watch, green for a scheduled reset, blue for a banked
    /// reset still to apply, none for the quiet card.
    private static func accent(_ kind: String) -> Color? {
        switch kind {
        case "watch": return GlanceResetPalette.orange
        case "scheduled": return .green
        case "banked": return .blue
        default: return nil
        }
    }
}
