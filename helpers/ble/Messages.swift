// The messages of lampster-ble, mirroring src/ble.rs: one request per line
// on stdin, one event per line on stdout. fixtures/ble/ holds a line of
// each, which the tests of both sides read.

import Foundation

struct Request: Decodable {
    enum Kind: String, Decodable { case connect, read, write, disconnect }
    enum Delivery: String, Decodable { case confirmed, unconfirmed }

    let request: Kind
    let id: UInt64?
    let advertised: String?
    let services: [String]?
    let subscribe: [String]?
    let characteristic: String?
    let value: [UInt8]?
    let delivery: Delivery?
}

/// The state of the Mac's Bluetooth: `Radio` in src/ble.rs.
enum Radio: String {
    case on, off, unauthorized, unsupported, unknown
}

/// What the helper tells the core: `Event` in src/ble.rs.
enum Event {
    case bluetooth(state: Radio)
    case connected(id: UInt64)
    case value(id: UInt64, value: [UInt8])
    case changed(characteristic: String, value: [UInt8])
    case written(id: UInt64)
    case rejected(id: UInt64)
    /// `id` nil when the failure answers no request.
    case failed(id: UInt64?, reason: String)
    case unsubscribed(characteristic: String, reason: String)
    case disconnected(reason: String?)

    /// The JSON object of the line; nil goes out as null.
    var object: [String: Any] {
        func orNull(_ value: Any?) -> Any { value ?? NSNull() }
        switch self {
        case .bluetooth(let state): return ["event": "bluetooth", "state": state.rawValue]
        case .connected(let id): return ["event": "connected", "id": id]
        case .value(let id, let value): return ["event": "value", "id": id, "value": value]
        case .changed(let characteristic, let value):
            return ["event": "changed", "characteristic": characteristic, "value": value]
        case .written(let id): return ["event": "written", "id": id]
        case .rejected(let id): return ["event": "rejected", "id": id]
        case .failed(let id, let reason): return ["event": "failed", "id": orNull(id), "reason": reason]
        case .unsubscribed(let characteristic, let reason):
            return ["event": "unsubscribed", "characteristic": characteristic, "reason": reason]
        case .disconnected(let reason): return ["event": "disconnected", "reason": orNull(reason)]
        }
    }

    /// One line, without the newline; the keys sorted, so it is the same
    /// every time.
    var line: Data? { try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) }
}

func emit(_ event: Event) {
    guard var data = event.line else { return }
    data.append(0x0A)
    FileHandle.standardOutput.write(data)
}
