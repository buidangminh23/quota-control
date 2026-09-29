import AppKit
import CoreText
import SwiftUI

/// The menu bar strip as the popup describes it (`src/strip/native.ts`): the same marks, window
/// names and readings the popup draws into its own picture of the strip, with the sizes it draws
/// them at. The system draws it from here, so the text is sharp on every display and follows the
/// menu bar's light or dark look the moment it changes.
struct StripDocument: Decodable, Equatable {
    var version: Int
    /// Device pixels per point the popup lays its picture out at. The layout here works in the
    /// same pixels, fonts included, so both pictures place every glyph alike.
    var scale: Double
    /// Height of the band in those pixels.
    var height: Double
    var metrics: StripMetrics
    var groups: [StripGroup]
    /// The readings in words, for VoiceOver.
    var text: String

    var drawable: Bool {
        version == 1 && scale.isFinite && scale >= 1 && scale <= 8 && height.isFinite && height >= 1 && height <= 512
            && !groups.isEmpty && groups.count <= 64
    }
}

struct StripMetrics: Decodable, Equatable {
    var singleSize: Double
    var singleWeight: Double
    var stackedSize: Double
    var stackedWeight: Double
    var markSide: Double
    var markGap: Double
    var markInset: Double
    var groupGap: Double
    var sidePadding: Double
    var labelSize: Double?
    var labelGap: Double
    var labelWeight: Double
    var labelAlpha: Double
    var rowGap: Double
    var edge: Double
    var minValue: String
}

struct StripGroup: Decodable, Equatable {
    var brand: String
    /// The mark's color on a light and on a dark menu bar; the text color when the brand has none.
    var light: String?
    var dark: String?
    var mark: GlanceMark?
    var rows: [StripRow]
}

struct StripRow: Decodable, Equatable {
    var label: String?
    var value: String
}

/// Where everything on the strip goes, in the document's pixels from the top-left corner. A port
/// of the popup's `renderTextStrip` and `layout.ts`, measured with the fonts drawn here.
struct StripLayout: Equatable {
    struct Group: Equatable {
        var mark: Double
        var left: Double
        var right: Double
        var baselines: [Double]
    }

    struct FontExtents: Equatable {
        var capAscent: Double
        var ascent: Double
        var descent: Double
    }

    var width: Double
    var height: Double
    var groups: [Group]

    init(_ document: StripDocument) {
        let metrics = document.metrics
        let scale = document.scale
        let labelFont = StripFonts.label(document)
        var placed: [Group] = []
        var x = metrics.sidePadding * scale
        for group in document.groups {
            let rows = Array(group.rows.prefix(2))
            let valueFont = StripFonts.value(document, rows: rows.count)
            let named = labelFont.map { font in
                rows.map { row in row.label.map { StripText.width($0, font) } }
            } ?? rows.map { _ in nil }
            let text = Self.textWidth(
                labels: named,
                values: rows.map { StripText.width($0.value, valueFont) },
                labelGap: metrics.labelGap * scale,
                minimumValue: StripText.width(metrics.minValue, valueFont)
            )
            let left = x + (metrics.markSide + metrics.markGap) * scale
            let right = left + text.rounded(.up)
            placed.append(Group(
                mark: x,
                left: left,
                right: right,
                baselines: Self.baselines(
                    rows: rows.count,
                    height: document.height,
                    font: Self.extents(value: valueFont, label: labelFont),
                    rowGap: metrics.rowGap * scale,
                    edge: metrics.edge * scale
                )
            ))
            x = right + metrics.groupGap * scale
        }
        let last = placed.last?.right ?? 0
        width = (last + metrics.sidePadding * scale).rounded(.up)
        height = document.height
        groups = placed
    }

    /// `groupTextWidth` in `layout.ts`: the window names in one column, the values right-aligned
    /// in the next, the value column never narrower than its widest expected reading.
    static func textWidth(labels: [Double?], values: [Double], labelGap: Double, minimumValue: Double) -> Double {
        let named = zip(labels, values).compactMap { label, value in label.map { ($0, value) } }
        let unnamed = zip(labels, values).filter { $0.0 == nil }.map(\.1)
        let source = named.isEmpty ? values : named.map(\.1)
        let valueColumn = max(minimumValue, source.max() ?? 0)
        let nameColumn = named.isEmpty ? 0 : (named.map(\.0).max() ?? 0) + labelGap
        return max(nameColumn + valueColumn, unnamed.max() ?? 0)
    }

