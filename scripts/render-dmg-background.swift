// Render the repository's vector brand using macOS' built-in drawing APIs.
// The PNG stores 2x pixels at 144 dpi for a 660 x 420 point Finder window.
import AppKit

guard CommandLine.arguments.count == 2 else {
    fatalError("Pass the output PNG path.")
}
let width: CGFloat = 660
let height: CGFloat = 420
let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: 1320, pixelsHigh: 840,
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
    isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
)!
bitmap.size = NSSize(width: width, height: height)
let graphics = NSGraphicsContext(bitmapImageRep: bitmap)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = graphics

func color(_ hex: UInt32, alpha: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 255) / 255,
            green: CGFloat((hex >> 8) & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: alpha)
}
func text(_ value: String, x: CGFloat, top: CGFloat, size: CGFloat,
          weight: NSFont.Weight = .regular, ink: UInt32 = 0x526258, centered: Bool = false) {
    let attributes: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: size, weight: weight),
        .foregroundColor: color(ink)
    ]
    let bounds = (value as NSString).size(withAttributes: attributes)
    (value as NSString).draw(at: NSPoint(x: centered ? x - bounds.width / 2 : x,
                                       y: height - top - bounds.height), withAttributes: attributes)
}

NSGradient(starting: color(0xf9fbf8), ending: color(0xeaf2ec))!
    .draw(in: NSRect(x: 0, y: 0, width: width, height: height), angle: -70)

text("Orivelle Orbit", x: 48, top: 38, size: 25, weight: .semibold, ink: 0x213e32)
text("让 AI 工作空间，常驻你的 Mac。", x: 49, top: 76, size: 13)

// Use the same orbit geometry and green as src/assets/orbit-mark.svg.
graphics.cgContext.saveGState()
graphics.cgContext.translateBy(x: 582, y: 350)
graphics.cgContext.rotate(by: 35 * .pi / 180)
color(0x326b58).setStroke()
let sphere = NSBezierPath(ovalIn: NSRect(x: -18, y: -18, width: 36, height: 36))
sphere.lineWidth = 2.1
sphere.stroke()
let orbit = NSBezierPath(ovalIn: NSRect(x: -23, y: -6.8, width: 46, height: 13.6))
orbit.lineWidth = 2.1
orbit.stroke()
color(0x326b58).setFill()
NSBezierPath(ovalIn: NSRect(x: -3, y: -3, width: 6, height: 6)).fill()
graphics.cgContext.restoreGState()

color(0xd7e3d9).setStroke()
let divider = NSBezierPath()
divider.move(to: NSPoint(x: 48, y: 302))
divider.line(to: NSPoint(x: 612, y: 302))
divider.lineWidth = 0.7
divider.stroke()

for center: CGFloat in [180, 480] {
    let card = NSBezierPath(roundedRect: NSRect(x: center - 83, y: 109, width: 166, height: 166),
                            xRadius: 25, yRadius: 25)
    color(0xffffff, alpha: 0.70).setFill()
    card.fill()
    color(0xd5e2d8).setStroke()
    card.lineWidth = 0.8
    card.stroke()
}
let arrow = NSBezierPath()
arrow.move(to: NSPoint(x: 310, y: 200))
arrow.line(to: NSPoint(x: 349, y: 200))
arrow.move(to: NSPoint(x: 340, y: 209))
arrow.line(to: NSPoint(x: 349, y: 200))
arrow.line(to: NSPoint(x: 340, y: 191))
arrow.lineWidth = 2.5
arrow.lineCapStyle = .round
arrow.lineJoinStyle = .round
color(0x709880).setStroke()
arrow.stroke()

text("将 Orbit 拖到 Applications 文件夹", x: 330, top: 339,
     size: 14, weight: .medium, ink: 0x365d47, centered: true)
text("Drag to install  ·  macOS 15 or later", x: 330, top: 367,
     size: 10.5, ink: 0x75867b, centered: true)
NSGraphicsContext.restoreGraphicsState()
try bitmap.representation(using: .png, properties: [:])!
    .write(to: URL(fileURLWithPath: CommandLine.arguments[1]), options: .atomic)
