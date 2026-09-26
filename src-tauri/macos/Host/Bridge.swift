import AppKit
import WidgetKit

/// The C entry points the Rust host calls. Every call may arrive on any thread; AppKit work hops to
/// the main thread, and byte buffers are copied before returning because Rust frees them after the
/// call.

public typealias QCIslandHandler = @convention(c) (Int32, Double, Double, Double, Double, Double) -> Void

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

@_cdecl("qc_widgets_reload")
public func qcWidgetsReload() {
    WidgetCenter.shared.reloadAllTimelines()
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
