import AppKit
import SwiftUI

@MainActor
final class Model: ObservableObject {
    @Published var view = LampView()
    // Slider positions live here because `apply` sets them from the core's
    // view; besides, `@State` is a macro that swiftc without Xcode cannot
    // expand.
    @Published var tone = 0.0
    @Published var brightness = 50.0
    @Published var hue = 0.0
    @Published var saturation = 100.0
    /// While a slider is dragged, a view from Lampster Remote must not
    /// move it back.
    var dragging = false
    @Published var page = Page.lamp
    /// The light the panel shows; set at once on a click, and taken from
    /// the core's views only once one has read the click, so the panel
    /// never shows the old light in between.
    @Published var mode = Mode.white
    /// What is typed into the sleep timer's field: the minutes chosen, not
    /// those left.
    @Published var sleepField: String
    /// What is typed into the wake-up field. A view from the core sets it
    /// only when its wake-up time changes, so it never overwrites typing.
    @Published var wakeField: String
    /// Per field, the text that holds already: set by the core, sent, or
    /// dealt with by the switch. Leaving a field sends only a time or
    /// minutes that differ from what it means.
    private var settled: FieldTexts

    private struct FieldTexts {
        var wake: String
        var sleep: String
    }

    init() {
        let first = FieldTexts(wake: Model.firstWakeTime.text(), sleep: String(Model.firstSleepMinutes))
        wakeField = first.wake
        sleepField = first.sleep
        settled = first
    }

    /// What the fields hold before anything is set; also their placeholders.
    nonisolated static let firstWakeTime = ClockTime(hour: 7, minute: 0)
    nonisolated static let firstSleepMinutes = 30

    var isConnected: Bool { view.connection == .connected }
    /// The toggle: on only while connected and switched on.
    var isOn: Bool { isConnected && view.settings.power == .on }

    func apply(_ view: LampView) {
        let shownWake = self.view.timers.wake?.date
        // A view sent before the core read the panel's latest change: what
        // the user set stays until a later view confirms or corrects it.
        guard view.commands >= commandsSent else {
            var current = view
            current.settings = self.view.settings
            current.options = self.view.options
            current.timers = self.view.timers
            self.view = current
            return
        }
        self.view = view
        mode = view.settings.mode
        if let date = view.timers.wake?.date, date != shownWake {
            wakeField = ClockTime(of: date).text()
            settled.wake = wakeField
        }
        guard !dragging else { return }
        let settings = view.settings
        tone = Double(settings.white.tone)
        brightness = Double(settings.brightness)
        hue = Double(settings.color.hue)
        saturation = Double(settings.color.saturation)
    }

    func setDragging(_ dragging: Bool) {
        self.dragging = dragging
    }

    /// Ends a drag whose end was never reported; the sliders show the
    /// lamp again.
    func stopDragging() {
        guard dragging else { return }
        dragging = false
        apply(view)
    }

    func setPower(_ on: Bool) {
        send(.power(on ? .on : .off))
    }

    func togglePower() { setPower(!isOn) }

    /// Without a connection only the page changes: a mode sent then would
    /// override what the lamp does once it connects again.
    func setMode(_ mode: Mode) {
        page = .lamp
        guard isConnected else { return }
        self.mode = mode
        send(.mode(mode))
    }

    var wakeIsOn: Bool { view.timers.wake?.date != nil }

    /// `nil` while the field holds no time.
    var typedWakeTime: ClockTime? { ClockTime(typed: wakeField) }

    /// Switching on needs a time in the field; without one nothing is
    /// sent, and the panel says why under the field.
    func setWake(_ on: Bool) {
        if on {
            guard let time = typedWakeTime else { return }
            send(.wake(at: time))
        } else {
            send(.wakeOff)
        }
        settled.wake = wakeField
    }

    /// The minutes the sleep timer takes; `Minutes` in src/timer.rs takes
    /// the same, which a test there checks.
    nonisolated static let sleepMinutes = 1...120

    /// The minutes in `text`: digits only, within `sleepMinutes`.
    nonisolated static func sleepMinutes(in text: String) -> Int? {
        let digits = text.trimmingCharacters(in: .whitespaces)
        guard !digits.isEmpty, digits.allSatisfy({ $0.isASCII && $0.isNumber }),
              let minutes = Int(digits), sleepMinutes.contains(minutes)
        else { return nil }
        return minutes
    }

