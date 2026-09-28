import AppKit
import WidgetKit

/// The C entry points the Rust host calls. Every call may arrive on any thread; AppKit work hops to
/// the main thread, and byte buffers are copied before returning because Rust frees them after the
/// call.

public typealias QCIslandHandler = @convention(c) (Int32, Double, Double, Double, Double, Double) -> Void

public typealias QCAppearanceHandler = @convention(c) (Bool) -> Void

/// Island event codes passed to the handler.
enum IslandEvent: Int32 {
    /// The island was clicked; the rectangle (top-left origin, points) and scale follow.
    case open = 1
}

func onMain(_ work: @escaping @MainActor () -> Void) {
    if Thread.isMainThread {
        MainActor.assumeIsolated(work)
    } else {
        DispatchQueue.main.async { MainActor.assumeIsolated(work) }
    }
}

@_cdecl("qc_popup_configure")
public func qcPopupConfigure(_ window: UnsafeMutableRawPointer?, _ radius: Double) {
    guard let window else { return }
    let object = Unmanaged<NSWindow>.fromOpaque(window).takeUnretainedValue()
    onMain { PopupWindow.configure(object, radius: CGFloat(radius)) }
}

/// Runs after the resize that asked for it: tao applies a new size asynchronously on the main
/// queue, so running inline would rebuild the shadow for the old frame.
@_cdecl("qc_popup_refresh_shadow")
public func qcPopupRefreshShadow(_ window: UnsafeMutableRawPointer?) {
    guard let window else { return }
    let object = Unmanaged<NSWindow>.fromOpaque(window).takeUnretainedValue()
    DispatchQueue.main.async { object.invalidateShadow() }
}

@_cdecl("qc_island_start")
public func qcIslandStart(_ handler: QCIslandHandler?) {
    onMain { IslandController.shared.start(handler: handler) }
}

@_cdecl("qc_island_update")
public func qcIslandUpdate(_ bytes: UnsafePointer<UInt8>?, _ length: Int) {
    guard let bytes, length > 0 else { return }
    let data = Data(bytes: bytes, count: length)
    onMain { IslandController.shared.update(data) }
}

@_cdecl("qc_island_popup_visible")
public func qcIslandPopupVisible(_ visible: Bool) {
    onMain { IslandController.shared.setPopupVisible(visible) }
}

/// Report whether the menu bar reads dark now and whenever that changes. The strip is drawn in
/// color, not as a template the system tints, so its text color follows this.
@_cdecl("qc_menu_bar_appearance_start")
public func qcMenuBarAppearanceStart(_ handler: QCAppearanceHandler?) {
    onMain { MenuBarAppearance.shared.start(handler: handler) }
}

@_cdecl("qc_widgets_reload")
public func qcWidgetsReload() {
    onMain { WidgetCenter.shared.reloadAllTimelines() }
}

/// Stop widget extension processes left from an earlier version of the app, then reload the
/// widgets, so the desktop widget always runs this version's extension (`WidgetExtension`). Works
/// on a background queue and returns at once.
@_cdecl("qc_widgets_adopt_current")
public func qcWidgetsAdoptCurrent() {
    WidgetExtension.adoptInBackground {
        onMain { WidgetCenter.shared.reloadAllTimelines() }
    }
}

enum PopupWindow {
    /// A menu bar popover: above the menu bar on every Space, including over full-screen apps, with
    /// rounded corners and the system shadow.
    @MainActor
    static func configure(_ window: NSWindow, radius: CGFloat) {
        window.level = .statusBar
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = true
        if let view = window.contentView {
            view.wantsLayer = true
            view.layer?.cornerRadius = radius
            view.layer?.cornerCurve = .continuous
            view.layer?.masksToBounds = true
        }
        window.invalidateShadow()
    }
}

/// The menu bar's light or dark look. The status item's own window carries the appearance the
/// menu bar draws with (on a transparent menu bar it follows the wallpaper), so it is read there,
/// falling back to the app's appearance before the status item exists. A two-second check catches
/// wallpaper and theme changes, which post no single notification.
@MainActor
final class MenuBarAppearance {
    static let shared = MenuBarAppearance()

    private var handler: QCAppearanceHandler?
    private var timer: Timer?
    private var last: Bool?

    func start(handler: QCAppearanceHandler?) {
        self.handler = handler
        last = nil
        check()
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in
            MainActor.assumeIsolated { MenuBarAppearance.shared.check() }
        }
    }

    private func check() {
        let statusWindow = NSApp.windows.first { String(describing: type(of: $0)) == "NSStatusBarWindow" }
        let appearance = statusWindow?.effectiveAppearance ?? NSApp.effectiveAppearance
        let match = appearance.bestMatch(from: [.aqua, .darkAqua, .vibrantLight, .vibrantDark])
        let dark = match == .darkAqua || match == .vibrantDark
        guard dark != last else { return }
        last = dark
        handler?(dark)
    }
}