    /// `stackBaselines` in `layout.ts`: the digits' ink centered on the band's middle, moved just
    /// enough that no ascender or descender leaves the band.
    static func baselines(rows: Int, height: Double, font: FontExtents, rowGap: Double, edge: Double) -> [Double] {
        let count = max(1, rows)
        let room = height - 2 * edge - font.ascent - font.descent
        let pitch = count > 1 ? max(0, min(font.ascent + rowGap, room / Double(count - 1))) : 0
        let span = Double(count - 1) * pitch
        var first = (height - font.capAscent - span) / 2 + font.capAscent
        let overflowBottom = first + span + font.descent - (height - edge)
        if overflowBottom > 0 { first -= overflowBottom }
        let overflowTop = edge - (first - font.ascent)
        if overflowTop > 0 { first += overflowTop }
        return (0..<count).map { first + Double($0) * pitch }
    }

    /// `rowFontMetrics` in `render.ts`: how far the ink of a reading or a window name can reach.
    static func extents(value: CTFont, label: CTFont?) -> FontExtents {
        let digits = StripText.ink("0123456789%", value)
        let tall = StripText.ink("hklđ", value)
        let deep = StripText.ink("gjpyợ", value)
        let name = label.map { StripText.ink("hklgpy", $0) } ?? (ascent: 0, descent: 0)
        return FontExtents(
            capAscent: digits.ascent,
            ascent: max(digits.ascent, tall.ascent, name.ascent),
            descent: max(0, deep.descent, name.descent)
        )
    }
}

enum StripFonts {
    static func value(_ document: StripDocument, rows: Int) -> CTFont {
        let metrics = document.metrics
        return rows > 1
            ? font(metrics.stackedSize * document.scale, metrics.stackedWeight)
            : font(metrics.singleSize * document.scale, metrics.singleWeight)
    }

    static func label(_ document: StripDocument) -> CTFont? {
        document.metrics.labelSize.map { font($0 * document.scale, document.metrics.labelWeight) }
    }

    private static func font(_ size: Double, _ weight: Double) -> CTFont {
        NSFont.systemFont(ofSize: CGFloat(size), weight: systemWeight(weight)) as CTFont
    }

    /// The system font's weight for a CSS weight, as the webview maps them.
    private static func systemWeight(_ css: Double) -> NSFont.Weight {
        switch css {
        case ..<150: return .ultraLight
        case ..<250: return .thin
        case ..<350: return .light
        case ..<450: return .regular
        case ..<550: return .medium
        case ..<650: return .semibold
        case ..<750: return .bold
        case ..<850: return .heavy
        default: return .black
        }
    }
}

enum StripText {
    static func line(_ text: String, _ font: CTFont, color: CGColor? = nil) -> CTLine {
        var attributes: [NSAttributedString.Key: Any] = [NSAttributedString.Key(kCTFontAttributeName as String): font]
        if let color {
            attributes[NSAttributedString.Key(kCTForegroundColorAttributeName as String)] = color
        }
        return CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attributes))
    }

    static func width(_ text: String, _ font: CTFont) -> Double {
        Double(CTLineGetTypographicBounds(line(text, font), nil, nil, nil))
    }

    /// How far the glyphs' ink rises above and falls below the baseline.
    static func ink(_ text: String, _ font: CTFont) -> (ascent: Double, descent: Double) {
        let bounds = CTLineGetBoundsWithOptions(line(text, font), .useGlyphPathBounds)
        guard !bounds.isNull, !bounds.isEmpty else { return (0, 0) }
        return (max(0, Double(bounds.maxY)), max(0, Double(-bounds.minY)))
    }
}

