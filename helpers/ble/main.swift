// Reads requests on a thread of its own and hands them to the bridge on
// the main thread, where CoreBluetooth calls back.

import Foundation

let bridge = Bridge()

Thread.detachNewThread {
    let decoder = JSONDecoder()
    while let line = readLine() {
        guard let request = try? decoder.decode(Request.self, from: Data(line.utf8)) else {
            DispatchQueue.main.async { emit(.failed(id: nil, reason: "unreadable request")) }
            continue
        }
        DispatchQueue.main.async { bridge.handle(request) }
    }
    DispatchQueue.main.async { exit(0) }
}

RunLoop.main.run()
