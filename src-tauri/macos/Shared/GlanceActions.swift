import Foundation

/// A press on a button of the island or a widget: the first one, or the "Xác nhận" / "Hủy" that
/// follows it for a request that waits for a confirmation (`GlanceActionRequest.needsConfirmation`).
enum GlanceActionStep: String {
    case press
    case confirm
    case cancel
}

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

/// The words of a Claude banked card's buttons and of the confirmation "Tôi đã dùng rồi" asks for
/// (`GlanceBankedActions` in `src/model/glance.ts`), as the Reset tab's `BankedCards` words them.
struct GlanceBankedActions: Decodable, Equatable {
    /// `Tôi đã dùng rồi`.
    var markUsed: String
    /// The confirmation's title.
    var title: String
    /// The confirmation's words.
    var message: String
    /// `Xác nhận`, filled with the accent color.
    var confirm: String
    /// `Hủy`.
    var cancel: String
    /// What a card marked as used folds to.
    var used: String
    /// `Hoàn tác`, which takes the mark off.
    var undo: String
}

/// Where a banked card's buttons stand: ready, asking "Xác nhận" or "Hủy", or sent, drawn as the
/// user asked (folded after "Xác nhận", open again after "Hoàn tác") until the document says so.
enum GlanceBankedPhase: Equatable {
    case ready
    case confirming
    case sent
}

/// The banked cards a surface is acting on (Claude): the one asking for its confirmation, and those
/// whose request went to the app, each with whether its card read as used when it was pressed. A
/// mark only counts while the card still reads that way: once the document catches up, or the card
/// goes, the card is drawn as the document says.
struct GlanceBankedMarks: Equatable {
    struct Mark: Equatable {
        var resetId: String
        var used: Bool
        var until: Date? = nil
    }

    static let sentLifetime: TimeInterval = 30

    var confirming: Mark?
    var sent: [String: Mark] = [:]

    /// Where the buttons of the card about `resetId`, now reading `used`, stand at `now`.
    func phase(resetId: String, used: Bool, now: Date) -> GlanceBankedPhase {
        if let mark = sent[resetId], mark.used == used, (mark.until ?? .distantFuture) > now { return .sent }
        if let confirming, confirming.resetId == resetId, confirming.used == used { return .confirming }
        return .ready
    }

    /// The marks still standing for `document`: a mark goes once its card reads otherwise, is gone,
    /// or its time ran out.
    func pruned(for document: GlanceDocument?, now: Date) -> GlanceBankedMarks {
        let reads = { (mark: Mark) in document?.bankedCard(resetId: mark.resetId).map { ($0.used == true) == mark.used } ?? false }
        return GlanceBankedMarks(
            confirming: confirming.flatMap { reads($0) ? $0 : nil },
            sent: sent.filter { reads($0.value) && ($0.value.until ?? .distantFuture) > now }
        )
    }

    /// The marks without the confirmation, for a surface that draws it apart from the card.
    var withoutConfirmation: GlanceBankedMarks {
        GlanceBankedMarks(confirming: nil, sent: sent)
    }
}

extension GlanceDocument {
    /// The Claude banked card about `resetId`, in whichever tracker the document carries it.
    func bankedCard(resetId: String) -> GlanceResetStatusCard? {
        [resets, claudeResets]
            .compactMap { $0?.presentation?.statuses }
            .joined()
            .first { $0.kind == "banked" && $0.resetId == resetId }
    }

    /// What a press on `request`'s button is checked against when its confirmation or its request
    /// lands: the count of resets on a Codex row, whether a banked card reads as used; `nil` where the
    /// button is gone.
    func actionReading(for request: GlanceActionRequest) -> String? {
        switch request {
        case .redeemLimitReset:
            return redeemReading(for: request)
        case let .markBankedReset(resetId, _):
            return bankedCard(resetId: resetId).map { Self.bankedReading(used: $0.used == true) }
        case .openResets:
            return nil
        }
    }

    /// A banked card's state as `actionReading(for:)` words it.
    static func bankedReading(used: Bool) -> String {
        used ? "used" : "open"
    }

    /// The row carrying `providerId`'s redemption button, on the island or the widget: an account
    /// either lists without its reset credits (the island's starred metrics, say) is passed over.
    func redeemRow(providerId: String) -> GlanceMetric? {
        (providers + widget.providers)
            .lazy
            .filter { $0.id == providerId }
            .compactMap { $0.metrics.first { $0.redeem?.providerId == providerId } }
            .first
    }

    /// The reading of the row carrying `request`'s button (its count of resets), to tell when the
    /// app's readings have caught up with a press; `nil` for any other request.
    func redeemReading(for request: GlanceActionRequest) -> String? {
        guard case let .redeemLimitReset(providerId) = request else { return nil }
        return redeemRow(providerId: providerId)?.headline
    }
}