enum MenuBarStrip {
    /// The strip as an image the system draws whenever it shows it: at the display's own
    /// resolution, in the menu bar's look at that moment. Nothing is kept between two drawings.
    static func image(for document: StripDocument) -> NSImage {
        let layout = StripLayout(document)
        let size = NSSize(width: layout.width / document.scale, height: layout.height / document.scale)
        let image = NSImage(size: size, flipped: false) { _ in
            guard let context = NSGraphicsContext.current?.cgContext else { return false }
            draw(document, layout: layout, dark: isDark(NSAppearance.currentDrawing()), in: context)
            return true
        }
        image.cacheMode = .never
        image.isTemplate = false
        return image
    }

    static func isDark(_ appearance: NSAppearance) -> Bool {
        let match = appearance.bestMatch(from: [.aqua, .darkAqua, .vibrantLight, .vibrantDark])
        return match == .darkAqua || match == .vibrantDark
    }

    /// Draw the strip into `context`, whose unit is one point with the origin at the bottom left.
    static func draw(_ document: StripDocument, layout: StripLayout, dark: Bool, in context: CGContext) {
        let metrics = document.metrics
        let scale = document.scale
        let ink = CGColor(gray: dark ? 1 : 0, alpha: 1)
        let faint = CGColor(gray: dark ? 1 : 0, alpha: CGFloat(metrics.labelAlpha))
        let labelFont = StripFonts.label(document)
        let side = metrics.markSide * scale

        context.saveGState()
        context.scaleBy(x: CGFloat(1 / scale), y: CGFloat(1 / scale))
        context.textMatrix = .identity
        for (group, place) in zip(document.groups, layout.groups) {
            let tint = (dark ? group.dark : group.light).flatMap(color(hex:)) ?? ink
            let box = CGRect(x: place.mark, y: (layout.height - side) / 2, width: side, height: side)
            drawMark(group.mark, in: box, tint: tint, inset: metrics.markInset, context: context)

            let rows = Array(group.rows.prefix(2))
            let valueFont = StripFonts.value(document, rows: rows.count)
            for (row, baseline) in zip(rows, place.baselines) {
                let y = layout.height - baseline
                let value = StripText.line(row.value, valueFont, color: ink)
                let width = Double(CTLineGetTypographicBounds(value, nil, nil, nil))
                context.textPosition = CGPoint(x: place.right - width, y: y)
                CTLineDraw(value, context)
                guard let label = row.label, let labelFont else { continue }
                context.textPosition = CGPoint(x: place.left, y: y)
                CTLineDraw(StripText.line(label, labelFont, color: faint), context)
            }
        }
        context.restoreGState()
    }

    /// The mark in `box`: the brand's color logo, its single-color paths in `tint`, or a dot for a
    /// brand without a mark, as `drawMark` in `render.ts` draws them.
    private static func drawMark(_ mark: GlanceMark?, in box: CGRect, tint: CGColor, inset: Double, context: CGContext) {
        if let art = mark?.artImage, let picture = art.cgImage(forProposedRect: nil, context: nil, hints: nil) {
            context.saveGState()
            context.interpolationQuality = .high
            context.draw(picture, in: box)
            context.restoreGState()
            return
        }
        context.setFillColor(tint)
        guard let mark, mark.box.count == 4, !mark.paths.isEmpty else {
            let radius = max(0, box.width / 2 - 0.5)
            context.fillEllipse(in: CGRect(x: box.midX - radius, y: box.midY - radius, width: radius * 2, height: radius * 2))
            return
        }
        let (minX, minY, width, height) = (mark.box[0], mark.box[1], mark.box[2], mark.box[3])
        let artwork = max(width, height) * (1 + inset * 2)
        guard artwork > 0 else { return }
        let factor = Double(box.width) / artwork
        var upright = CGAffineTransform(
            a: CGFloat(factor), b: 0, c: 0, d: CGFloat(-factor),
            tx: box.midX - CGFloat(factor * (minX + width / 2)),
            ty: box.midY + CGFloat(factor * (minY + height / 2))
        )
        for component in mark.paths {
            guard let path = SVGPathParser.path(from: component.d).cgPath.copy(using: &upright) else { continue }
            context.addPath(path)
            context.fillPath(using: component.evenOdd == true ? .evenOdd : .winding)
        }
    }

