// Mirrors src/ui.rs, src/setting.rs, src/timer.rs and src/preferences.rs:
// the view comes as one JSON line on stdin, every change goes out as one
// JSON line on stdout.

import Foundation

enum Connection: String, Decodable {
    case searching, connecting, connected, unauthorized, unsupported
    case bluetoothOff = "bluetooth_off"
}

enum Lit: String, Decodable {
    case dark, white, color
}

enum Power: String, Codable {
    case on, off
}

enum Mode: String, Codable {
    case white, color
}

struct WhiteSetting: Codable {
    var tone = 0
}

struct ColorSetting: Codable {
    var hue = 0
    var saturation = 100
}

struct LampSettings: Decodable {
    var power = Power.off
    var mode = Mode.white
    /// One brightness for the white and the colour light.
    var brightness = 50
    var white = WhiteSetting()
    var color = ColorSetting()
}

enum Switch: String, Codable {
    case on, off
}

struct Options: Decodable {
    var openAtLogin = Switch.off
    var offWithMac = Switch.off
}

/// A timer; one that is off, and that the lamp knows is off, is nil.
enum TimerView: Decodable {
    /// In the lamp, which fires it at the date.
    case inLamp(Date)
    /// Set for the date; the lamp gets it once it is connected.
    case pending(Date)
    /// Switched off; until it is connected, the lamp still fires it.
    case cancelling

    private enum Keys: String, CodingKey {
        case state, at
    }

    init(from decoder: Decoder) throws {
        let fields = try decoder.container(keyedBy: Keys.self)
        let state = try fields.decode(String.self, forKey: .state)
        // `at` is seconds since 1970.
        func at() throws -> Date {
            Date(timeIntervalSince1970: TimeInterval(try fields.decode(Int64.self, forKey: .at)))
        }
        switch state {
        case "in_lamp": self = .inLamp(try at())
        case "pending": self = .pending(try at())
        case "cancelling": self = .cancelling
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .state, in: fields, debugDescription: "unknown timer state \(state)")
        }
    }

    /// When it fires; nil while it is being cancelled.
    var date: Date? {
        switch self {
        case .inLamp(let date), .pending(let date): date
        case .cancelling: nil
        }
    }
}

struct TimersView: Decodable {
    var wake: TimerView?
    var sleep: TimerView?
}

struct LampView: Decodable {
    var connection = Connection.searching
    var lit = Lit.dark
    /// Hundredths of a degree Celsius, inside the lamp.
    var temperature: Int?
    var settings = LampSettings()
    var options = Options()
    var timers = TimersView()
    var problem: String?
    /// How many lines of the panel the core has read.
    var commands: UInt64 = 0
}

struct ClockTime: Encodable, Equatable {
    let hour: Int
    let minute: Int
}

/// What the panel asks of the core: `Command` in src/ui.rs.
enum Command: Encodable {
    case power(Power)
    case mode(Mode)
    case white(WhiteSetting, brightness: Int)
    case color(ColorSetting, brightness: Int)
    case quit, shutdown, away
    case openAtLogin(Switch)
    case offWithMac(Switch)
    case wake(at: ClockTime)
    case wakeOff
    case sleep(minutes: Int)
    case sleepOff
    case log(String)

    private enum Key: CodingKey {
        case command, power, mode, white, color, brightness, openAtLogin, offWithMac, at, minutes, message
    }

    private var name: String {
        switch self {
        case .power: "power"
        case .mode: "mode"
        case .white: "white"
        case .color: "color"
        case .quit: "quit"
        case .shutdown: "shutdown"
        case .away: "away"
        case .openAtLogin: "open_at_login"
        case .offWithMac: "off_with_mac"
        case .wake: "wake"
        case .wakeOff: "wake_off"
        case .sleep: "sleep"
        case .sleepOff: "sleep_off"
        case .log: "log"
        }
    }

    func encode(to encoder: Encoder) throws {
        var fields = encoder.container(keyedBy: Key.self)
        try fields.encode(name, forKey: .command)
        switch self {
        case .power(let power):
            try fields.encode(power, forKey: .power)
        case .mode(let mode):
            try fields.encode(mode, forKey: .mode)
        case .white(let white, let brightness):
            try fields.encode(white, forKey: .white)
            try fields.encode(brightness, forKey: .brightness)
        case .color(let color, let brightness):
            try fields.encode(color, forKey: .color)
            try fields.encode(brightness, forKey: .brightness)
        case .openAtLogin(let open):
            try fields.encode(open, forKey: .openAtLogin)
        case .offWithMac(let off):
            try fields.encode(off, forKey: .offWithMac)
        case .wake(let at):
            try fields.encode(at, forKey: .at)
        case .sleep(let minutes):
            try fields.encode(minutes, forKey: .minutes)
        case .log(let message):
            try fields.encode(message, forKey: .message)
        case .quit, .shutdown, .away, .wakeOff, .sleepOff:
            break
        }
    }
}

/// Lines written to the core; a view that has read fewer was sent before
/// the panel's latest change (`LampView.commands`). Only the main thread
/// sends.
var commandsSent: UInt64 = 0

/// The keys go out in snake case, as the core reads them.
func commandEncoder() -> JSONEncoder {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    return encoder
}

/// A command that cannot be encoded is not sent, nor counted.
func send(_ command: Command) {
    guard var data = try? commandEncoder().encode(command) else { return }
    data.append(0x0A)
    FileHandle.standardOutput.write(data)
    commandsSent += 1
}

/// What went wrong in decoding, by field names only: the error's own text
/// can repeat the values of the line, which the log must not hold.
func describe(_ error: Error) -> String {
    func path(_ context: DecodingError.Context) -> String {
        context.codingPath.map(\.stringValue).joined(separator: ".")
    }
    switch error {
    case DecodingError.typeMismatch(_, let context): return "wrong type at \(path(context))"
    case DecodingError.valueNotFound(_, let context): return "missing value at \(path(context))"
    case DecodingError.keyNotFound(let key, let context):
        return "missing \(key.stringValue) at \(path(context))"
    case DecodingError.dataCorrupted(let context): return "unreadable at \(path(context))"
    default: return "not JSON"
    }
}
