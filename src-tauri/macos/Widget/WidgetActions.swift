import AppIntents
import Foundation
import SwiftUI
import WidgetKit

/// The step a widget button takes: the first press, or the "Xác nhận" / "Hủy" that follows it for a
/// request that waits for a confirmation (`GlanceActionRequest.needsConfirmation`).
typealias WidgetActionStep = GlanceActionStep

/// The widget button waiting for its confirmation, and the one whose request just went to the app.
/// One confirmation is held at a time and only for `lifetime`: a confirmation the user walked away
/// from goes back to its button instead of spending a reset hours later.
enum WidgetPendingAction {
    static let lifetime: TimeInterval = 60
    static let sentLifetime: TimeInterval = 30

    private static let pendingKey = "glance-action.pending"
    private static let pendingUntilKey = "glance-action.pending-until"
    private static let sentKey = "glance-action.sent"
    private static let sentUntilKey = "glance-action.sent-until"
    private static let pendingReadingKey = "glance-action.pending-reading"
    private static let sentReadingKey = "glance-action.sent-reading"

    /// The key (`GlanceActionRequest.key`) of the button now asking "Xác nhận" or "Hủy".
    static func current(now: Date = Date()) -> String? {
        held(pendingKey, until: pendingUntilKey, now: now)
    }

    /// The key of the button whose request went to the app moments ago, for the widget to say so
    /// until the app's readings catch up.
    static func sent(now: Date = Date()) -> String? {
        held(sentKey, until: sentUntilKey, now: now)
    }

    /// When the held confirmation or the sent note lapses, for the timeline to redraw then.
    static func nextChange(after now: Date = Date()) -> Date? {
        [pendingUntilKey, sentUntilKey]
            .map { Date(timeIntervalSince1970: UserDefaults.standard.double(forKey: $0)) }
            .filter { $0 > now }
            .min()
    }

    /// The reading of the row whose button asks for its confirmation, as it was when pressed
    /// (`GlanceDocument.redeemReading(for:)`); `nil` where the request has none.
    static func pendingReading() -> String? {
        UserDefaults.standard.string(forKey: pendingReadingKey)
    }

    /// The reading of the row whose request went to the app, as it was then.
    static func sentReading() -> String? {
        UserDefaults.standard.string(forKey: sentReadingKey)
    }

    /// Where `redeem`'s button stands at `now` with its row reading `reading`: `Đang dùng…` while its
    /// request is on its way and the count has not changed, asking for its confirmation while it was
    /// pressed at this count, else ready.
    static func phase(of redeem: GlanceRedeem, reading: String, now: Date) -> GlanceRedeemPhase {
        let key = redeem.request.key
        if sent(now: now) == key, (sentReading() ?? reading) == reading { return .redeeming }
        if current(now: now) == key, (pendingReading() ?? reading) == reading { return .confirming }
        return .ready
    }

    /// The banked cards the widget buttons are acting on: the one asking for its confirmation and the
    /// one whose request went to the app, each with whether its card read as used then.
    static func bankedMarks(now: Date = Date()) -> GlanceBankedMarks {
        func mark(_ key: String?, reading: String?, until: String? = nil) -> GlanceBankedMarks.Mark? {
            let prefix = "markBankedReset:"
            guard let key, key.hasPrefix(prefix), let reading else { return nil }
            let at = until.map { Date(timeIntervalSince1970: UserDefaults.standard.double(forKey: $0)) }
            return GlanceBankedMarks.Mark(resetId: String(key.dropFirst(prefix.count)), used: reading == GlanceDocument.bankedReading(used: true), until: at)
        }
        let sent = mark(sent(now: now), reading: sentReading(), until: sentUntilKey)
        return GlanceBankedMarks(
            confirming: mark(current(now: now), reading: pendingReading()),
            sent: sent.map { [$0.resetId: $0] } ?? [:]
        )
    }

    static func hold(_ request: GlanceActionRequest, reading: String? = nil, now: Date = Date()) {
        UserDefaults.standard.set(request.key, forKey: pendingKey)
        UserDefaults.standard.set(now.addingTimeInterval(lifetime).timeIntervalSince1970, forKey: pendingUntilKey)
        UserDefaults.standard.set(reading, forKey: pendingReadingKey)
    }

    static func clear() {
        UserDefaults.standard.removeObject(forKey: pendingKey)
        UserDefaults.standard.removeObject(forKey: pendingUntilKey)
        UserDefaults.standard.removeObject(forKey: pendingReadingKey)
    }

