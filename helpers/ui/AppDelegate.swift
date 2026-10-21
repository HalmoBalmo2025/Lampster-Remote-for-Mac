import AppKit
import SwiftUI

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSPopoverDelegate {
    let model = Model()
    var item: NSStatusItem?
    let popover = NSPopover()
    /// Closes the panel on a click outside it. "Transient" alone does not,
    /// because an app without a Dock icon is not active while the user
    /// clicks elsewhere.
    var outsideClicks: Any?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.target = self
        item.button?.action = #selector(clicked)
        item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
        item.button?.imagePosition = .imageOnly
        self.item = item

        popover.behavior = .transient
        popover.delegate = self
        let hosting = NSHostingController(rootView: Panel(model: model).frame(width: 320))
        hosting.sizingOptions = [.preferredContentSize]
        popover.contentViewController = hosting
        updateButton()
        startReading()
        watchTheMac()
    }

    /// Left click opens the panel; right click (or Control-click) switches
    /// the lamp on or off.
    @objc func clicked() {
        let event = NSApp.currentEvent
        let secondary = event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true
        if secondary && model.isConnected {
            model.togglePower()
        } else {
            toggle()
        }
    }

    func toggle() {
        guard let button = item?.button else { return }
        if popover.isShown {
            popover.performClose(nil)
        } else {
            popover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)
            NSApp.activate(ignoringOtherApps: true)
            let window = popover.contentViewController?.view.window
            window?.makeKey()
            // No field is typed into until it is clicked.
            window?.makeFirstResponder(nil)
        }
    }

    func popoverDidShow(_ notification: Notification) {
        outsideClicks = NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDown, .rightMouseDown]) {
            [weak self] _ in
            Task { @MainActor in self?.popover.performClose(nil) }
        }
    }

    /// A time typed and not yet set takes effect now, not when the panel
    /// opens again and the field loses the cursor then.
    func popoverWillClose(_ notification: Notification) {
        model.leaveBoth()
        popover.contentViewController?.view.window?.makeFirstResponder(nil)
    }

    func popoverDidClose(_ notification: Notification) {
        if let outsideClicks { NSEvent.removeMonitor(outsideClicks) }
        outsideClicks = nil
        model.stopDragging()
    }

    func updateButton() {
        item?.button?.image = Robot.image(size: 18, look: model.robotLook)
        item?.button?.toolTip = "Lampster Remote: \(model.status)"
    }

    /// Reads the views from stdin; when stdin ends, Lampster Remote has
    /// ended and so does the panel. The main queue keeps the views in
    /// their order, which tasks do not.
    func startReading() {
        Thread.detachNewThread { [weak self] in
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            while let line = readLine() {
                do {
                    let view = try decoder.decode(LampView.self, from: Data(line.utf8))
                    DispatchQueue.main.async {
                        self?.model.apply(view)
                        self?.updateButton()
                    }
                } catch {
                    let message = "unreadable view (\(line.count) characters): \(describe(error))"
                    DispatchQueue.main.async { send(.log(message)) }
                }
            }
            DispatchQueue.main.async {
                guard let self else { return NSApp.terminate(nil) }
                self.coreEnded = true
                if self.terminating {
                    NSApp.reply(toApplicationShouldTerminate: true)
                } else {
                    NSApp.terminate(nil)
                }
            }
        }
    }

    /// Lid closed, sleep from the menu, or a locked screen: the core may
    /// switch the lamp off.
    func watchTheMac() {
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.willSleepNotification, object: nil, queue: .main
        ) { _ in send(.away) }
        DistributedNotificationCenter.default().addObserver(
            forName: NSNotification.Name("com.apple.screenIsLocked"), object: nil, queue: .main
        ) { _ in send(.away) }
    }

    /// Set once Lampster Remote's core has ended (stdin closed).
    var coreEnded = false
    var terminating = false

    /// How long quitting waits for the core at most; longer than
    /// `QUIT_GRACE` in src/controller/mod.rs, which a test checks.
    static let quitWait: TimeInterval = 5

    /// macOS asks the panel to quit when the Mac shuts down, restarts or
    /// logs out. The core may switch the lamp off first, so quitting waits
    /// until the core has ended, at most `quitWait`.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if coreEnded { return .terminateNow }
        guard !terminating else { return .terminateLater }
        terminating = true
        send(.shutdown)
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.quitWait) {
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}
