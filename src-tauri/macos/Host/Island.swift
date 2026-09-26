import AppKit
import SwiftUI

/// The Dynamic Island: a black shape around the MacBook notch whose two wings carry the readings
/// picked in Settings, as a percentage, a ring or a bar. Hovering it (or, when Settings say so, a
/// click) opens a detail view with the accounts its Settings list, a new alert (a limit close to
/// running out, or one that came back) opens it for a few seconds, and a click on the open island
/// opens the popup right below it. A screen without a notch gets the same island as a pill in the middle of the menu
/// bar. It lives in a non-activating panel, so it never takes focus from the app in front.
@MainActor
final class IslandController {
    static let shared = IslandController()

    private let model = IslandModel()
    private var panel: IslandPanel?
    private var hosting: IslandHostingView<IslandRootView>?
    private var handler: QCIslandHandler?
    private var observers: [NSObjectProtocol] = []
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
    }

    func update(_ data: Data) {
        guard let document = GlanceDocument.decode(data) else { return }
        model.document = document
        if model.mode == .expanded {
            model.measureExpanded()
        }
        if let alert = document.alert, seenAlerts.insert(alert.id).inserted, document.island.enabled, document.island.alerts {
            showAlert(alert)
        }
        relayout()
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

    // MARK: Layout

    private var shouldShow: Bool {
        guard let document = model.document else { return false }
        return document.island.enabled && !model.slots.isEmpty
    }

    private func relayout() {
        guard shouldShow, let geometry = IslandGeometry.current() else {
            cancelPending()
            pendingShrink?.cancel()
            hovering = false
            model.mode = .compact
            panel?.orderOut(nil)
            return
        }
        model.geometry = geometry
        let panel = ensurePanel()
        panel.setFrame(frame(for: model.mode == .compact ? .compact : .canvas, geometry: geometry), display: true)
        panel.orderFrontRegardless()
    }

    private enum FrameKind {
        case compact
        case canvas
    }

    private func frame(for kind: FrameKind, geometry: IslandGeometry) -> NSRect {
        let size: CGSize
        switch kind {
        case .compact:
            size = geometry.compactSize
        case .canvas:
            size = CGSize(
                width: max(geometry.compactSize.width, model.expandedSize.width) + IslandGeometry.shadowMargin * 2,
                height: max(geometry.compactSize.height, model.expandedSize.height) + IslandGeometry.shadowMargin
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
        hosting.onClick = { [weak self] in self?.click() }
        panel.contentView = hosting
        self.panel = panel
        self.hosting = hosting
        return panel
    }

    // MARK: Interaction

    private var expandsOnHover: Bool { model.document?.island.expandOnHover ?? true }

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
        } else if model.mode == .expanded {
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated { self?.setMode(.compact) }
            }
            pendingCollapse = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.leaveDelay, execute: work)
        }
    }

    private func click() {
        if model.mode == .compact && !expandsOnHover {
            pendingCollapse?.cancel()
            expand()
            return
        }
        open()
    }

    private func expand() {
        guard !popupVisible else { return }
        alertTimer?.cancel()
        model.measureExpanded()
        setMode(.expanded)
    }

    private func showAlert(_ alert: GlanceAlert) {
        guard !hovering, !popupVisible else { return }
        alertTimer?.cancel()
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

    private func setMode(_ mode: IslandMode) {
        guard let geometry = model.geometry ?? IslandGeometry.current(), shouldShow else { return }
        pendingShrink?.cancel()
        if mode != .compact {
            panel?.setFrame(frame(for: .canvas, geometry: geometry), display: true)
        }
        withAnimation(.spring(response: 0.38, dampingFraction: 0.82)) {
            model.mode = mode
        }
        if mode == .compact {
            let work = DispatchWorkItem { [weak self] in
                MainActor.assumeIsolated {
                    guard let self, self.model.mode == .compact, let panel = self.panel else { return }
                    panel.setFrame(self.frame(for: .compact, geometry: geometry), display: true)
                }
            }
            pendingShrink = work
            DispatchQueue.main.asyncAfter(deadline: .now() + Self.settleDelay, execute: work)
        }
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

    static let wing: CGFloat = 64
    static let expandedWidth: CGFloat = 380
    /// Wide enough for two columns of accounts.
    static let wideExpandedWidth: CGFloat = 460

    static func expandedWidth(for document: GlanceDocument) -> CGFloat {
        document.visibleProviders.count > 3 ? wideExpandedWidth : expandedWidth
    }
    static let pillHeight: CGFloat = 22
    /// Room around the open island for its shadow.
    static let shadowMargin: CGFloat = 16

    var pillInset: CGFloat { max(0, (barHeight - Self.pillHeight) / 2) }

    /// Space above the details: the notch itself, or a little air under the top of the pill.
    var detailsInset: CGFloat { hasNotch ? barHeight : 8 }

    var compactSize: CGSize {
        if hasNotch {
            return CGSize(width: notchWidth + Self.wing * 2, height: barHeight)
        }
        return CGSize(width: Self.wing * 2 + 14, height: Self.pillHeight)
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
                scale: notched.backingScaleFactor
            )
        }
        guard let screen = screens.first else { return nil }
        let menuBar = screen.frame.maxY - screen.visibleFrame.maxY
        return IslandGeometry(
            screenFrame: screen.frame,
            hasNotch: false,
            notchWidth: 0,
            barHeight: menuBar > 0 ? menuBar : 24,
            scale: screen.backingScaleFactor
        )
    }
}

