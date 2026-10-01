import AppIntents
import Foundation
import WidgetKit

/// Whose reset tracker one Reset, Reset Calendar or Overview widget shows, as its own Edit Widget
/// sets it: as Settings say for the widgets (the default, which widgets placed before the choice
/// keep), Codex's, Claude's, or both at once. Each choice is worded in the app's language, as the
/// widgets' names in the gallery are.
struct WidgetResetsChoice: AppEntity {
    static var typeDisplayRepresentation = TypeDisplayRepresentation(name: "Reset")
    static var defaultQuery = WidgetResetsChoiceQuery()

    /// `app` for as Settings say, else a tracker (`GlanceResetsProvider`).
    let id: String

    static let app = WidgetResetsChoice(id: "app")
    static let all = ["app", "codex", "claude", "both"].map(WidgetResetsChoice.init(id:))

    /// The tracker chosen, or `nil` for as Settings say.
    var provider: GlanceResetsProvider? { GlanceResetsProvider(rawValue: id) }

    var displayRepresentation: DisplayRepresentation {
        DisplayRepresentation(title: "\(WidgetText.resetsChoice(provider))")
    }
}

struct WidgetResetsChoiceQuery: EntityQuery {
    func entities(for identifiers: [WidgetResetsChoice.ID]) async throws -> [WidgetResetsChoice] {
        WidgetResetsChoice.all.filter { identifiers.contains($0.id) }
    }

    func suggestedEntities() async throws -> [WidgetResetsChoice] {
        WidgetResetsChoice.all
    }

    func defaultResult() async -> WidgetResetsChoice? {
        .app
    }
}

/// The Edit Widget of the widgets that draw a reset tracker: which tracker this one widget shows.
struct ResetWidgetConfiguration: WidgetConfigurationIntent {
    static var title: LocalizedStringResource = "Reset"

    @Parameter(title: "Reset")
    var tracker: WidgetResetsChoice?

    init() {}

    /// The tracker this widget shows, or `nil` while it shows the one Settings chose for the widgets.
    var provider: GlanceResetsProvider? { tracker?.provider }
}

/// A widget whose Edit Widget chose a tracker the document does not carry asks the app for it
/// (`showResets`): the Claude tracker, while no Settings choice reads it. Until the app answers, the
/// widget says the tracker is on its way and looks again every `retry`; the app's answer is the
/// tracker's name in the document, then the tracker itself, or the line saying it is off.
enum WidgetResetsAsk {
    static let retry: TimeInterval = 60

    /// Whether the app has not yet heard that a widget showing `choice` needs the Claude tracker.
    static func awaits(_ document: GlanceDocument, choice: GlanceResetsProvider?) -> Bool {
        guard choice == .claude || choice == .both else { return false }
        return document.labels.claudeResetsTab == nil && document.claudeResets == nil && document.claudeResetsPending == nil
    }

    /// `document` as a widget showing `choice` draws it: when it still awaits the Claude tracker, the
    /// request is left for the app and the tracker reads as on its way.
    static func answering(_ document: GlanceDocument, choice: GlanceResetsProvider?) -> GlanceDocument {
        guard awaits(document, choice: choice) else { return document }
        try? WidgetRequests.write(payload: ["kind": "showResets", "provider": GlanceResetsProvider.claude.rawValue], name: "show-resets-claude")
        var copy = document
        copy.claudeResetsPending = GlanceResetsPending(text: WidgetText.claudeOnItsWay(document), failed: nil)
        return copy
    }
}

extension GlanceDocument {
    /// The document as a widget draws it whose own Edit Widget chose `choice`; `nil` follows the
    /// choice Settings made for the widgets.
    func forWidget(_ choice: GlanceResetsProvider?) -> GlanceDocument {
        guard let choice else { return forWidget }
        var copy = self
        copy.widget.resetsProvider = choice
        return copy.forWidget
    }
}
