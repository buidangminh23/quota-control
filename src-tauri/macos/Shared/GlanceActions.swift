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

/// The popup's "Dùng 1 lượt" under a connected Codex account's reset credits (`GlanceRedeem` in
/// `src/model/glance.ts`): the account, the button's words and its confirmation's, as the popup
/// builds them.
struct GlanceRedeem: Decodable, Equatable {
    var providerId: String
    /// `Dùng 1 lượt`.
    var redeem: String
    /// `Đang dùng…`, while the reset is being spent.
    var redeeming: String
    /// The confirmation's title.
    var title: String
    /// The confirmation's words without an expiry.
    var message: String
    /// The confirmation's words with `{at}` where the soonest credit's expiry goes.
    var messageAt: String?
    /// When the soonest credit still ahead expires.
    var expiresAt: Date?
    /// How `{at}` words `expiresAt` at the moment the confirmation is asked.
    var expiry: GlanceDayWords?
    /// `Xác nhận`.
    var confirm: String
    /// `Hủy`.
    var cancel: String

    var request: GlanceActionRequest { .redeemLimitReset(providerId: providerId) }

    /// The words the button reads in `phase`: `Dùng 1 lượt`, or `Đang dùng…` while the reset it asked
    /// for is being spent.
    func buttonTitle(_ phase: GlanceRedeemPhase) -> String {
        phase == .redeeming ? redeeming : redeem
    }

    /// The confirmation's words as the popup builds them when it is asked at `now`: naming the
    /// soonest credit's expiry while it is ahead.
    func confirmMessage(now: Date, locale: Locale, calendar: Calendar = .current) -> String {
        guard let messageAt, let expiresAt, expiresAt > now, let expiry else { return message }
        return messageAt.replacingOccurrences(
            of: GlanceResetRow.momentPlaceholder,
            with: expiry.label(expiresAt, now: now, locale: locale, calendar: calendar)
        )
    }
}

/// Where a redemption button stands: ready, asking "Xác nhận" or "Hủy" in its place, or saying
/// `Đang dùng…` while the request it sent has not changed the account's count yet.
enum GlanceRedeemPhase: Equatable {
    case ready
    case confirming
    case redeeming
}

extension GlanceDocument {
    /// The row carrying `providerId`'s redemption button, on the island or the widget.
    func redeemRow(providerId: String) -> GlanceMetric? {
        (providers + widget.providers)
            .first { $0.id == providerId }?
            .metrics.first { $0.redeem?.providerId == providerId }
    }

    /// The reading of the row carrying `request`'s button (its count of resets), to tell when the
    /// app's readings have caught up with a press; `nil` for any other request.
    func redeemReading(for request: GlanceActionRequest) -> String? {
        guard case let .redeemLimitReset(providerId) = request else { return nil }
        return redeemRow(providerId: providerId)?.headline
    }
}