/// What a wing shows: a provider's mark and one reading.
struct IslandSlot: Equatable, Identifiable {
    var id: String
    var provider: GlanceProvider
    var metric: GlanceMetric
}

@MainActor
final class IslandModel: ObservableObject {
    @Published var document: GlanceDocument?
    @Published var mode: IslandMode = .compact
    @Published var geometry: IslandGeometry?
    @Published var expandedSize = CGSize(width: IslandGeometry.expandedWidth, height: 120)

    /// The left and right wings, as the popup chose them: a picked metric, or the next reading of
    /// the island's accounts.
    var slots: [IslandSlot] {
        guard let document else { return [] }
        return document.island.wings.prefix(2).compactMap { provider in
            provider.metrics.first.map { IslandSlot(id: "\(provider.id)|\($0.id)", provider: provider, metric: $0) }
        }
    }

    func measureExpanded() {
        guard let document, let geometry else { return }
        let width = IslandGeometry.expandedWidth(for: document)
        let view = IslandDetails(document: document, now: Date(), topInset: geometry.detailsInset)
            .frame(width: width)
        expandedSize = fitted(view, width: width)
    }

    func measureAlert(_ alert: GlanceAlert) {
        guard let geometry else { return }
        let view = IslandAlertView(alert: alert, provider: provider(for: alert), topInset: geometry.detailsInset)
            .frame(width: IslandGeometry.expandedWidth)
        expandedSize = fitted(view, width: IslandGeometry.expandedWidth)
    }

    func provider(for alert: GlanceAlert) -> GlanceProvider? {
        guard let brand = alert.brand else { return nil }
        return document?.providers.first { $0.brand == brand }
    }

    private func fitted<V: View>(_ view: V, width: CGFloat) -> CGSize {
        let controller = NSHostingController(rootView: view)
        let size = controller.sizeThatFits(in: CGSize(width: width, height: 2000))
        return CGSize(width: width, height: ceil(size.height))
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
    var onClick: (() -> Void)?
    private var area: NSTrackingArea?

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
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) { onClick?() }
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
            .environment(\.locale, document.resolvedLocale)
        }
    }

    private func shapeSize(_ geometry: IslandGeometry) -> CGSize {
        model.mode.isOpen ? model.expandedSize : geometry.compactSize
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
            IslandWings(slots: model.slots, geometry: geometry, style: document.island.style)
                .transition(.opacity)
        case .expanded:
            TimelineView(.periodic(from: .now, by: 30)) { context in
                IslandDetails(document: document, now: context.date, topInset: geometry.detailsInset)
            }
            .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .top)))
        case let .alert(alert):
            IslandAlertView(alert: alert, provider: model.provider(for: alert), topInset: geometry.detailsInset)
                .transition(.opacity.combined(with: .scale(scale: 0.96, anchor: .top)))
        }
    }
}

/// The two readings either side of the notch (or inside the pill).
struct IslandWings: View {
    let slots: [IslandSlot]
    let geometry: IslandGeometry
    let style: IslandStyle

    var body: some View {
        HStack(spacing: 0) {
            if let first = slots.first {
                IslandSlotView(slot: first, style: style)
                    .frame(width: IslandGeometry.wing, alignment: geometry.hasNotch ? .leading : .center)
                    .padding(.leading, geometry.hasNotch ? 12 : 0)
            }
            Spacer(minLength: 0)
            if slots.count > 1 {
                IslandSlotView(slot: slots[1], style: style)
                    .frame(width: IslandGeometry.wing, alignment: geometry.hasNotch ? .trailing : .center)
                    .padding(.trailing, geometry.hasNotch ? 12 : 0)
            }
        }
        .frame(width: geometry.compactSize.width, height: geometry.compactSize.height)
    }
}

