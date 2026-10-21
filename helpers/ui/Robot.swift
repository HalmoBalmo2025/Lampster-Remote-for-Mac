// The Lampster as an icon: a small robot whose head is the lamp. Drawn as
// vectors, so it stays sharp at any size, and drawn on demand, so the
// body takes the menu bar's text colour in light and dark mode.

import AppKit

enum Robot {
    enum Look: Equatable {
        /// Not connected: the whole robot is faint.
        case away
        /// Connected, lamp off: the face is a hole.
        case dark
        /// The face shines in the lamp's colour.
        case lit(NSColor)
    }

    static func image(size: CGFloat, look: Look) -> NSImage {
        let image = NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            draw(in: rect, look: look)
            return true
        }
        image.accessibilityDescription = "Lampster Remote"
        return image
    }

    /// The app's icon: the robot with a red face on a tile in the warm
    /// light of the lamp, laid out on Apple's grid (the tile is 824 of 1024
    /// points, its corners about 22 % round), so macOS shows it as it is.
    static func appIcon(size: CGFloat) -> NSImage {
        NSImage(size: NSSize(width: size, height: size), flipped: false) { rect in
            let tile = rect.insetBy(dx: rect.width * 100 / 1024, dy: rect.height * 100 / 1024)
            let shape = NSBezierPath(roundedRect: tile, xRadius: tile.width * 0.225, yRadius: tile.width * 0.225)
            NSGradient(
                starting: NSColor(calibratedRed: 1.0, green: 0.86, blue: 0.62, alpha: 1),
                ending: NSColor(calibratedRed: 0.98, green: 0.62, blue: 0.36, alpha: 1)
            )?.draw(in: shape, angle: -90)
            draw(
                in: tile.insetBy(dx: tile.width * 0.14, dy: tile.width * 0.14),
                look: .lit(NSColor(calibratedRed: 0.93, green: 0.20, blue: 0.16, alpha: 1)),
                body: NSColor(white: 0.12, alpha: 1))
            return true
        }
    }

    /// `image` as a PNG of `pixels` × `pixels`; `nil` if AppKit cannot
    /// make the bitmap.
    static func png(_ image: NSImage, pixels: Int) -> Data? {
        guard let bitmap = NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0),
            let context = NSGraphicsContext(bitmapImageRep: bitmap)
        else { return nil }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        image.draw(in: NSRect(x: 0, y: 0, width: pixels, height: pixels))
        NSGraphicsContext.restoreGraphicsState()
        return bitmap.representation(using: .png, properties: [:])
    }

    /// Drawn on an 18 × 18 grid, origin bottom left, facing left like the
    /// real lamp. The body takes `body`: the menu bar's text colour for the
    /// menu bar, a fixed one for the app's icon.
    private static func draw(in rect: NSRect, look: Look, body bodyColor: NSColor = .labelColor) {
        let unit = rect.width / 18
        func box(_ x: CGFloat, _ y: CGFloat, _ width: CGFloat, _ height: CGFloat, _ radius: CGFloat)
            -> NSBezierPath
        {
            NSBezierPath(
                roundedRect: NSRect(
                    x: rect.minX + x * unit, y: rect.minY + y * unit, width: width * unit, height: height * unit),
                xRadius: radius * unit, yRadius: radius * unit)
        }

        let body = NSBezierPath()
        body.append(box(5.8, 0.6, 2.6, 4.4, 0.8))  // left leg
        body.append(box(9.6, 0.6, 2.6, 4.4, 0.8))  // right leg
        body.append(box(4.9, 0.6, 3.5, 1.5, 0.7))  // left foot
        body.append(box(8.7, 0.6, 3.5, 1.5, 0.7))  // right foot
        body.append(box(5.6, 4.7, 6.8, 4.9, 1.4))  // torso
        body.append(box(3.8, 4.6, 1.4, 4.6, 0.7))  // left arm
        body.append(box(12.8, 4.6, 1.4, 4.6, 0.7))  // right arm
        body.append(box(8.4, 9.2, 1.4, 1.2, 0.3))  // neck

        let faceCenter = NSPoint(x: rect.minX + 7.9 * unit, y: rect.minY + 13.5 * unit)
        let head = NSBezierPath(
            ovalIn: NSRect(x: rect.minX + 4.6 * unit, y: rect.minY + 9.8 * unit, width: 8.8 * unit, height: 8.0 * unit))
        let face = ellipse(center: faceCenter, width: 5.4 * unit, height: 6.0 * unit, degrees: -20)
        // A dark lamp's face is a hole; a lit one is drawn on the head.
        if look == .away || look == .dark {
            head.append(face)
            head.windingRule = .evenOdd
        }

        // Faint as a whole, so the overlapping parts do not show darker.
        let context = NSGraphicsContext.current?.cgContext
        if look == .away {
            context?.setAlpha(0.35)
            context?.beginTransparencyLayer(auxiliaryInfo: nil)
        }
        bodyColor.setFill()
        body.fill()
        head.fill()
        if look == .away {
            context?.endTransparencyLayer()
        }

        if case .lit(let color) = look {
            color.setFill()
            face.fill()
        }
    }

    private static func ellipse(center: NSPoint, width: CGFloat, height: CGFloat, degrees: CGFloat)
        -> NSBezierPath
    {
        let path = NSBezierPath(ovalIn: NSRect(x: -width / 2, y: -height / 2, width: width, height: height))
        let transform = NSAffineTransform()
        transform.translateX(by: center.x, yBy: center.y)
        transform.rotate(byDegrees: degrees)
        path.transform(using: transform as AffineTransform)
        return path
    }
}
