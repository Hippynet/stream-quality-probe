import Foundation
import Testing
@testable import StreamprobeKit

private final class Box: @unchecked Sendable {
    let lock = NSLock()
    var lines: [String] = []
    var exit: Int32?
}

/// A window that closes mid-check drops its ProbeProcess while the child still runs.
/// The process must keep reading its pipes to the end (not spin on them) and still report the exit.
@Test func releasedProcessStillDrainsAndReportsExit() async throws {
    let box = Box()
    do {
        let p = ProbeProcess(
            executable: URL(fileURLWithPath: "/bin/sh"), arguments: ["-c", "sleep 0.3; echo hi; echo err >&2"],
            onLine: { d in box.lock.withLock { box.lines.append(String(decoding: d, as: UTF8.self)) } },
            onStderr: { _ in },
            onExit: { s in box.lock.withLock { box.exit = s } })
        try p.start()
    } // p released here
    let end = Date().addingTimeInterval(5)
    while box.lock.withLock({ box.exit }) == nil && Date() < end {
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(box.lock.withLock { box.exit } == 0)
    #expect(box.lock.withLock { box.lines } == ["hi"])
}
