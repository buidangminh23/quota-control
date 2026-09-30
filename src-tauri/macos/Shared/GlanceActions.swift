import Foundation

/// What a button on the island or a widget asks the app to do. The surface asks the user to confirm
/// first wherever the popup's own button does; the popup then checks the request and does it with
/// the code its buttons run (`src/glance/glanceActions.ts`), so a press has the same effect wherever
/// it happened.
enum GlanceActionRequest: Equatable {
    /// Spend one banked limit reset of a connected Codex account.
    case redeemLimitReset(providerId: String)
    /// Mark a Claude banked reset as used, or take the mark back.
    case markBankedReset(resetId: String, used: Bool)
    /// Show the popup on a tracker's Reset tab.
    case openResets(GlanceResetsProvider)

    /// Rebuild a request from the three plain values a widget button carries (`parts`).
    init?(kind: String, subject: String, used: Bool) {
        switch kind {
        case "redeemLimitReset":
            self = .redeemLimitReset(providerId: subject)
        case "markBankedReset":
            self = .markBankedReset(resetId: subject, used: used)
        case "openResets":
            guard let provider = GlanceResetsProvider(rawValue: subject) else { return nil }
            self = .openResets(provider)
        default:
            return nil
        }
    }

    var parts: (kind: String, subject: String, used: Bool) {
        switch self {
        case .redeemLimitReset(let providerId):
            return ("redeemLimitReset", providerId, false)
        case .markBankedReset(let resetId, let used):
            return ("markBankedReset", resetId, used)
        case .openResets(let provider):
            return ("openResets", provider.rawValue, false)
        }
    }

    /// Which button the request belongs to, so a surface can hold one confirmation at a time.
    var key: String {
        let parts = parts
        return "\(parts.kind):\(parts.subject)"
    }

    /// Spending a reset and marking one used wait for a second press on "Xác nhận", as in the popup;
    /// taking the mark back and opening a tab happen on the first press.
    var needsConfirmation: Bool {
        switch self {
        case .redeemLimitReset:
            return true
        case .markBankedReset(_, let used):
            return used
        case .openResets:
            return false
        }
    }

    /// The request as the popup reads it.
    var payload: [String: Any] {
        switch self {
        case .redeemLimitReset(let providerId):
            return ["kind": "redeemLimitReset", "providerId": providerId]
        case .markBankedReset(let resetId, let used):
            return ["kind": "markBankedReset", "resetId": resetId, "used": used]
        case .openResets(let provider):
            return ["kind": "openResets", "provider": provider.rawValue]
        }
    }

    var json: Data? {
        try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys])
    }
}
