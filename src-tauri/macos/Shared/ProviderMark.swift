import SwiftUI

/// A provider's logo from the same SVG path data the popup draws (`src/assets/providerMarks.ts`),
/// scaled into its box with a small inset so every mark fills the same space (upstream
/// `ProviderIconShape`). Paths keep their own fill rule. A brand without a mark shows its initial in
/// a ring, as the popup does; without a brand either, a dot. A brand with an official color logo
/// draws that picture in its own colors instead, ignoring the foreground tint, as the popup and the
/// taskbar strip do.
struct ProviderMark: View {
    let mark: GlanceMark?
    /// The brand whose initial stands in for a missing mark.
    var brand: String? = nil
    var inset: CGFloat = 0.04

    var body: some View {
        if let image = mark?.artImage {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .aspectRatio(contentMode: .fit)
        } else if let mark, mark.box.count == 4, !mark.paths.isEmpty {
            ZStack {
                ForEach(Array(mark.paths.enumerated()), id: \.offset) { _, component in
                    MarkPathShape(box: mark.box, data: component.d, inset: inset)
                        .fill(style: FillStyle(eoFill: component.evenOdd ?? false))
                }
            }
        } else if let initial = brand?.first {
            GeometryReader { proxy in
                let side = min(proxy.size.width, proxy.size.height)
                ZStack {
                    Circle().strokeBorder(lineWidth: max(1, side * 0.094))
                    Text(String(initial).uppercased())
                        .font(.glance(size: max(1, side * 0.62), weight: .bold))
                        .lineLimit(1)
                        .fixedSize()
                }
                .frame(width: side, height: side)
                .frame(width: proxy.size.width, height: proxy.size.height)
            }
        } else {
            Circle().padding(1)
        }
    }
}

private struct MarkPathShape: Shape {
    let box: [Double]
    let data: String
    let inset: CGFloat

    func path(in rect: CGRect) -> Path {
        let (minX, minY, width, height) = (box[0], box[1], box[2], box[3])
        let side = max(width, height) * (1 + inset * 2)
        guard side > 0 else { return Path() }
        let scale = min(rect.width, rect.height) / side
        let transform = CGAffineTransform(translationX: rect.midX, y: rect.midY)
            .scaledBy(x: scale, y: scale)
            .translatedBy(x: -(minX + width / 2), y: -(minY + height / 2))
        return SVGPathParser.path(from: data).applying(transform)
    }
}

/// Parses SVG path data: moveto, lineto (with the horizontal and vertical forms), cubic and
/// quadratic Béziers (with their smooth forms) and closepath, absolute and relative. An elliptical
/// arc is approximated by a line to its end point.
enum SVGPathParser {
    private enum Token {
        case command(Character)
        case number(Double)
    }

    static func path(from data: String) -> Path {
        var path = Path()
        let tokens = tokenize(data)
        var index = 0
        var current = CGPoint.zero
        var subpathStart = CGPoint.zero
        var lastCubicControl: CGPoint?
        var lastQuadControl: CGPoint?
        var command: Character?

        func number() -> CGFloat? {
            guard index < tokens.count, case let .number(value) = tokens[index] else { return nil }
            index += 1
            return CGFloat(value)
        }

        func point(relative: Bool) -> CGPoint? {
            guard let x = number(), let y = number() else { return nil }
            return relative ? CGPoint(x: current.x + x, y: current.y + y) : CGPoint(x: x, y: y)
        }

        while index < tokens.count {
            if case let .command(next) = tokens[index] {
                command = next
                index += 1
            }
            guard let active = command else { break }
            let relative = active.isLowercase
            var cubic: CGPoint?
            var quad: CGPoint?
            switch Character(active.uppercased()) {
            case "M":
                guard let target = point(relative: relative) else { return path }
                path.move(to: target)
                current = target
                subpathStart = target
                command = relative ? "l" : "L"
            case "L":
                guard let target = point(relative: relative) else { return path }
                path.addLine(to: target)
                current = target
            case "H":
                guard let x = number() else { return path }
                current = CGPoint(x: relative ? current.x + x : x, y: current.y)
                path.addLine(to: current)
            case "V":
                guard let y = number() else { return path }
                current = CGPoint(x: current.x, y: relative ? current.y + y : y)
                path.addLine(to: current)
            case "C":
                guard let first = point(relative: relative),
                      let second = point(relative: relative),
                      let target = point(relative: relative)
                else { return path }
                path.addCurve(to: target, control1: first, control2: second)
                cubic = second
                current = target
            case "S":
                let first = reflect(lastCubicControl, around: current)
                guard let second = point(relative: relative), let target = point(relative: relative) else { return path }
                path.addCurve(to: target, control1: first, control2: second)
                cubic = second
                current = target
            case "Q":
                guard let control = point(relative: relative), let target = point(relative: relative) else { return path }
                path.addQuadCurve(to: target, control: control)
                quad = control
                current = target
            case "T":
                let control = reflect(lastQuadControl, around: current)
                guard let target = point(relative: relative) else { return path }
                path.addQuadCurve(to: target, control: control)
                quad = control
                current = target
            case "A":
                guard number() != nil, number() != nil, number() != nil, number() != nil, number() != nil,
                      let target = point(relative: relative)
                else { return path }
                path.addLine(to: target)
                current = target
            case "Z":
                path.closeSubpath()
                current = subpathStart
                command = nil
            default:
                return path
            }
            lastCubicControl = cubic
            lastQuadControl = quad
        }
        return path
    }

    private static func reflect(_ control: CGPoint?, around point: CGPoint) -> CGPoint {
        guard let control else { return point }
        return CGPoint(x: 2 * point.x - control.x, y: 2 * point.y - control.y)
    }

    private static func tokenize(_ data: String) -> [Token] {
        var tokens: [Token] = []
        let characters = Array(data.utf8)
        var index = 0
        while index < characters.count {
            let byte = characters[index]
            let character = Character(UnicodeScalar(byte))
            if character.isLetter, character != "e", character != "E" {
                tokens.append(.command(character))
                index += 1
            } else if character == "-" || character == "+" || character == "." || character.isNumber {
                var end = index
                var seenDot = false
                var seenExponent = false
                if characters[end] == UInt8(ascii: "-") || characters[end] == UInt8(ascii: "+") { end += 1 }
                while end < characters.count {
                    let next = Character(UnicodeScalar(characters[end]))
                    if next.isNumber {
                        end += 1
                    } else if next == ".", !seenDot, !seenExponent {
                        seenDot = true
                        end += 1
                    } else if (next == "e" || next == "E"), !seenExponent {
                        seenExponent = true
                        end += 1
                        if end < characters.count,
                           characters[end] == UInt8(ascii: "-") || characters[end] == UInt8(ascii: "+") {
                            end += 1
                        }
                    } else {
                        break
                    }
                }
                if let text = String(bytes: characters[index..<end], encoding: .utf8), let value = Double(text) {
                    tokens.append(.number(value))
                }
                index = max(end, index + 1)
            } else {
                index += 1
            }
        }
        return tokens
    }
}