    /// `nil` while the field holds no time the timer takes.
    var typedSleepMinutes: Int? { Model.sleepMinutes(in: sleepField) }

    var sleepIsOn: Bool { view.timers.sleep?.date != nil }

    /// Switching on starts the timer with the minutes in the field, also
    /// again while it runs; without them nothing is sent, and the panel
    /// says why under the field.
    func setSleep(_ on: Bool) {
        if on {
            guard let minutes = typedSleepMinutes else { return }
            send(.sleep(minutes: minutes))
        } else {
            send(.sleepOff)
        }
        settled.sleep = sleepField
    }

    /// The cursor leaves a field, or the page or the panel goes away: a
    /// changed time takes effect while its timer is on, as on Return;
    /// switched off, it waits for the switch.
    func leave(_ field: TimerField) {
        switch field {
        case .wake:
            // The same time written otherwise ("07.30" for "7:30") is no change.
            guard typedWakeTime != ClockTime(typed: settled.wake) else { return }
            if wakeIsOn { setWake(true) } else { settled.wake = wakeField }
        case .sleep:
            guard typedSleepMinutes != Model.sleepMinutes(in: settled.sleep) else { return }
            if sleepIsOn { setSleep(true) } else { settled.sleep = sleepField }
        }
    }

    /// The timers page or the panel closes, with the cursor maybe still in
    /// a field.
    func leaveBoth() {
        leave(.wake)
        leave(.sleep)
    }

    /// Return sets the timer, also one that is off.
    func submit(_ field: TimerField) {
        switch field {
        case .wake: setWake(true)
        case .sleep: setSleep(true)
        }
    }

    func setOpenAtLogin(_ on: Bool) {
        send(.openAtLogin(on ? .on : .off))
    }

    func setOffWithMac(_ on: Bool) {
        send(.offWithMac(on ? .on : .off))
    }

    var brightnessValue: Int { max(1, Int(brightness.rounded())) }

    func sendWhite() {
        send(.white(WhiteSetting(tone: Int(tone.rounded())), brightness: brightnessValue))
    }

    func sendColor() {
        let color = ColorSetting(hue: min(359, Int(hue.rounded())), saturation: Int(saturation.rounded()))
        send(.color(color, brightness: brightnessValue))
    }

    /// The robot's face shows the colour the lamp really shines in, at
    /// full brightness so it stays visible in the menu bar.
    var robotLook: Robot.Look {
        guard isConnected else { return .away }
        let settings = view.settings
        switch view.lit {
        case .dark:
            return .dark
        case .white:
            let cool = Double(settings.white.tone) / 100
            return .lit(NSColor(
                red: 1 - 0.14 * cool, green: 0.78 + 0.14 * cool, blue: 0.52 + 0.48 * cool, alpha: 1))
        case .color:
            return .lit(NSColor(
                hue: Double(settings.color.hue) / 360, saturation: Double(settings.color.saturation) / 100,
                brightness: 1, alpha: 1))
        }
    }

    var temperatureText: String? {
        view.temperature.map { "\(Int((Double($0) / 100).rounded())) °C" }
    }

    var status: String {
        switch view.connection {
        case .searching: "Looking for Lampster…"
        case .connecting: "Connecting…"
        case .connected: view.lit == .dark ? "Off" : "On"
        case .bluetoothOff: "Bluetooth is off"
        case .unauthorized: "No access to Bluetooth"
        case .unsupported: "This Mac has no Bluetooth LE"
        }
    }

    var hint: Hint? {
        switch view.connection {
        case .searching:
            Hint(text: "Is Lampster switched on at the plug and nearby?", button: nil)
        case .bluetoothOff:
            Hint(
                text: "Switch Bluetooth on to control Lampster.",
                button: ("Bluetooth Settings…", "x-apple.systempreferences:com.apple.BluetoothSettings"))
        case .unauthorized:
            Hint(
                text: "Allow Lampster Remote to use Bluetooth in System Settings.",
                button: ("Privacy Settings…",
                         "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"))
        case .connecting, .connected, .unsupported:
            nil
        }
    }
}

enum Page {
    case lamp, timers, settings
}

/// The fields of the timers page.
enum TimerField {
    case wake, sleep
}

struct Hint {
    let text: String
    /// Title and the System Settings URL it opens.
    let button: (title: String, url: String)?
}