    static func markSent(_ request: GlanceActionRequest, reading: String? = nil, now: Date = Date()) {
        UserDefaults.standard.set(request.key, forKey: sentKey)
        UserDefaults.standard.set(now.addingTimeInterval(sentLifetime).timeIntervalSince1970, forKey: sentUntilKey)
        UserDefaults.standard.set(reading, forKey: sentReadingKey)
    }

    private static func held(_ key: String, until: String, now: Date) -> String? {
        guard let value = UserDefaults.standard.string(forKey: key),
              UserDefaults.standard.double(forKey: until) > now.timeIntervalSince1970
        else { return nil }
        return value
    }
}

/// Where the widgets leave their requests for the app (`glance::take_requests` in the core), the
/// only folder the widget may write to.
enum WidgetRequests {
    static var folderURL: URL {
        GlanceStore.folderURL.appendingPathComponent("requests", isDirectory: true)
    }

    /// A button's request, under a name of its own.
    static func write(_ request: GlanceActionRequest, now: Date = Date()) throws {
        try write(payload: request.payload, name: UUID().uuidString.lowercased(), now: now)
    }

    /// Written under a dot name first and renamed after, so the app never reads half a request; a
    /// request under the same `name` the app has not read yet is replaced.
    static func write(payload: [String: Any], name: String, now: Date = Date()) throws {
        let body: [String: Any] = [
            "requestedAt": ISO8601DateFormatter().string(from: now),
            "action": payload,
        ]
        let data = try JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
        let partial = folderURL.appendingPathComponent(".\(name).json", isDirectory: false)
        let complete = folderURL.appendingPathComponent("\(name).json", isDirectory: false)
        try data.write(to: partial)
        guard rename(partial.path, complete.path) == 0 else {
            let error = POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
            try? FileManager.default.removeItem(at: partial)
            throw error
        }
    }
}

/// A widget button's press. Runs in the widget extension: it holds or clears the confirmation, or
/// leaves the request for the app, then redraws the widgets.
struct PressGlanceAction: AppIntent {
    static var title: LocalizedStringResource = "Quota Control button"
    static var openAppWhenRun: Bool = false
    static var isDiscoverable: Bool = false

    @Parameter(title: "Kind") var kind: String
    @Parameter(title: "Subject") var subject: String
    @Parameter(title: "Used", default: false) var used: Bool
    @Parameter(title: "Step") var step: String

    init() {}

    init(_ request: GlanceActionRequest, step: WidgetActionStep) {
        let parts = request.parts
        kind = parts.kind
        subject = parts.subject
        used = parts.used
        self.step = step.rawValue
    }

    func perform() async throws -> some IntentResult {
        if let request = GlanceActionRequest(kind: kind, subject: subject, used: used) {
            let reading = GlanceStore.load()?.actionReading(for: request)
            Self.apply(request, step: WidgetActionStep(rawValue: step) ?? .press, reading: reading, now: Date())
        }
        WidgetCenter.shared.reloadAllTimelines()
        return .result()
    }

    /// A confirmation counts only while it is still held for the same button, and, for a button
    /// under a row (`reading`, its count of resets), while that row reads as it did at the press;
    /// anything else puts the button back.
    static func apply(_ request: GlanceActionRequest, step: WidgetActionStep, reading: String? = nil, now: Date) {
        switch step {
        case .cancel:
            WidgetPendingAction.clear()
        case .confirm:
            let held = WidgetPendingAction.current(now: now) == request.key
                && (WidgetPendingAction.pendingReading() ?? reading) == reading
            WidgetPendingAction.clear()
            if held { send(request, reading: reading, now: now) }
        case .press:
            if request.needsConfirmation {
                WidgetPendingAction.hold(request, reading: reading, now: now)
            } else {
                WidgetPendingAction.clear()
                send(request, reading: reading, now: now)
            }
        }
    }

    private static func send(_ request: GlanceActionRequest, reading: String?, now: Date) {
        do {
            try WidgetRequests.write(request, now: now)
            WidgetPendingAction.markSent(request, reading: reading, now: now)
        } catch {
            NSLog("Quota Control widget could not leave a request for the app: \(error.localizedDescription)")
        }
    }
}

