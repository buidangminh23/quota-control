import AppKit
import SwiftUI

/// The Dynamic Island: a black shape around the MacBook notch whose two wings carry the readings
/// picked in Settings, as a percentage, a ring or a bar. Hovering it (or, when Settings say so, a
/// click) opens a detail view with the sections its Settings list (quota limits, the Codex or
/// Claude reset tracker, the next limits to come back), a new alert (a limit close to running out,
/// or one that came back) opens it for a few seconds. A screen without a notch gets the same island
/// as a pill in the middle of the menu bar.
/// It lives in a non-activating panel, so it never takes focus from the app in front.
@MainActor
final class IslandController {
    static let shared = IslandController()

    private let model = IslandModel()
    private var panel: IslandPanel?
    private var hosting: IslandHostingView<IslandRootView>?
    private var handler: QCIslandHandler?
    private var observers: [NSObjectProtocol] = []
    private var clock: Timer?
    private var pendingExpand: DispatchWorkItem?
    private var pendingCollapse: DispatchWorkItem?
    private var pendingShrink: DispatchWorkItem?
    private var alertTimer: DispatchWorkItem?
    private var seenAlerts: Set<String> = []
    private var hovering = false
    /// While the popup is open under the island, the island stays closed so it never covers it.
    private var popupVisible = false

    private static let hoverDelay: TimeInterval = 0.12
    private static let leaveDelay: TimeInterval = 0.25
    private static let alertDuration: TimeInterval = 5
    private static let settleDelay: TimeInterval = 0.45
    /// How often the countdowns in the wings and the open island are measured again, so a wing
    /// grows with its words and a limit that came back leaves the list.
    private static let tickInterval: TimeInterval = 30

