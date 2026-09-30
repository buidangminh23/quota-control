import AppIntents
import Foundation
import WidgetKit

/// The step a widget button takes: the first press, or the "Xác nhận" / "Hủy" that follows it for a
/// request that waits for a confirmation (`GlanceActionRequest.needsConfirmation`).
enum WidgetActionStep: String {
    case press
    case confirm
    case cancel
}

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

    static func hold(_ request: GlanceActionRequest, now: Date = Date()) {
        UserDefaults.standard.set(request.key, forKey: pendingKey)
        UserDefaults.standard.set(now.addingTimeInterval(lifetime).timeIntervalSince1970, forKey: pendingUntilKey)
    }

    static func clear() {
        UserDefaults.standard.removeObject(forKey: pendingKey)
        UserDefaults.standard.removeObject(forKey: pendingUntilKey)
    }

    static func markSent(_ request: GlanceActionRequest, now: Date = Date()) {
        UserDefaults.standard.set(request.key, forKey: sentKey)
        UserDefaults.standard.set(now.addingTimeInterval(sentLifetime).timeIntervalSince1970, forKey: sentUntilKey)
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

    /// Written under a dot name first and renamed after, so the app never reads half a request.
    static func write(_ request: GlanceActionRequest, now: Date = Date()) throws {
        let body: [String: Any] = [
            "requestedAt": ISO8601DateFormatter().string(from: now),
            "action": request.payload,
        ]
        let data = try JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
        let name = UUID().uuidString.lowercased()
        let partial = folderURL.appendingPathComponent(".\(name).json", isDirectory: false)
        let complete = folderURL.appendingPathComponent("\(name).json", isDirectory: false)
        try data.write(to: partial)
        do {
            try FileManager.default.moveItem(at: partial, to: complete)
        } catch {
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
            Self.apply(request, step: WidgetActionStep(rawValue: step) ?? .press, now: Date())
        }
        WidgetCenter.shared.reloadAllTimelines()
        return .result()
    }

    /// A confirmation counts only while it is still held for the same button; anything else puts
    /// the button back.
    static func apply(_ request: GlanceActionRequest, step: WidgetActionStep, now: Date) {
        switch step {
        case .cancel:
            WidgetPendingAction.clear()
        case .confirm:
            let held = WidgetPendingAction.current(now: now) == request.key
            WidgetPendingAction.clear()
            if held { send(request, now: now) }
        case .press:
            if request.needsConfirmation {
                WidgetPendingAction.hold(request, now: now)
            } else {
                WidgetPendingAction.clear()
                send(request, now: now)
            }
        }
    }

    private static func send(_ request: GlanceActionRequest, now: Date) {
        do {
            try WidgetRequests.write(request, now: now)
            WidgetPendingAction.markSent(request, now: now)
        } catch {
            NSLog("Quota Control widget could not leave a request for the app: \(error.localizedDescription)")
        }
    }
}
