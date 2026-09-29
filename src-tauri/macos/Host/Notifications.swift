import AppKit
import UserNotifications

/// Hears the answer to a question about notification access, with the context the question came
/// with: see `NotificationAccess`.
public typealias QCNotificationAccessHandler = @convention(c) (UnsafeMutableRawPointer?, Int32) -> Void

/// Hears that a notification, or the button on it, was clicked.
public typealias QCNotificationOpenHandler = @convention(c) () -> Void

enum NotificationAccess: Int32 {
    /// The system's notification center is out of reach (a development run outside an app bundle).
    case unavailable = -1
    /// The user was never asked.
    case undetermined = 0
    case denied = 1
    case granted = 2
}

/// Notifications through the system's notification center: they carry a button that opens the
/// popup, a click on them opens it too, a newer alert about the same limit replaces the older one,
/// and Notification Center groups them by account.
final class SystemNotifications: NSObject, UNUserNotificationCenterDelegate {
    static let shared = SystemNotifications()

    private static let openAction = "qc.open"

    private enum Category: String, CaseIterable {
        case english = "qc.alert.en"
        case vietnamese = "qc.alert.vi"

        var category: UNNotificationCategory {
            let title = self == .english ? "Open Quota Control" : "Mở Quota Control"
            let open = UNNotificationAction(identifier: SystemNotifications.openAction, title: title, options: [.foreground])
            return UNNotificationCategory(identifier: rawValue, actions: [open], intentIdentifiers: [], options: [])
        }
    }

    private var onOpen: QCNotificationOpenHandler?

    /// The notification center belongs to an app bundle, and asking for it without one raises an
    /// exception.
    static var available: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    func start(handler: QCNotificationOpenHandler?) -> Bool {
        guard Self.available else { return false }
        onOpen = handler
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories(Set(Category.allCases.map(\.category)))
        return true
    }

    func access(_ done: @escaping (NotificationAccess) -> Void) {
        guard Self.available else { return done(.unavailable) }
        UNUserNotificationCenter.current().getNotificationSettings { settings in
            switch settings.authorizationStatus {
            case .notDetermined: done(.undetermined)
            case .denied: done(.denied)
            default: done(.granted)
            }
        }
    }

    /// Ask the user, when they were never asked. The system asks only once: after a refusal its
    /// settings are the one place the answer changes, so they open instead.
    func request(_ done: @escaping (NotificationAccess) -> Void) {
        access { current in
            switch current {
            case .undetermined:
                UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, error in
                    if let error { NSLog("Quota Control: asking for notifications failed: %@", error.localizedDescription) }
                    self.access(done)
                }
            case .denied:
                Self.openSettings()
                done(.denied)
            default:
                done(current)
            }
        }
    }

    func send(title: String, body: String, identifier: String, thread: String, english: Bool) {
        guard Self.available else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.threadIdentifier = thread
        content.categoryIdentifier = (english ? Category.english : Category.vietnamese).rawValue
        let name = identifier.isEmpty ? UUID().uuidString : identifier
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: name, content: content, trigger: nil)) { error in
            if let error { NSLog("Quota Control: a notification was not delivered: %@", error.localizedDescription) }
        }
    }

    private static func openSettings() {
        guard let bundle = Bundle.main.bundleIdentifier,
              let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=\(bundle)")
        else { return }
        DispatchQueue.main.async { NSWorkspace.shared.open(url) }
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .list])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        if response.actionIdentifier != UNNotificationDismissActionIdentifier {
            DispatchQueue.main.async { self.onOpen?() }
        }
        completionHandler()
    }
}

/// Take over the app's notifications; `false` when the system's notification center is out of
/// reach and the caller keeps sending them its own way.
@_cdecl("qc_notifications_start")
public func qcNotificationsStart(_ handler: QCNotificationOpenHandler?) -> Bool {
    SystemNotifications.shared.start(handler: handler)
}

@_cdecl("qc_notifications_access")
public func qcNotificationsAccess(_ context: UnsafeMutableRawPointer?, _ handler: QCNotificationAccessHandler?) {
    SystemNotifications.shared.access { handler?(context, $0.rawValue) }
}

@_cdecl("qc_notifications_request")
public func qcNotificationsRequest(_ context: UnsafeMutableRawPointer?, _ handler: QCNotificationAccessHandler?) {
    SystemNotifications.shared.request { handler?(context, $0.rawValue) }
}

@_cdecl("qc_notifications_send")
public func qcNotificationsSend(
    _ title: UnsafePointer<CChar>?,
    _ body: UnsafePointer<CChar>?,
    _ identifier: UnsafePointer<CChar>?,
    _ thread: UnsafePointer<CChar>?,
    _ english: Bool
) {
    let text = { (pointer: UnsafePointer<CChar>?) in pointer.map { String(cString: $0) } ?? "" }
    SystemNotifications.shared.send(title: text(title), body: text(body), identifier: text(identifier), thread: text(thread), english: english)
}