    private static func color(hex text: String) -> CGColor? {
        var hex = text.trimmingCharacters(in: .whitespaces)
        if hex.hasPrefix("#") { hex.removeFirst() }
        guard hex.count == 6, let value = UInt32(hex, radix: 16) else { return nil }
        return CGColor(
            srgbRed: CGFloat((value >> 16) & 0xFF) / 255,
            green: CGFloat((value >> 8) & 0xFF) / 255,
            blue: CGFloat(value & 0xFF) / 255,
            alpha: 1
        )
    }

    /// The strip as a PNG `pixelsPerPoint` device pixels to the point, in the light or the dark
    /// look: what the menu bar shows on such a display.
    static func png(_ document: StripDocument, dark: Bool, pixelsPerPoint: Double) -> Data? {
        let layout = StripLayout(document)
        let points = CGSize(width: layout.width / document.scale, height: layout.height / document.scale)
        let width = Int((Double(points.width) * pixelsPerPoint).rounded(.up))
        let height = Int((Double(points.height) * pixelsPerPoint).rounded(.up))
        guard width > 0, height > 0,
              let context = CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
              )
        else { return nil }
        context.scaleBy(x: CGFloat(pixelsPerPoint), y: CGFloat(pixelsPerPoint))
        draw(document, layout: layout, dark: dark, in: context)
        guard let picture = context.makeImage() else { return nil }
        return NSBitmapImageRep(cgImage: picture).representation(using: .png, properties: [:])
    }
}

/// Puts the strip into the menu bar item and keeps it there. The item itself, its menu and its
/// clicks stay with the tray the app created; only the picture it shows comes from here.
@MainActor
final class MenuBarStripController {
    static let shared = MenuBarStripController()

    private weak var button: NSStatusBarButton?
    private var document: StripDocument?
    private var image: NSImage?
    private var appearance: NSKeyValueObservation?

    func show(_ item: NSStatusItem, _ data: Data) -> Bool {
        guard let button = item.button,
              let document = try? JSONDecoder().decode(StripDocument.self, from: data),
              document.drawable
        else { return false }
        if document == self.document, button === self.button, let image, button.image === image { return true }

        let image = MenuBarStrip.image(for: document)
        guard image.size.width >= 1, image.size.height >= 1 else { return false }
        button.image = image
        button.imagePosition = .imageLeft
        button.setAccessibilityLabel(document.text)
        followButton(button)
        if button !== self.button {
            appearance = button.observe(\.effectiveAppearance) { button, _ in
                DispatchQueue.main.async { button.needsDisplay = true }
            }
        }
        self.button = button
        self.document = document
        self.image = image
        return true
    }

    /// The item goes back to an ordinary icon: forget the strip, so the next one is drawn afresh.
    func clear() {
        appearance = nil
        button?.setAccessibilityLabel(nil)
        button = nil
        document = nil
        image = nil
    }

    /// The tray's own click target covers the button it was created over; let it follow the
    /// button's size, which now changes with every strip.
    private func followButton(_ button: NSStatusBarButton) {
        for view in button.subviews where view.className == "TaoTrayTarget" {
            view.autoresizingMask = [.width, .height]
            view.frame = button.bounds
        }
    }
}

/// Show the strip in the menu bar item. Must run on the main thread, which the tray's own calls
/// do; `false` leaves the item untouched, and the caller shows the popup's picture instead.
@_cdecl("qc_strip_show")
public func qcStripShow(_ item: UnsafeMutableRawPointer?, _ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Bool {
    guard Thread.isMainThread, let item, let bytes, length > 0 else { return false }
    let statusItem = Unmanaged<NSStatusItem>.fromOpaque(item).takeUnretainedValue()
    let data = Data(bytes: bytes, count: length)
    return MainActor.assumeIsolated { MenuBarStripController.shared.show(statusItem, data) }
}

@_cdecl("qc_strip_clear")
public func qcStripClear() {
    onMain { MenuBarStripController.shared.clear() }
}