    func start(handler: QCIslandHandler?) {
        self.handler = handler
        guard observers.isEmpty else { return }
        let center = NotificationCenter.default
        observers.append(center.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { _ in MainActor.assumeIsolated { IslandController.shared.relayout() } })
        observers.append(NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.activeSpaceDidChangeNotification, object: nil, queue: .main
        ) { _ in MainActor.assumeIsolated { IslandController.shared.relayout() } })
        clock = Timer.scheduledTimer(withTimeInterval: Self.tickInterval, repeats: true) { _ in
            MainActor.assumeIsolated { IslandController.shared.relayout(animated: true) }
        }
    }

    func update(_ data: Data) {
        guard let decoded = GlanceDocument.decode(data) else { return }
        let document = decoded.forIsland
        model.trackers = (decoded.resets, decoded.claudeResets)
        model.document = document
        relayout(animated: true)
        if let alert = document.alert, seenAlerts.insert(alert.id).inserted, document.island.enabled, document.island.alerts {
            showAlert(alert)
        }
    }

    func collapse() {
        cancelPending()
        guard model.mode != .compact else { return }
        setMode(.compact)
    }

    func setPopupVisible(_ visible: Bool) {
        popupVisible = visible
        if visible {
            collapse()
        }
    }

    private func cancelPending() {
        pendingExpand?.cancel()
        pendingCollapse?.cancel()
        alertTimer?.cancel()
    }

    private func cancelShrink() {
        pendingShrink?.cancel()
        pendingShrink = nil
    }

    // MARK: Layout

    private var shouldShow: Bool {
        guard let document = model.document, document.island.enabled else { return false }
        return !model.slots.isEmpty || IslandPlan.hasContent(document, now: Date())
    }

    /// Places the panel for the current screen and content. Every measurement follows the geometry,
    /// so an update, a screen change and the clock all measure again here. While the island is
    /// closing the panel keeps its large frame until the shrink settles, so the animation is never
    /// cut short.
    private func relayout(animated: Bool = false) {
        guard shouldShow, let geometry = IslandGeometry.current() else {
            cancelPending()
            cancelShrink()
            hovering = false
            model.mode = .compact
            panel?.orderOut(nil)
            return
        }
        model.geometry = geometry
        let now = Date()
        if animated && model.mode.isOpen {
            withAnimation(Self.spring) { model.measure(now: now) }
        } else {
            model.measure(now: now)
        }
        let panel = ensurePanel()
        let kind: FrameKind = model.mode == .compact && pendingShrink == nil ? .compact : .canvas
        panel.setFrame(frame(for: kind, geometry: geometry), display: true)
        panel.orderFrontRegardless()
    }

    private enum FrameKind {
        case compact
        case canvas
    }

    private static let spring = Animation.spring(response: 0.38, dampingFraction: 0.82)

    private func frame(for kind: FrameKind, geometry: IslandGeometry) -> NSRect {
        let compact = model.compactSize(for: geometry)
        let size: CGSize
        switch kind {
        case .compact:
            size = compact
        case .canvas:
            size = CGSize(
                width: max(compact.width, model.expandedSize.width) + IslandGeometry.shadowMargin * 2,
                height: max(compact.height, model.expandedSize.height) + IslandGeometry.shadowMargin
            )
        }
        let screen = geometry.screenFrame
        let top = geometry.hasNotch ? screen.maxY : screen.maxY - geometry.pillInset
        return NSRect(x: (screen.midX - size.width / 2).rounded(), y: top - size.height, width: size.width, height: size.height)
    }

    private func ensurePanel() -> IslandPanel {
        if let panel { return panel }
        let panel = IslandPanel()
        let hosting = IslandHostingView(rootView: IslandRootView(model: model))
        hosting.onHover = { [weak self] inside in self?.hover(inside) }
        hosting.onClick = { [weak self] point in self?.click(at: point) }
        panel.contentView = hosting
        self.panel = panel
        self.hosting = hosting
        return panel
    }

    // MARK: Interaction

    private var expandsOnHover: Bool { model.document?.island.expandOnHover ?? true }

    /// Hovering opens the details; leaving closes whatever is open, the details or an alert (an
    /// alert whose time ran out while the pointer rested on it closes here too).
    private func hover(_ inside: Bool) {
        hovering = inside
        pendingExpand?.cancel()
        pendingCollapse?.cancel()
        if inside {
            guard model.mode != .expanded, expandsOnHover else { return }
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated { self?.expand() }
            }
            pendingExpand = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.hoverDelay, execute: work)
        } else if model.mode.isOpen {
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, !self.hovering else { return }
                    self.alertTimer?.cancel()
                    self.setMode(.compact)
                }
            }
            pendingCollapse = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.leaveDelay, execute: work)
        }
    }

    private func click(at point: CGPoint) {
        if model.mode == .expanded {
            if let tab = model.tab(at: point) {
                select(tab)
            } else if model.footerFrame.contains(point) {
                open()
            }
            return
        }
        if model.mode == .compact && !expandsOnHover {
            pendingCollapse?.cancel()
            expand()
            return
        }
        open()
    }

    private func select(_ tab: GlanceView) {
        guard model.selectedTab != tab || model.plan(now: Date())?.selected != tab else { return }
        pendingCollapse?.cancel()
        withAnimation(Self.spring) {
            model.selectedTab = tab
        }
        relayout(animated: true)
    }

    private func expand() {
        guard !popupVisible, let geometry = currentGeometry() else { return }
        alertTimer?.cancel()
        model.geometry = geometry
        model.measureExpanded(now: Date())
        setMode(.expanded)
    }

    private func showAlert(_ alert: GlanceAlert) {
        guard !hovering, !popupVisible, let geometry = currentGeometry() else { return }
        alertTimer?.cancel()
        model.geometry = geometry
        model.measureAlert(alert)
        setMode(.alert(alert))
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self, !self.hovering, case .alert = self.model.mode else { return }
                self.setMode(.compact)
            }
        }
        alertTimer = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.alertDuration, execute: work)
    }

    private func currentGeometry() -> IslandGeometry? {
        model.geometry ?? IslandGeometry.current()
    }

    private func setMode(_ mode: IslandMode) {
        guard let geometry = currentGeometry(), shouldShow else { return }
        cancelShrink()
        if mode != .compact {
            panel?.setFrame(frame(for: .canvas, geometry: geometry), display: true)
        }
        withAnimation(Self.spring) {
            model.mode = mode
        }
        guard mode == .compact else { return }
        let work = DispatchWorkItem { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.pendingShrink = nil
                guard self.model.mode == .compact, let panel = self.panel, let geometry = self.model.geometry else { return }
                panel.setFrame(self.frame(for: .compact, geometry: geometry), display: true)
            }
        }
        pendingShrink = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.settleDelay, execute: work)
    }

    private func open() {
        guard panel != nil, let geometry = model.geometry else { return }
        cancelPending()
        let primaryHeight = NSScreen.screens.first?.frame.maxY ?? geometry.screenFrame.maxY
        let anchor = frame(for: .compact, geometry: geometry)
        setMode(.compact)
        handler?(
            IslandEvent.open.rawValue,
            Double(anchor.minX),
            Double(primaryHeight - anchor.maxY),
            Double(anchor.width),
            Double(anchor.height),
            Double(geometry.scale)
        )
    }
}

