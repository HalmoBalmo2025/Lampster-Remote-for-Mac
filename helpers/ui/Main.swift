// lampster-ui: menu bar icon and panel of Lampster Remote. The Rust core
// starts it, sends the view as JSON lines on stdin and reads the commands
// from stdout (see src/ui.rs). It ends when stdin ends.
//
// `--snapshot <file.png> [white|color|timers|settings]` renders the panel with example
// values into a picture, to check the layout without the lamp.
// `--icon <folder.iconset>` writes the app's icon in every size macOS asks
// for, which scripts/bundle.sh turns into AppIcon.icns.

import AppKit
import SwiftUI

@main
enum Main {
    static func main() {
        let arguments = CommandLine.arguments
        if arguments.count >= 3, arguments[1] == "--icon" {
            writeIcons(into: arguments[2])
            return
        }
        if arguments.count >= 3, arguments[1] == "--snapshot" {
            let mode = arguments.count >= 4 ? Mode(rawValue: arguments[3]) ?? .white : .white
            let page: Page = switch arguments.count >= 4 ? arguments[3] : "" {
            case "settings": .settings
            case "timers": .timers
            default: .lamp
            }
            MainActor.assumeIsolated { snapshot(into: arguments[2], mode: mode, page: page) }
            return
        }
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        let delegate = AppDelegate()
        app.delegate = delegate
        app.run()
    }
}

/// The files of an iconset: each size at 1× and 2×.
func writeIcons(into folder: String) {
    let directory = URL(fileURLWithPath: folder)
    do {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        for points in [16, 32, 128, 256, 512] {
            for scale in [1, 2] {
                let name = scale == 1 ? "icon_\(points)x\(points).png" : "icon_\(points)x\(points)@2x.png"
                let pixels = points * scale
                guard let png = Robot.png(Robot.appIcon(size: CGFloat(pixels)), pixels: pixels) else {
                    FileHandle.standardError.write(Data("cannot draw the icon at \(pixels) pixels\n".utf8))
                    exit(1)
                }
                try png.write(to: directory.appendingPathComponent(name))
            }
        }
    } catch {
        FileHandle.standardError.write(Data("cannot write the icons to \(folder): \(error)\n".utf8))
        exit(1)
    }
}

@MainActor
func snapshot(into path: String, mode: Mode, page: Page) {
    let model = Model()
    model.page = page
    var view = LampView()
    view.connection = .connected
    view.lit = mode == .white ? .white : .color
    view.temperature = 4120
    view.settings.power = .on
    view.settings.mode = mode
    view.settings.brightness = 70
    view.settings.white = WhiteSetting(tone: 30)
    view.settings.color = ColorSetting(hue: 200, saturation: 80)
    view.timers.wake = .inLamp(Date().addingTimeInterval(9 * 3600 + 12 * 60))
    view.timers.sleep = .pending(Date().addingTimeInterval(27 * 60))
    model.apply(view)
    // A window of its own, never shown: `ImageRenderer` draws AppKit
    // controls (fields, switches) only as placeholders. Its switches look
    // inactive, as the window never becomes key.
    _ = NSApplication.shared
    let hosting = NSHostingView(rootView: Panel(model: model).frame(width: 320))
    hosting.frame = NSRect(origin: .zero, size: NSSize(width: 320, height: hosting.fittingSize.height))
    let window = NSWindow(contentRect: hosting.frame, styleMask: [.borderless], backing: .buffered, defer: false)
    window.contentView = hosting
    hosting.layoutSubtreeIfNeeded()
    guard let bitmap = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else {
        FileHandle.standardError.write(Data("cannot render the panel\n".utf8))
        exit(1)
    }
    hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        FileHandle.standardError.write(Data("cannot render the panel\n".utf8))
        exit(1)
    }
    do {
        try png.write(to: URL(fileURLWithPath: path))
    } catch {
        FileHandle.standardError.write(Data("cannot write \(path): \(error)\n".utf8))
        exit(1)
    }
}
