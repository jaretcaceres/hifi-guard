// Renders the menu bar template icons to raw 36x36 RGBA (assets/tray-*.rgba). Run: swift tools/make-tray-icons.swift
import AppKit

func render(_ name: String, alpha: CGFloat, to path: String) {
    let px = 36
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bitmapFormat: [], bytesPerRow: px * 4, bitsPerPixel: 32)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let config = NSImage.SymbolConfiguration(pointSize: 26, weight: .medium)
        .applying(.init(paletteColors: [NSColor.black.withAlphaComponent(alpha)]))
    let img = NSImage(systemSymbolName: name, accessibilityDescription: nil)!.withSymbolConfiguration(config)!
    let s = img.size
    let scale = min(32 / s.width, 32 / s.height)
    let w = s.width * scale, h = s.height * scale
    img.draw(in: NSRect(x: (36 - w) / 2, y: (36 - h) / 2, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    try! Data(bytes: rep.bitmapData!, count: px * px * 4).write(to: URL(fileURLWithPath: path))
}

render("headphones", alpha: 1, to: "assets/tray-ok.rgba")
render("exclamationmark.triangle.fill", alpha: 1, to: "assets/tray-warn.rgba")
render("headphones", alpha: 0.4, to: "assets/tray-off.rgba")