enum IslandMode: Equatable {
    case compact
    case expanded
    case alert(GlanceAlert)

    var isOpen: Bool { self != .compact }
}

/// Where the island sits: the built-in screen's notch, or the menu bar of the main screen.
struct IslandGeometry: Equatable {
    var screenFrame: CGRect
    var hasNotch: Bool
    var notchWidth: CGFloat
    var barHeight: CGFloat
    var scale: CGFloat
    /// The tallest the open island may grow: from the top of the screen down to just above the Dock.
    var maxOpenHeight: CGFloat = 800

    /// A wing beside the notch is never narrower than this, nor wider than `maxWing`.
    static let minWing: CGFloat = 64
    static let maxWing: CGFloat = 124
    /// A notch with no reading beside it keeps only a sliver either side to hover.
    static let emptyWing: CGFloat = 12
    /// Room between a wing's reading and the island's outer edge (its shoulder included), and
    /// between the reading and the notch.
    static let wingOuterPadding: CGFloat = 14
    static let wingInnerGap: CGFloat = 8
    static let pillPadding: CGFloat = 11
    static let pillGap: CGFloat = 14
    static let expandedWidth: CGFloat = 380
    static let wideExpandedWidth: CGFloat = 720

    static func expandedWidth(for document: GlanceDocument, plan: IslandPlan, budget: IslandBudget) -> CGFloat {
        let accounts = min(document.visibleProviders.count, budget.maxAccounts ?? .max)
        return !plan.sections.contains(.resets) && plan.sections.contains(.quota) && accounts > 1 ? wideExpandedWidth : expandedWidth
    }
    static let pillHeight: CGFloat = 22
    /// Room around the open island for its shadow.
    static let shadowMargin: CGFloat = 16
    /// Space kept free below the open island, above the Dock or the screen's edge.
    static let bottomMargin: CGFloat = 24

    var pillInset: CGFloat { max(0, (barHeight - Self.pillHeight) / 2) }

    /// Space above the details: the notch itself, or a little air under the top of the pill.
    var detailsInset: CGFloat { hasNotch ? barHeight : 8 }

    /// The width of each wing beside the notch for readings `content` points wide.
    func wing(content: CGFloat, empty: Bool) -> CGFloat {
        if empty { return Self.emptyWing }
        let needed = ceil(content) + Self.wingOuterPadding + Self.wingInnerGap
        return min(max(needed, Self.minWing), Self.maxWing)
    }

