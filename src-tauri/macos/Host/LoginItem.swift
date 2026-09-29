import Foundation
import ServiceManagement

/// Launch at login as a login item of the app itself: System Settings lists it under Login Items
/// by the app's name and icon, where it can be switched off, and it goes away with the app.
enum LoginItem {
    enum State: Int32 {
        /// Not an app bundle (a development run): the caller keeps its own launch agent.
        case unavailable = -1
        /// Not registered, or the system refused to register it.
        case off = 0
        case on = 1
        /// Registered, but switched off in System Settings, where only the user switches it back.
        case needsApproval = 2
    }

    static var available: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    static var state: State {
        guard available else { return .unavailable }
        switch SMAppService.mainApp.status {
        case .enabled: return .on
        case .requiresApproval: return .needsApproval
        default: return .off
        }
    }

    /// Register or remove the login item and say where it stands after; a failure leaves it as
    /// it was, and the answer tells so. An item that waits for approval is registered already:
    /// only the user switches it on, in System Settings, which opens at Login Items when the user
    /// asked (`byUser`).
    static func set(_ enabled: Bool, byUser: Bool) -> State {
        guard available else { return .unavailable }
        let service = SMAppService.mainApp
        do {
            switch (enabled, service.status) {
            case (true, .enabled), (true, .requiresApproval): break
            case (true, _): try service.register()
            case (false, .enabled), (false, .requiresApproval): try service.unregister()
            case (false, _): break
            }
        } catch {
            NSLog("Quota Control: the login item did not change: %@", error.localizedDescription)
        }
        if enabled, byUser, service.status == .requiresApproval {
            SMAppService.openSystemSettingsLoginItems()
        }
        return state
    }
}

@_cdecl("qc_login_item_state")
public func qcLoginItemState() -> Int32 {
    LoginItem.state.rawValue
}

@_cdecl("qc_login_item_set")
public func qcLoginItemSet(_ enabled: Bool, _ byUser: Bool) -> Int32 {
    LoginItem.set(enabled, byUser: byUser).rawValue
}
