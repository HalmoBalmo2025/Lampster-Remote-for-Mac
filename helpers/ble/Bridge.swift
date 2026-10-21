// lampster-ble: the Bluetooth side of Lampster Remote (CoreBluetooth).
//
// One request per line on stdin, one event per line on stdout (see
// Messages.swift and src/ble.rs). It knows nothing about the lamp; what to
// look for and what to write comes from the Rust core. It ends when stdin
// ends. For experiments it can be driven by hand, see the README.

import CoreBluetooth
import Foundation

struct PendingWrite {
    let id: UInt64
    let characteristic: CBCharacteristic
    let value: Data
}

struct Connecting {
    let id: UInt64
    let advertised: CBUUID
    let services: [CBUUID]
    let subscribe: [CBUUID]
}

/// Where the peripheral being connected came from.
enum Origin {
    /// Known from an earlier connection or still held by macOS; connecting
    /// to it waits for it to show up, which can go stale.
    case remembered
    /// Seen just now by the scan.
    case scanned
}

final class Bridge: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    var connecting: Connecting?
    var peripheral: CBPeripheral?
    var origin = Origin.scanned
    /// The lamp of the last connection, to ask CoreBluetooth for it
    /// directly instead of only waiting for its advertisement.
    var lastIdentifier: UUID?
    var characteristics: [CBUUID: CBCharacteristic] = [:]
    var servicesLeft = 0
    var reads: [CBUUID: UInt64] = [:]
    var writes: [CBUUID: UInt64] = [:]
    var pendingWrite: PendingWrite?

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: nil)
    }

    func handle(_ request: Request) {
        switch request.request {
        case .connect: connect(request)
        case .read: read(request)
        case .write: write(request)
        case .disconnect: disconnect()
        }
    }

    func connect(_ request: Request) {
        guard let id = request.id, let advertised = request.advertised else {
            return emit(.failed(id: request.id, reason: "connect needs id and advertised"))
        }
        let next = Connecting(
            id: id, advertised: CBUUID(string: advertised),
            services: (request.services ?? []).map { CBUUID(string: $0) },
            subscribe: (request.subscribe ?? []).map { CBUUID(string: $0) })
        disconnect()
        connecting = next
        search()
    }

    /// Looks for the lamp in three ways at once: connected to macOS
    /// already, known from the last connection, and by scanning.
    func search() {
        guard let connecting, central.state == .poweredOn, peripheral == nil else { return }
        let held = central.retrieveConnectedPeripherals(withServices: connecting.services).first
        let known = lastIdentifier.flatMap { central.retrievePeripherals(withIdentifiers: [$0]).first }
        if let remembered = held ?? known {
            attach(remembered, from: .remembered)
        }
        scan(for: connecting.advertised)
    }

    func scan(for advertised: CBUUID) {
        if central.isScanning { central.stopScan() }
        central.scanForPeripherals(
            withServices: [advertised], options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
    }

    func attach(_ found: CBPeripheral, from origin: Origin) {
        if let peripheral, peripheral != found { central.cancelPeripheralConnection(peripheral) }
        peripheral = found
        self.origin = origin
        found.delegate = self
        central.connect(found)
    }

    func read(_ request: Request) {
        guard let id = request.id else {
            return emit(.failed(id: nil, reason: "read needs an id"))
        }
        guard connecting == nil, let peripheral, let characteristic = characteristic(request.characteristic) else {
            return emit(.failed(id: id, reason: "not connected or unknown characteristic"))
        }
        reads[characteristic.uuid] = id
        peripheral.readValue(for: characteristic)
    }

    func write(_ request: Request) {
        guard let id = request.id else {
            return emit(.failed(id: nil, reason: "write needs an id"))
        }
        guard connecting == nil, let peripheral, let characteristic = characteristic(request.characteristic),
              let value = request.value, let delivery = request.delivery
        else {
            return emit(.failed(id: id, reason: "not connected, unknown characteristic or no value"))
        }
        switch delivery {
        case .confirmed:
            writes[characteristic.uuid] = id
            peripheral.writeValue(Data(value), for: characteristic, type: .withResponse)
        case .unconfirmed:
            pendingWrite = PendingWrite(id: id, characteristic: characteristic, value: Data(value))
            sendPendingWrite()
        }
    }

    func sendPendingWrite() {
        guard let peripheral, let pending = pendingWrite, peripheral.canSendWriteWithoutResponse else { return }
        pendingWrite = nil
        peripheral.writeValue(pending.value, for: pending.characteristic, type: .withoutResponse)
        emit(.written(id: pending.id))
    }

    func characteristic(_ uuid: String?) -> CBCharacteristic? {
        uuid.flatMap { characteristics[CBUUID(string: $0)] }
    }

    /// Drops the connection or the search, without an event.
    func disconnect() {
        if let peripheral { central.cancelPeripheralConnection(peripheral) }
        if central.isScanning { central.stopScan() }
        forget()
    }

    func forget() {
        peripheral = nil
        characteristics = [:]
        servicesLeft = 0
        reads = [:]
        writes = [:]
        pendingWrite = nil
        connecting = nil
    }

    func failConnecting(_ reason: String) {
        let id = connecting?.id
        disconnect()
        emit(.failed(id: id, reason: reason))
    }

    // MARK: CBCentralManagerDelegate

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        let state: Radio
        switch central.state {
        case .poweredOn: state = .on
        case .poweredOff: state = .off
        case .unauthorized: state = .unauthorized
        case .unsupported: state = .unsupported
        case .resetting, .unknown: state = .unknown
        @unknown default: state = .unknown
        }
        emit(.bluetooth(state: state))
        if central.state == .poweredOn {
            search()
        } else if peripheral != nil {
            // Without the radio, CoreBluetooth drops connections silently.
            let wasConnecting = connecting
            forget()
            if let wasConnecting {
                // Keep searching once the radio is back.
                connecting = wasConnecting
            } else {
                emit(.disconnected(reason: "Bluetooth is \(state.rawValue)"))
            }
        }
    }

    func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                        advertisementData: [String: Any], rssi RSSI: NSNumber) {
        guard connecting != nil else { return }
        switch (self.peripheral, origin) {
        case (nil, _), (_?, .remembered):
            // A fresh sighting beats a remembered peripheral, whose
            // connection attempt may never complete.
            central.stopScan()
            attach(peripheral, from: .scanned)
        case (_?, .scanned):
            break
        }
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        guard peripheral == self.peripheral, let connecting else { return }
        if central.isScanning { central.stopScan() }
        lastIdentifier = peripheral.identifier
        peripheral.discoverServices(connecting.services)
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        // Without an error it is the late answer to our own cancel; CoreBluetooth
        // may hand out the same object again for the next attempt.
        guard peripheral == self.peripheral, let error else { return }
        failConnecting(error.localizedDescription)
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral,
                        error: Error?) {
        guard peripheral == self.peripheral else { return }
        if connecting != nil {
            // See didFailToConnect: no error means our own cancel.
            guard let error else { return }
            return failConnecting(error.localizedDescription)
        }
        forget()
        emit(.disconnected(reason: error?.localizedDescription))
    }

    // MARK: CBPeripheralDelegate

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard peripheral == self.peripheral else { return }
        let services = peripheral.services ?? []
        guard error == nil, !services.isEmpty else {
            return failConnecting(error?.localizedDescription ?? "the lamp offers none of the services")
        }
        servicesLeft = services.count
        for service in services { peripheral.discoverCharacteristics(nil, for: service) }
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard peripheral == self.peripheral else { return }
        if let error { return failConnecting(error.localizedDescription) }
        for characteristic in service.characteristics ?? [] {
            characteristics[characteristic.uuid] = characteristic
        }
        servicesLeft -= 1
        guard servicesLeft == 0, let connecting else { return }
        // Without its changes the core cannot follow the lamp, nor protect
        // it from heat: a missing one fails the connect.
        if let missing = connecting.subscribe.first(where: { characteristics[$0] == nil }) {
            return failConnecting("the lamp lacks \(missing.uuidString)")
        }
        self.connecting = nil
        for uuid in connecting.subscribe {
            if let characteristic = characteristics[uuid] { peripheral.setNotifyValue(true, for: characteristic) }
        }
        emit(.connected(id: connecting.id))
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard peripheral == self.peripheral else { return }
        let value = [UInt8](characteristic.value ?? Data())
        guard let id = reads.removeValue(forKey: characteristic.uuid) else {
            // Not an answer to a read: a notification.
            if let error {
                emit(.failed(
                    id: nil,
                    reason: "a change of \(characteristic.uuid) came with an error: \(error.localizedDescription)"))
            } else {
                emit(.changed(characteristic: characteristic.uuid.uuidString, value: value))
            }
            return
        }
        if let error { return emit(.failed(id: id, reason: error.localizedDescription)) }
        emit(.value(id: id, value: value))
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic,
                    error: Error?) {
        guard peripheral == self.peripheral else { return }
        if let error {
            emit(.unsubscribed(characteristic: characteristic.uuid.uuidString, reason: error.localizedDescription))
        }
    }

    func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
        guard peripheral == self.peripheral, let id = writes.removeValue(forKey: characteristic.uuid) else { return }
        switch error {
        case nil:
            emit(.written(id: id))
        case let error as CBATTError where error.code == .writeNotPermitted:
            // The lamp refuses a mode change shortly after the previous one.
            emit(.rejected(id: id))
        case let error?:
            emit(.failed(id: id, reason: error.localizedDescription))
        }
    }

    func peripheralIsReady(toSendWriteWithoutResponse peripheral: CBPeripheral) {
        guard peripheral == self.peripheral else { return }
        sendPendingWrite()
    }
}