    /// The closed island: the notch with a wing either side, or a pill holding `pieces` readings
    /// of `content` points each.
    func compactSize(content: CGFloat, pieces: Int) -> CGSize {
        if hasNotch {
            let wing = wing(content: content, empty: pieces == 0)
            return CGSize(width: notchWidth + wing * 2, height: barHeight)
        }
        let reading = min(ceil(content), Self.maxWing)
        let inner = pieces == 0 ? Self.pillHeight : reading * CGFloat(pieces) + Self.pillGap * CGFloat(max(pieces - 1, 0))
        return CGSize(width: max(inner + Self.pillPadding * 2, Self.pillHeight * 2), height: Self.pillHeight)
    }

    /// The notched screen when there is one, else the screen with the menu bar.
    static func current() -> IslandGeometry? {
        let screens = NSScreen.screens
        if let notched = screens.first(where: { $0.auxiliaryTopLeftArea != nil && $0.safeAreaInsets.top > 0 }),
           let left = notched.auxiliaryTopLeftArea,
           let right = notched.auxiliaryTopRightArea {
            let width = notched.frame.width - left.width - right.width
            return IslandGeometry(
                screenFrame: notched.frame,
                hasNotch: true,
                notchWidth: max(width, 120),
                barHeight: notched.safeAreaInsets.top,
                scale: notched.backingScaleFactor,
                maxOpenHeight: openHeight(on: notched)
            )
        }
        guard let screen = screens.first else { return nil }
        let menuBar = screen.frame.maxY - screen.visibleFrame.maxY
        return IslandGeometry(
            screenFrame: screen.frame,
            hasNotch: false,
            notchWidth: 0,
            barHeight: menuBar > 0 ? menuBar : 24,
            scale: screen.backingScaleFactor,
            maxOpenHeight: openHeight(on: screen)
        )
    }

    private static func openHeight(on screen: NSScreen) -> CGFloat {
        let available = screen.frame.maxY - screen.visibleFrame.minY
        return max(available - bottomMargin - shadowMargin, 200)
    }
}

/// What a wing shows: a provider's mark and one reading.
struct IslandSlot: Equatable, Identifiable {
    var id: String
    var provider: GlanceProvider
    var metric: GlanceMetric
}

/// How the closed island spreads its readings: none (the notch alone), one reading (its mark on
/// the left of the notch and its value on the right, or whole inside the pill), or two readings,
/// one per wing.
enum IslandWingLayout: Equatable {
    case empty
    case single(IslandSlot)
    case pair(IslandSlot, IslandSlot)

    init(_ slots: [IslandSlot]) {
        switch slots.count {
        case 0: self = .empty
        case 1: self = .single(slots[0])
        default: self = .pair(slots[0], slots[1])
        }
    }

    /// The pieces drawn side by side, left to right: split across the notch, or whole in a pill.
    func pieces(hasNotch: Bool) -> [IslandWingPiece.Part] {
        switch self {
        case .empty: return []
        case .single: return hasNotch ? [.lead, .trail] : [.whole]
        case .pair: return [.whole, .whole]
        }
    }

    func slot(at index: Int) -> IslandSlot? {
        switch self {
        case .empty: return nil
        case let .single(slot): return slot
        case let .pair(first, second): return index == 0 ? first : second
        }
    }
}

@MainActor
final class IslandModel: ObservableObject {
    @Published var document: GlanceDocument?
    /// The document's Codex and Claude reset trackers as sent, before the island picked one, so an
    /// alert finds its brand's mark whichever tracker the island shows.
    var trackers: (codex: GlanceResets?, claude: GlanceResets?) = (nil, nil)
    @Published var mode: IslandMode = .compact
    @Published var geometry: IslandGeometry?
    @Published var expandedSize = CGSize(width: IslandGeometry.expandedWidth, height: 120)
    @Published var budget = IslandBudget.full
    /// The widest reading beside the notch, which both wings take so the notch stays centered.
    @Published var wingContent: CGFloat = 0
    /// The tab last clicked on the open island; it stays picked while the island closes and opens.
    @Published var selectedTab: GlanceView?
    /// Where the open island's tabs sit, in the panel's top-left coordinates.
    var tabFrames: [GlanceView: CGRect] = [:]
    var footerFrame: CGRect = .zero

