// Asset generator only; the application remains Rust.
import AppKit

let output = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent()
func rounded(_ r: NSRect, _ radius: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: r, xRadius: radius, yRadius: radius)
}
let frontHead = NSRect(x: 2.5, y: 1.5, width: 15, height: 13)
let rearHead = NSRect(x: 12.5, y: 8.5, width: 13.2, height: 12)
func eyeDots(_ frame: NSRect, left: Bool = true, right: Bool = true, height: CGFloat = 3.6) {
    for (index, fraction) in [CGFloat(0.33), CGFloat(0.67)].enumerated() {
        if (index == 0 && left) || (index == 1 && right) {
            let x = frame.minX + frame.width * fraction
            NSBezierPath(ovalIn: NSRect(x: x - 1.8, y: frame.midY - height / 2, width: 3.6, height: height)).fill()
        }
    }
}
func rearClip() {
    let clip = NSBezierPath(rect: NSRect(x: -2, y: -2, width: 34, height: 26))
    clip.windingRule = .evenOdd
    clip.append(rounded(frontHead.insetBy(dx: -1.4, dy: -1.4), 5.1))
    clip.addClip()
}
func eyeMask(left: Bool, right: Bool, color: NSColor) {
    NSGraphicsContext.saveGraphicsState()
    let offset = NSAffineTransform(); offset.translateX(by: 1, yBy: 0); offset.concat()
    color.set()
    NSGraphicsContext.saveGraphicsState()
    rearClip()
    eyeDots(rearHead, left: left, right: right)
    NSGraphicsContext.restoreGraphicsState()
    eyeDots(frontHead, left: left, right: right)
    NSGraphicsContext.restoreGraphicsState()
}
func robots(color: NSColor, hiddenEyes: Bool = false) {
    NSGraphicsContext.saveGraphicsState()
    let offset = NSAffineTransform(); offset.translateX(by: 1, yBy: 0); offset.concat()
    color.set()
    // Reserve a full ear width outside the face.
    let front = frontHead
    let rear = rearHead
    func head(_ frame: NSRect) {
        let shape = rounded(frame.insetBy(dx: -1.1, dy: -1.1), 4.8)
        shape.windingRule = .evenOdd
        shape.append(NSBezierPath(roundedRect: frame.insetBy(dx: 0.5, dy: 1.9), xRadius: 3.2, yRadius: 3.2))
        shape.fill()
        if !hiddenEyes { eyeDots(frame) }
    }
    // Hide the rear silhouette where it passes behind the front robot.
    NSGraphicsContext.saveGraphicsState()
    rearClip()
    head(rear)
    rounded(NSRect(x: rear.maxX - 0.1, y: 11.4, width: 3.0, height: 6.0), 1.3).fill()
    NSGraphicsContext.restoreGraphicsState()
    color.set()
    head(front)
    rounded(NSRect(x: -0.6, y: 4.9, width: 3.2, height: 6.2), 1.3).fill()
    rounded(NSRect(x: front.maxX - 0.3, y: 4.9, width: 3.2, height: 6.2), 1.3).fill()
    NSGraphicsContext.restoreGraphicsState()
}
final class RobotView: NSView {
    var hiddenEyes = false
    override func draw(_ dirtyRect: NSRect) { robots(color: .black, hiddenEyes: hiddenEyes) }
}
final class EyesView: NSView {
    var left = true
    var right = false
    override func draw(_ dirtyRect: NSRect) { eyeMask(left: left, right: right, color: .black) }
}
for (name, left, right) in [("left", true, false), ("both", true, true), ("right", false, true)] {
    let view = EyesView(frame: NSRect(x: 0, y: 0, width: 30, height: 22))
    view.left = left; view.right = right
    try view.dataWithPDF(inside: view.bounds).write(to: output.appendingPathComponent("eyes-" + name + ".pdf"))
}
func png(_ image: NSImage, _ name: String) throws {
    let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
    try rep.representation(using: .png, properties: [:])!.write(to: output.appendingPathComponent(name))
}
let view = RobotView(frame: NSRect(x: 0, y: 0, width: 30, height: 22))
try view.dataWithPDF(inside: view.bounds).write(to: output.appendingPathComponent("robots-template.pdf"))
let activeView = RobotView(frame: NSRect(x: 0, y: 0, width: 30, height: 22))
activeView.hiddenEyes = true
try activeView.dataWithPDF(inside: activeView.bounds).write(to: output.appendingPathComponent("robots-working.pdf"))
let image = NSImage(size: NSSize(width: 120, height: 88))
image.lockFocus()
let transform = NSAffineTransform(); transform.scale(by: 4); transform.concat()
robots(color: .black)
image.unlockFocus()
try png(image, "robots-template.png")
let preview = NSImage(size: NSSize(width: 440, height: 360))
preview.lockFocus()
NSColor(calibratedWhite: 0.96, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: 440, height: 360).fill()
func text(_ value: String, _ x: CGFloat, _ y: CGFloat, _ color: NSColor, _ size: CGFloat = 14) {
    (value as NSString).draw(at: NSPoint(x: x, y: y), withAttributes: [.font: NSFont.systemFont(ofSize: size), .foregroundColor: color])
}
do {
    let x: CGFloat = 12
    text("Outline · 1.6 / 3 pt", x + 12, 328, .darkGray, 18)
    for (row, dark) in [false, true].enumerated() {
        let y: CGFloat = row == 0 ? 172 : 16
        (dark ? NSColor(calibratedWhite: 0.15, alpha: 1) : NSColor(calibratedWhite: 0.88, alpha: 1)).setFill()
        rounded(NSRect(x: x, y: y, width: 416, height: 142), 14).fill()
        let color: NSColor = dark ? .white : .black
        NSGraphicsContext.saveGraphicsState()
        let transform = NSAffineTransform(); transform.translateX(by: x + 28, yBy: y + 32); transform.scale(by: 3.5); transform.concat()
        robots(color: color)
        NSGraphicsContext.restoreGraphicsState()
        NSGraphicsContext.saveGraphicsState()
        let transform2 = NSAffineTransform(); transform2.translateX(by: x + 192, yBy: y + 60); transform2.concat()
        robots(color: color)
        NSGraphicsContext.restoreGraphicsState()
        text("30 × 22 pt", x + 240, y + 63, color)
    }
}
preview.unlockFocus()
try png(preview, "preview.png")

let workingPreview = NSImage(size: NSSize(width: 960, height: 140))
workingPreview.lockFocus()
NSColor(calibratedWhite: 0.15, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: 960, height: 140).fill()
let levels: [(CGFloat, CGFloat)] = [(0.5, 0), (1, 0), (1, 0.5), (1, 1), (0.5, 1), (0, 1), (0, 0.5), (0, 0)]
for (phase, level) in levels.enumerated() {
    let x = CGFloat(phase) * 120 + 25
    NSGraphicsContext.saveGraphicsState()
    let transform = NSAffineTransform(); transform.translateX(by: x, yBy: 55); transform.scale(by: 2); transform.concat()
    robots(color: .white, hiddenEyes: true)
    eyeMask(left: true, right: false, color: NSColor.systemGreen.withAlphaComponent(level.0))
    eyeMask(left: false, right: true, color: NSColor.systemGreen.withAlphaComponent(level.1))
    NSGraphicsContext.restoreGraphicsState()
    text("\(Int(level.0 * 100)) / \(Int(level.1 * 100)) %", x - 5, 20, .white, 12)
}
workingPreview.unlockFocus()
try png(workingPreview, "preview-working.png")
