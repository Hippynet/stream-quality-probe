// Renders the app icon into an .iconset directory: an SF Symbol in white on a
// blue-to-teal rounded square. Usage: swift make-icon.swift OUT.iconset
import AppKit

let out = URL(fileURLWithPath: CommandLine.arguments[1])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px)
    // macOS icon grid: the shape fills ~80% of the canvas
    let inset = s * 0.1, rect = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let shape = NSBezierPath(roundedRect: rect, xRadius: rect.width * 0.225, yRadius: rect.width * 0.225)
    NSGradient(colors: [NSColor(red: 0.10, green: 0.36, blue: 0.95, alpha: 1),
                        NSColor(red: 0.05, green: 0.70, blue: 0.72, alpha: 1)])!
        .draw(in: shape, angle: -60)
    let config = NSImage.SymbolConfiguration(pointSize: rect.width * 0.5, weight: .semibold)
        .applying(.init(paletteColors: [.white]))
    let symbol = (NSImage(systemSymbolName: "waveform.badge.magnifyingglass", accessibilityDescription: nil)
        ?? NSImage(systemSymbolName: "waveform", accessibilityDescription: nil)!)
        .withSymbolConfiguration(config)!
    let sz = symbol.size, scale = min(rect.width * 0.62 / sz.width, rect.height * 0.62 / sz.height)
    let w = sz.width * scale, h = sz.height * scale
    symbol.draw(in: NSRect(x: rect.midX - w / 2, y: rect.midY - h / 2, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for base in [16, 32, 128, 256, 512] {
    try render(base).write(to: out.appending(path: "icon_\(base)x\(base).png"))
    try render(base * 2).write(to: out.appending(path: "icon_\(base)x\(base)@2x.png"))
}