    func plan(now: Date) -> IslandPlan? {
        guard let document else { return nil }
        return IslandPlan.make(document, now: now, selected: selectedTab)
    }

    /// The tab under `point`, when the open island shows a tab bar.
    func tab(at point: CGPoint) -> GlanceView? {
        guard let plan = plan(now: Date()), !plan.tabs.isEmpty else { return nil }
        return plan.tabs.first { tabFrames[$0]?.insetBy(dx: -2, dy: -3).contains(point) == true }
    }

    /// The left and right wings, as the popup chose them: a picked metric, or the next reading of
    /// the island's accounts.
    var slots: [IslandSlot] {
        guard let document else { return [] }
        return document.island.wings.prefix(2).compactMap { provider in
            provider.metrics.first.map { IslandSlot(id: "\(provider.id)|\($0.id)", provider: provider, metric: $0) }
        }
    }

    var wingLayout: IslandWingLayout { IslandWingLayout(slots) }

    func compactSize(for geometry: IslandGeometry) -> CGSize {
        let pieces = wingLayout.pieces(hasNotch: geometry.hasNotch).count
        return geometry.compactSize(content: wingContent, pieces: pieces)
    }

    /// Measures the wings and whatever is open, for the current geometry.
    func measure(now: Date) {
        measureWings(now: now)
        switch mode {
        case .compact:
            break
        case .expanded:
            measureExpanded(now: now)
        case let .alert(alert):
            measureAlert(alert)
        }
    }

    func measureWings(now: Date) {
        guard let document, let geometry else { return }
        let layout = wingLayout
        let widths = layout.pieces(hasNotch: geometry.hasNotch).enumerated().compactMap { index, part -> CGFloat? in
            guard let slot = layout.slot(at: index) else { return nil }
            let view = IslandWingPiece(slot: slot, part: part, style: document.island.style, units: document.labels.units, now: now)
            return Self.size(of: view.fixedSize(), proposing: CGSize(width: 1000, height: 100)).width
        }
        let widest = ceil(widths.max() ?? 0)
        if widest != wingContent {
            wingContent = widest
        }
    }

    func measureExpanded(now: Date) {
        guard let document, let geometry else { return }
        let compact = compactSize(for: geometry).width
        let plan = IslandPlan.make(document, now: now, selected: selectedTab)
        let preferred = max(IslandGeometry.expandedWidth(for: document, plan: plan, budget: .full), compact)
        let width = min(preferred, max(1, geometry.screenFrame.width - IslandGeometry.shadowMargin * 2))
        let view = IslandDetails(
            document: document, now: now, topInset: geometry.detailsInset,
            budget: .full, selected: selectedTab, availableWidth: width
        )
        .frame(width: width)
        .fixedSize(horizontal: false, vertical: true)
        let height = Self.size(of: view, proposing: CGSize(width: width, height: 100_000)).height
        if budget != .full { budget = .full }
        expandedSize = CGSize(width: width, height: min(ceil(height), geometry.maxOpenHeight))
    }

    func measureAlert(_ alert: GlanceAlert) {
        guard let geometry else { return }
        let width = max(IslandGeometry.expandedWidth, compactSize(for: geometry).width)
        let look = look(for: alert)
        let view = IslandAlertView(alert: alert, mark: look.mark, tint: look.tint, topInset: geometry.detailsInset)
            .frame(width: width)
        let height = Self.size(of: view, proposing: CGSize(width: width, height: 2000)).height
        expandedSize = CGSize(width: width, height: min(ceil(height), geometry.maxOpenHeight))
    }

    /// The mark beside an alert: an island account of the alert's brand, else the reset tracker of
    /// that brand (the Codex tracker for any Codex alert), else a plain dot.
    func look(for alert: GlanceAlert) -> (mark: GlanceMark?, tint: Color) {
        guard let brand = alert.brand, let document else { return (nil, .white) }
        if let provider = document.providers.first(where: { $0.brand == brand }) {
            return (provider.mark, provider.tint)
        }
        if let resets = trackers.codex, resets.brand == brand || brand == "codex" {
            return (resets.mark, resets.tint)
        }
        if let resets = trackers.claude, resets.brand == brand {
            return (resets.mark, resets.tint)
        }
        return (nil, .white)
    }