/// One wing: the account's mark with the reading as a percentage, a ring or a bar. A reading
/// without a limit has no ring or bar to fill, so it always shows its value.
struct IslandSlotView: View {
    let slot: IslandSlot
    let style: IslandStyle

    var body: some View {
        switch style {
        case .ring where slot.metric.fraction != nil:
            HStack(spacing: 4) {
                GlanceRing(fraction: slot.metric.fraction, severity: slot.metric.severity, onDark: true, lineWidth: 2.2) {
                    ProviderMark(mark: slot.provider.mark)
                        .foregroundStyle(slot.provider.tint)
                        .padding(3.2)
                }
                .frame(width: 18, height: 18)
                value(size: 11.5)
            }
        case .bar where slot.metric.fraction != nil:
            HStack(spacing: 5) {
                mark(size: 12)
                GlanceMeter(fraction: slot.metric.fraction ?? 0, severity: slot.metric.severity, onDark: true, height: 5)
                    .frame(width: 34)
            }
        default:
            HStack(spacing: 4) {
                mark(size: 13)
                value(size: 12)
            }
        }
    }

    private func mark(size: CGFloat) -> some View {
        ProviderMark(mark: slot.provider.mark)
            .foregroundStyle(slot.provider.tint)
            .frame(width: size, height: size)
    }

    private func value(size: CGFloat) -> some View {
        Text(slot.metric.value)
            .font(.system(size: size, weight: .semibold))
            .monospacedDigit()
            .foregroundStyle(GlancePalette.text(slot.metric.severity, onDark: true))
            .lineLimit(1)
            .minimumScaleFactor(0.75)
            .contentTransition(.numericText())
    }
}

/// The island's accounts with their meters and countdowns, under the notch: one column for up to
/// three accounts, two beyond that, fewer rows per account as the list grows.
struct IslandDetails: View {
    let document: GlanceDocument
    let now: Date
    let topInset: CGFloat

    var body: some View {
        let providers = document.visibleProviders
        let perAccount = providers.count <= 2 ? 4 : (providers.count <= 4 ? 2 : 1)
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            Group {
                if providers.isEmpty {
                    Text(document.island.empty ?? document.labels.empty)
                        .font(.system(size: 11.5))
                        .foregroundStyle(Color.white.opacity(0.7))
                        .fixedSize(horizontal: false, vertical: true)
                } else if providers.count > 3 {
                    HStack(alignment: .top, spacing: 16) {
                        column(stride(from: 0, to: providers.count, by: 2).map { providers[$0] }, perAccount: perAccount)
                        column(stride(from: 1, to: providers.count, by: 2).map { providers[$0] }, perAccount: perAccount)
                    }
                } else {
                    column(providers, perAccount: perAccount)
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 10)
            HStack {
                Text("\(document.labels.updated) \(GlanceFormat.time(document.generatedAt, locale: document.resolvedLocale, hour12: document.hour12))")
                Spacer()
                Text(document.labels.open)
            }
            .font(.system(size: 10))
            .foregroundStyle(Color.white.opacity(0.45))
            .padding(.horizontal, 20)
            .padding(.top, 12)
            .padding(.bottom, 14)
        }
    }

    private func column(_ providers: [GlanceProvider], perAccount: Int) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(providers) { provider in
                VStack(alignment: .leading, spacing: 7) {
                    GlanceProviderHeader(provider: provider, shows: document.island.shows, onDark: true, size: 13)
                    if provider.metrics.isEmpty {
                        GlanceNoticeRow(text: provider.notice ?? document.labels.noData, onDark: true)
                    }
                    ForEach(provider.metrics.prefix(perAccount)) { metric in
                        GlanceMetricRow(
                            metric: metric,
                            labels: document.labels,
                            now: now,
                            onDark: true,
                            compact: true,
                            showsReset: document.island.shows.resets
                        )
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

/// A short notice: a limit about to run out, or one that came back.
struct IslandAlertView: View {
    let alert: GlanceAlert
    let provider: GlanceProvider?
    let topInset: CGFloat

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(height: topInset)
            HStack(alignment: .top, spacing: 12) {
                ProviderMark(mark: provider?.mark)
                    .foregroundStyle(provider?.tint ?? .white)
                    .frame(width: 22, height: 22)
                VStack(alignment: .leading, spacing: 3) {
                    Text(alert.title)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(GlancePalette.text(alert.severity, onDark: true))
                    Text(alert.body)
                        .font(.system(size: 11.5))
                        .foregroundStyle(Color.white.opacity(0.75))
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 20)
            .padding(.top, 10)
            .padding(.bottom, 16)
        }
    }
}
