// Regenerate the original vector icon with: swift scripts/generate-icon.swift assets/icon.png
import AppKit

let size = 1024
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
let frame = NSRect(x: 48, y: 48, width: 928, height: 928)
NSColor(calibratedRed: 0.08, green: 0.26, blue: 0.36, alpha: 1).setFill()
NSBezierPath(roundedRect: frame, xRadius: 210, yRadius: 210).fill()
NSColor(calibratedRed: 0.36, green: 0.86, blue: 0.77, alpha: 1).setFill()
// A cloud above the local machine: one continuous, original geometric mark.
let cloud = NSBezierPath()
cloud.move(to: NSPoint(x: 295, y: 510))
cloud.curve(to: NSPoint(x: 292, y: 730), controlPoint1: NSPoint(x: 148, y: 510), controlPoint2: NSPoint(x: 148, y: 730))
cloud.curve(to: NSPoint(x: 610, y: 770), controlPoint1: NSPoint(x: 315, y: 910), controlPoint2: NSPoint(x: 558, y: 934))
cloud.curve(to: NSPoint(x: 748, y: 510), controlPoint1: NSPoint(x: 824, y: 826), controlPoint2: NSPoint(x: 891, y: 510))
cloud.close()
cloud.fill()
NSColor.white.setFill()
NSBezierPath(roundedRect: NSRect(x: 276, y: 220, width: 472, height: 238), xRadius: 40, yRadius: 40).fill()
NSColor(calibratedRed: 0.08, green: 0.26, blue: 0.36, alpha: 1).setFill()
NSBezierPath(roundedRect: NSRect(x: 314, y: 270, width: 396, height: 150), xRadius: 15, yRadius: 15).fill()
NSColor.white.setFill()
NSBezierPath(roundedRect: NSRect(x: 480, y: 148, width: 64, height: 100), xRadius: 15, yRadius: 15).fill()
NSBezierPath(roundedRect: NSRect(x: 400, y: 140, width: 224, height: 32), xRadius: 16, yRadius: 16).fill()
image.unlockFocus()
guard let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else { fatalError("icon_render_failed") }
try png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