    static func size<V: View>(of view: V, proposing proposal: CGSize) -> CGSize {
        let controller = NSHostingController(rootView: view)
        return controller.sizeThatFits(in: proposal)
    }
}

final class IslandPanel: NSPanel {
    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isFloatingPanel = true
        level = NSWindow.Level(rawValue: NSWindow.Level.mainMenu.rawValue + 3)
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        isMovable = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        becomesKeyOnlyIfNeeded = true
        collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary, .ignoresCycle]
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// Reports hover and clicks without activating the app.
final class IslandHostingView<Content: View>: NSHostingView<Content> {
    var onHover: ((Bool) -> Void)?
    /// The click's point in the view's top-left coordinates, as SwiftUI lays the island out.
    var onClick: ((CGPoint) -> Void)?
    private var area: NSTrackingArea?
    private var clickOrigin: CGPoint?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let area { removeTrackingArea(area) }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self,
            userInfo: nil
        )
        addTrackingArea(area)
        self.area = area
    }

    override func mouseEntered(with event: NSEvent) { onHover?(true) }
    override func mouseExited(with event: NSEvent) { onHover?(false) }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    override func mouseDown(with event: NSEvent) {
        clickOrigin = event.locationInWindow
        super.mouseDown(with: event)
    }
    override func mouseDragged(with event: NSEvent) {
        clickOrigin = nil
        super.mouseDragged(with: event)
    }
    override func scrollWheel(with event: NSEvent) {
        clickOrigin = nil
        super.scrollWheel(with: event)
    }
    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        guard let origin = clickOrigin else { return }
        clickOrigin = nil
        guard hypot(event.locationInWindow.x - origin.x, event.locationInWindow.y - origin.y) < 5 else { return }
        var point = convert(event.locationInWindow, from: nil)
        if !isFlipped {
            point.y = bounds.height - point.y
        }
        onClick?(point)
    }
}

/// The island's outline: flush with the top of the screen, with concave shoulders where it meets the
/// menu bar and rounded bottom corners.
struct IslandShape: Shape {
    var shoulder: CGFloat
    var radius: CGFloat

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(shoulder, radius) }
        set {
            shoulder = newValue.first
            radius = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        let shoulder = min(self.shoulder, rect.width / 4)
        let radius = min(self.radius, (rect.width - shoulder * 2) / 2, rect.height / 2)
        var path = Path()
        path.move(to: CGPoint(x: rect.minX, y: rect.minY))
        path.addQuadCurve(
            to: CGPoint(x: rect.minX + shoulder, y: rect.minY + shoulder),
            control: CGPoint(x: rect.minX + shoulder, y: rect.minY)
        )
        path.addLine(to: CGPoint(x: rect.minX + shoulder, y: rect.maxY - radius))
        path.addQuadCurve(
            to: CGPoint(x: rect.minX + shoulder + radius, y: rect.maxY),
            control: CGPoint(x: rect.minX + shoulder, y: rect.maxY)
        )
        path.addLine(to: CGPoint(x: rect.maxX - shoulder - radius, y: rect.maxY))
        path.addQuadCurve(
            to: CGPoint(x: rect.maxX - shoulder, y: rect.maxY - radius),
            control: CGPoint(x: rect.maxX - shoulder, y: rect.maxY)
        )
        path.addLine(to: CGPoint(x: rect.maxX - shoulder, y: rect.minY + shoulder))
        path.addQuadCurve(
            to: CGPoint(x: rect.maxX, y: rect.minY),
            control: CGPoint(x: rect.maxX - shoulder, y: rect.minY)
        )
        path.closeSubpath()
        return path
    }
}

