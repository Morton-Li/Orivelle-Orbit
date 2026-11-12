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
text("连接所需，专注所想。", x: 49, top: 76, size: 13)

// Use the same orbit geometry and green as src/assets/orbit-mark.svg.
let brandURL = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("../src/assets/orbit-mark.svg").standardizedFileURL
guard let brand = NSImage(contentsOf: brandURL) else {
    fatalError("Cannot load the Orbit vector mark.")
}
brand.draw(in: NSRect(x: 550, y: 318, width: 64, height: 64))

color(0xd7e3d9).setStroke()
let divider = NSBezierPath()
divider.move(to: NSPoint(x: 48, y: 312))
divider.line(to: NSPoint(x: 612, y: 312))
divider.lineWidth = 0.7
divider.stroke()

for center: CGFloat in [180, 480] {
    // Finder draws the filename below the 128-point icon. Reserve space for
    // both, including bottom padding, rather than framing only the icon.
    let card = NSBezierPath(roundedRect: NSRect(x: center - 95, y: 105, width: 190, height: 190),
                            xRadius: 25, yRadius: 25)
    color(0xffffff, alpha: 0.70).setFill()
    card.fill()
    color(0xd5e2d8).setStroke()
    card.lineWidth = 0.8
    card.stroke()
}
let arrow = NSBezierPath()
arrow.move(to: NSPoint(x: 310, y: 220))
arrow.line(to: NSPoint(x: 349, y: 220))
arrow.move(to: NSPoint(x: 340, y: 229))
arrow.line(to: NSPoint(x: 349, y: 220))
arrow.line(to: NSPoint(x: 340, y: 211))
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
