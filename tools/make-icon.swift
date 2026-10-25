// Renders assets/AppIcon.icns. Run: swift tools/make-icon.swift && iconutil -c icns target/AppIcon.iconset -o assets/AppIcon.icns
import AppKit

func render(_ px: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px)
    let inset = s * 0.1
    let rect = NSRect(x: inset, y: inset, width: s - inset * 2, height: s - inset * 2)
    let shape = NSBezierPath(roundedRect: rect, xRadius: rect.width * 0.225, yRadius: rect.width * 0.225)

    // Drop shadow + body gradient (deep navy → electric blue).
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.35)
    shadow.shadowBlurRadius = s * 0.03
    shadow.shadowOffset = NSSize(width: 0, height: -s * 0.012)
    shadow.set()
    NSColor.black.setFill()
    shape.fill()
    NSGraphicsContext.restoreGraphicsState()
    NSGradient(colors: [NSColor(red: 0.05, green: 0.07, blue: 0.2, alpha: 1),
                        NSColor(red: 0.12, green: 0.25, blue: 0.75, alpha: 1),
                        NSColor(red: 0.3, green: 0.6, blue: 1, alpha: 1)])!.draw(in: shape, angle: 70)

    // Soft glow in the middle.
    NSGraphicsContext.saveGraphicsState()
    shape.addClip()
    NSGradient(colors: [NSColor(red: 0.45, green: 0.7, blue: 1, alpha: 0.55), .clear])!
        .draw(in: NSBezierPath(ovalIn: rect.insetBy(dx: rect.width * 0.12, dy: rect.width * 0.12)), relativeCenterPosition: .zero)
    NSGraphicsContext.restoreGraphicsState()

    // Inner highlight rim.
    NSColor.white.withAlphaComponent(0.18).setStroke()
    let rim = NSBezierPath(roundedRect: rect.insetBy(dx: s * 0.004, dy: s * 0.004), xRadius: rect.width * 0.22, yRadius: rect.width * 0.22)
    rim.lineWidth = s * 0.006
    rim.stroke()

    // Headphones glyph with a waveform underneath.
    func symbol(_ name: String, _ pt: CGFloat, _ weight: NSFont.Weight, center: NSPoint, alpha: CGFloat) {
        let config = NSImage.SymbolConfiguration(pointSize: pt, weight: weight)
            .applying(.init(paletteColors: [NSColor.white.withAlphaComponent(alpha)]))
        guard let img = NSImage(systemSymbolName: name, accessibilityDescription: nil)?.withSymbolConfiguration(config) else { return }
        img.draw(in: NSRect(x: center.x - img.size.width / 2, y: center.y - img.size.height / 2, width: img.size.width, height: img.size.height))
    }
    symbol("headphones", s * 0.34, .semibold, center: NSPoint(x: s / 2, y: s * 0.55), alpha: 1)
    symbol("waveform", s * 0.13, .bold, center: NSPoint(x: s / 2, y: s * 0.27), alpha: 0.85)

    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let dir = URL(fileURLWithPath: "target/AppIcon.iconset")
try? FileManager.default.removeItem(at: dir)
try! FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
for base in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = scale == 1 ? "icon_\(base)x\(base).png" : "icon_\(base)x\(base)@2x.png"
        try! render(base * scale).representation(using: .png, properties: [:])!.write(to: dir.appendingPathComponent(name))
    }
}