/// The popup's "Dùng 1 lượt" under a Codex account's reset credits, on a widget: the small bordered
/// button right-aligned under the row. A press asks for the confirmation (`WidgetRedeemConfirmation`,
/// held `WidgetPendingAction.lifetime`); after "Xác nhận" the button reads `Đang dùng…`, disabled,
/// until the count changes or `WidgetPendingAction.sentLifetime` passes.
struct WidgetRedeemButton: View {
    let metric: GlanceMetric
    let redeem: GlanceRedeem
    let now: Date

    var body: some View {
        let phase = WidgetPendingAction.phase(of: redeem, reading: metric.headline, now: now)
        GlanceRowAction {
            Button(intent: PressGlanceAction(redeem.request, step: .press)) {
                Text(redeem.buttonTitle(phase))
            }
            .buttonStyle(GlanceButtonStyle(tone: .bordered, small: true))
            .disabled(phase == .redeeming)
        }
    }
}

/// The Reset tab's confirmation for a pressed "Tôi đã dùng rồi", drawn over the whole widget as the
/// popup draws its dialog over the popup: the title, the words where they fit, "Hủy" and "Xác nhận"
/// in blue. A small widget leaves the words out; it never leaves "Hủy" out.
struct WidgetBankedConfirmation: View {
    let document: GlanceDocument
    let now: Date
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        if case let (request, words)? = pending {
            ZStack {
                Rectangle()
                    .fill(GlanceResetPalette(scheme: colorScheme).background.opacity(0.9))
                    .padding(-40)
                ViewThatFits(in: .vertical) {
                    card(request, words, message: words.message)
                    card(request, words, message: nil)
                    card(request, words, message: nil, compact: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// The card whose "Tôi đã dùng rồi" asks for its confirmation now, while it still reads as it did.
    private var pending: (request: GlanceActionRequest, words: GlanceBankedActions)? {
        guard let mark = WidgetPendingAction.bankedMarks(now: now).confirming, !mark.used,
              let card = document.bankedCard(resetId: mark.resetId), card.used != true,
              let words = [document.resets, document.claudeResets].compactMap({ $0?.presentation }).first(where: { $0.statuses.contains(card) })?.bankedActions
        else { return nil }
        return (.markBankedReset(resetId: mark.resetId, used: true), words)
    }

    private func card(_ request: GlanceActionRequest, _ words: GlanceBankedActions, message: String?, compact: Bool = false) -> some View {
        GlanceConfirmCard(title: words.title, message: message, compact: compact) {
            Button(intent: PressGlanceAction(request, step: .cancel)) { Text(words.cancel) }
                .buttonStyle(GlanceButtonStyle(tone: .bordered, wide: true))
            Button(intent: PressGlanceAction(request, step: .confirm)) { Text(words.confirm) }
                .buttonStyle(GlanceButtonStyle(tone: .prominent, wide: true))
        }
    }
}

/// The popup's confirmation for a pressed "Dùng 1 lượt", drawn over the whole widget as the popup
/// draws its dialog over the popup: the title, the words where they fit, "Hủy" and "Xác nhận" in red.
/// A small widget leaves the words out; it never leaves "Hủy" out.
struct WidgetRedeemConfirmation: View {
    let document: GlanceDocument
    let now: Date
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        if let redeem = pending {
            ZStack {
                Rectangle()
                    .fill(GlanceResetPalette(scheme: colorScheme).background.opacity(0.9))
                    .padding(-40)
                ViewThatFits(in: .vertical) {
                    card(redeem, message: redeem.confirmMessage(now: now, locale: document.resolvedLocale))
                    card(redeem, message: nil)
                    card(redeem, message: nil, compact: true)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// The row whose button asks for its confirmation now, at the count it was pressed at.
    private var pending: GlanceRedeem? {
        for provider in document.widget.providers {
            for metric in provider.metrics {
                if let redeem = metric.redeem, WidgetPendingAction.phase(of: redeem, reading: metric.headline, now: now) == .confirming {
                    return redeem
                }
            }
        }
        return nil
    }

    private func card(_ redeem: GlanceRedeem, message: String?, compact: Bool = false) -> some View {
        GlanceConfirmCard(title: redeem.title, message: message, compact: compact) {
            Button(intent: PressGlanceAction(redeem.request, step: .cancel)) { Text(redeem.cancel) }
                .buttonStyle(GlanceButtonStyle(tone: .bordered, wide: true))
            Button(intent: PressGlanceAction(redeem.request, step: .confirm)) { Text(redeem.confirm) }
                .buttonStyle(GlanceButtonStyle(tone: .destructive, wide: true))
        }
    }
}