struct IslandRootView: View {
    @ObservedObject var model: IslandModel

    var body: some View {
        if let document = model.document, let geometry = model.geometry {
            let size = shapeSize(geometry)
            ZStack(alignment: .top) {
                background(geometry)
                    .frame(width: size.width, height: size.height)
                content(document, geometry: geometry)
                    .frame(width: size.width, height: size.height, alignment: .top)
                    .clipped()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .coordinateSpace(name: IslandTabFrames.space)
            .onPreferenceChange(IslandTabFrames.self) { frames in
                model.tabFrames = frames
            }
            .onPreferenceChange(IslandFooterFrame.self) { frame in
                model.footerFrame = frame
            }
            .environment(\.locale, document.resolvedLocale)
        }
    }

    private func shapeSize(_ geometry: IslandGeometry) -> CGSize {
        model.mode.isOpen ? model.expandedSize : model.compactSize(for: geometry)
    }

    @ViewBuilder
    private func background(_ geometry: IslandGeometry) -> some View {
        let shadow = Color.black.opacity(model.mode.isOpen ? 0.35 : 0)
        if geometry.hasNotch {
            IslandShape(shoulder: 6, radius: model.mode.isOpen ? 22 : 10)
                .fill(Color.black)
                .shadow(color: shadow, radius: 10, y: 4)
        } else {
            RoundedRectangle(cornerRadius: model.mode.isOpen ? 18 : IslandGeometry.pillHeight / 2, style: .continuous)
                .fill(Color.black)
                .shadow(color: shadow, radius: 10, y: 4)
        }
    }

    @ViewBuilder
    private func content(_ document: GlanceDocument, geometry: IslandGeometry) -> some View {
        switch model.mode {
        case .compact:
            TimelineView(.periodic(from: .now, by: 30)) { context in
                IslandWings(
                    layout: model.wingLayout,
                    geometry: geometry,
                    size: model.compactSize(for: geometry),
                    content: model.wingContent,
                    style: document.island.style,
                    units: document.labels.units,
                    now: context.date
                )
            }
            .transition(.opacity)
        case .expanded:
            TimelineView(.periodic(from: .now, by: 30)) { context in
                IslandDetails(
                    document: document, now: context.date, topInset: geometry.detailsInset,
                    budget: model.budget, selected: model.selectedTab,
                    availableWidth: model.expandedSize.width, viewportHeight: model.expandedSize.height
                )
            }
            .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .top)))
        case let .alert(alert):
            let look = model.look(for: alert)
            IslandAlertView(alert: alert, mark: look.mark, tint: look.tint, topInset: geometry.detailsInset)
                .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .top)))
        }
    }
}

/// The readings either side of the notch (or inside the pill). Both wings share one width so the
/// notch stays centered, and each reading sits against the island's outer edge, clear of the notch.
struct IslandWings: View {
    let layout: IslandWingLayout
    let geometry: IslandGeometry
    let size: CGSize
    let content: CGFloat
    let style: IslandStyle
    let units: GlanceUnits
    let now: Date

    var body: some View {
        let parts = layout.pieces(hasNotch: geometry.hasNotch)
        Group {
            if geometry.hasNotch {
                notched(parts)
            } else {
                pill(parts)
            }
        }
        .frame(width: size.width, height: size.height)
    }

    private func notched(_ parts: [IslandWingPiece.Part]) -> some View {
        let wing = max((size.width - geometry.notchWidth) / 2, 0)
        let room = max(wing - IslandGeometry.wingOuterPadding - IslandGeometry.wingInnerGap, 0)
        return HStack(spacing: 0) {
            piece(parts, at: 0)
                .frame(maxWidth: room, alignment: .leading)
                .padding(.leading, IslandGeometry.wingOuterPadding)
                .padding(.trailing, IslandGeometry.wingInnerGap)
                .frame(width: wing, alignment: .leading)
            Color.clear.frame(width: geometry.notchWidth)
            piece(parts, at: 1)
                .frame(maxWidth: room, alignment: .trailing)
                .padding(.leading, IslandGeometry.wingInnerGap)
                .padding(.trailing, IslandGeometry.wingOuterPadding)
                .frame(width: wing, alignment: .trailing)
        }
    }

