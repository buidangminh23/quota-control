import CoreText
import SwiftUI

/// The popup's typeface, Inter, for every word and number on the island and the widgets. The files
/// are the subsets the popup bundles, which `scripts/macos-widget.mjs` puts in the bundle's
/// `Resources/Fonts`: the app's for the island, the extension's for the widgets. Latin text draws
/// from the Latin subset and other letters fall through to the Latin Extended and Vietnamese subsets,
/// as the popup's `unicode-range` rules do. The subsets carry no `cv11` or `ss01`, so the alternate
/// letters `src/styles/base.css` asks for never show in the popup either. Without the files (a
/// development build) the system font stands in.
enum GlanceFont {
    private static let subsets = ["Inter-Latin", "Inter-LatinExt", "Inter-Vietnamese"]
    /// The OpenType tag of the weight axis, `wght`.
    private static let weightAxis = NSNumber(value: 0x7767_6874)
    private static let lock = NSLock()
    private static var cache: [String: Font] = [:]
    private static let faces: [CTFontDescriptor]? = Bundle.main.resourceURL.flatMap {
        load(from: $0.appendingPathComponent("Fonts", isDirectory: true))
    }

    /// The subsets in `folder`, Latin first, or `nil` unless every one is there.
    static func load(from folder: URL) -> [CTFontDescriptor]? {
        let faces = subsets.compactMap { name -> CTFontDescriptor? in
            let url = folder.appendingPathComponent("\(name).woff2", isDirectory: false) as CFURL
            return (CTFontManagerCreateFontDescriptorsFromURL(url) as? [CTFontDescriptor])?.first
        }
        return faces.count == subsets.count ? faces : nil
    }

    static func font(size: CGFloat, weight: Font.Weight) -> Font {
        guard let faces else { return .system(size: size, weight: weight) }
        let key = "\(size)|\(value(of: weight))"
        lock.lock()
        defer { lock.unlock() }
        if let font = cache[key] { return font }
        let font = Font(ctFont(faces: faces, size: size, weight: weight))
        cache[key] = font
        return font
    }

    /// Inter at `size` and `weight` from `faces` (`load(from:)`), the later subsets as its cascade.
    static func ctFont(faces: [CTFontDescriptor], size: CGFloat, weight: Font.Weight) -> CTFont {
        let styled = faces.map { face -> CTFontDescriptor in
            let attributes: [CFString: Any] = [kCTFontVariationAttribute: [weightAxis: value(of: weight)]]
            return CTFontDescriptorCreateCopyWithAttributes(face, attributes as CFDictionary)
        }
        let primary = CTFontDescriptorCreateCopyWithAttributes(
            styled[0],
            [kCTFontCascadeListAttribute: Array(styled.dropFirst())] as CFDictionary
        )
        return CTFontCreateWithFontDescriptor(primary, size, nil)
    }

    /// The CSS weight SwiftUI's weight stands for.
    static func value(of weight: Font.Weight) -> Double {
        switch weight {
        case .ultraLight:
            return 100
        case .thin:
            return 200
        case .light:
            return 300
        case .medium:
            return 500
        case .semibold:
            return 600
        case .bold:
            return 700
        case .heavy:
            return 800
        case .black:
            return 900
        default:
            return 400
        }
    }
}

extension Font {
    /// The popup's typeface at `size` points (`GlanceFont`).
    static func glance(size: CGFloat, weight: Font.Weight = .regular) -> Font {
        GlanceFont.font(size: size, weight: weight)
    }
}