    private func pill(_ parts: [IslandWingPiece.Part]) -> some View {
        let room = min(ceil(content), IslandGeometry.maxWing)
        return HStack(spacing: IslandGeometry.pillGap) {
            if parts.isEmpty {
                Image(systemName: "gauge.with.dots.needle.33percent")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.8))
            }
            ForEach(parts.indices, id: \.self) { index in
                piece(parts, at: index)
                    .frame(width: room, alignment: parts.count == 1 ? .center : (index == 0 ? .leading : .trailing))
            }
        }
    }

    @ViewBuilder
    private func piece(_ parts: [IslandWingPiece.Part], at index: Int) -> some View {
        if index < parts.count, let slot = layout.slot(at: index) {
            IslandWingPiece(slot: slot, part: parts[index], style: style, units: units, now: now)
        } else {
            Color.clear.frame(width: 0, height: 0)
        }
    }
}

/// One reading beside the notch: the account's mark with the value as a percentage, a ring or a
/// bar with the value beside it. A lone reading is split across the notch: `lead` draws the mark
/// (or the ring around it) on the left, `trail` the value (and the bar) on the right. A reading
/// without a limit has no ring or bar to fill, so it always shows its value; a countdown reading
/// shows its words at `now`.
struct IslandWingPiece: View {
    enum Part {
        case whole
        case lead
        case trail
    }

    let slot: IslandSlot
    let part: Part
    let style: IslandStyle
    let units: GlanceUnits
    let now: Date

    static let markSize: CGFloat = 14
    static let ringSize: CGFloat = 22
    static let barWidth: CGFloat = 26

    var body: some View {
        switch part {
        case .whole:
            HStack(spacing: 5) {
                lead
                trail
            }
        case .lead:
            lead
        case .trail:
            trail
        }
    }

    /// The reading as the popup's row reads it at `now`: rolled over once its reset has passed, its
    /// color the pace verdict's at `now`.
    private var metric: GlanceMetric { slot.metric.reading(at: now, pacing: .colorOnly) }

    private var fraction: Double? { style == .percent ? nil : metric.fraction }

    @ViewBuilder
    private var lead: some View {
        if style == .ring, let fraction {
            GlanceRing(fraction: fraction, severity: metric.severity, onDark: true, lineWidth: 2.4) {
                mark.padding(4)
            }
            .frame(width: Self.ringSize, height: Self.ringSize)
        } else {
            mark.frame(width: Self.markSize, height: Self.markSize)
        }
    }

    @ViewBuilder
    private var trail: some View {
        if style == .bar, let fraction {
            HStack(spacing: 5) {
                GlanceMeter(fraction: fraction, severity: metric.severity, onDark: true, height: 5)
                    .frame(width: Self.barWidth)
                value
            }
        } else {
            value
        }
    }

    private var mark: some View {
        ProviderMark(mark: slot.provider.mark, brand: slot.provider.brand)
            .foregroundStyle(slot.provider.islandMarkColor)
    }

    /// The value in the pace color only where no ring or bar beside it carries that color, as the
    /// popup's rows leave it to their meter, after the limit window's name (`5h`, `week`) as the menu
    /// bar strip labels it.
    private var value: some View {
        HStack(alignment: .firstTextBaseline, spacing: 3.5) {
            if let period = metric.period {
                Text(period)
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(0.85))
                    .lineLimit(1)
                    .fixedSize()
            }
            Text(metric.liveValue(now: now, units: units))
                .font(.system(size: 12.5, weight: .semibold))
                .monospacedDigit()
                .foregroundStyle(fraction == nil ? GlancePalette.text(metric.severity, onDark: true) : Color.white)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .contentTransition(.numericText())
        }
    }
}
