import Foundation

/// Runs streamprobe and reports its stdout lines, stderr lines and exit.
/// `onExit` fires once, only after stdout has reached end of file and every
/// line (including an unterminated last one) has been passed to `onLine`. If
/// something else still holds stdout open (a grandchild), it gives up waiting
/// for end of file `eofGrace` after the process exits.
public final class ProbeProcess: @unchecked Sendable {
    private let process = Process()
    private let onLine: @Sendable (Data) -> Void
    private let onStderr: @Sendable (String) -> Void
    private let onExit: @Sendable (Int32) -> Void
    private let lock = NSLock()
    private var stdoutDone = false
    private var exitStatus: Int32?
    private var exitReported = false
    private var outSplit = LineSplitter()
    private var errSplit = LineSplitter()
    private var outHandle: FileHandle?
    let eofGrace: TimeInterval = 1.0

    public init(executable: URL, arguments: [String],
                onLine: @Sendable @escaping (Data) -> Void,
                onStderr: @Sendable @escaping (String) -> Void,
                onExit: @Sendable @escaping (Int32) -> Void) {
        process.executableURL = executable
        process.arguments = arguments
        self.onLine = onLine
        self.onStderr = onStderr
        self.onExit = onExit
    }

    public func start() throws {
        let out = Pipe(), err = Pipe()
        process.standardOutput = out
        process.standardError = err
        process.standardInput = FileHandle.nullDevice

        // The handlers hold `self` strongly on purpose: a closed window may drop its
        // ProbeProcess mid-check, and the pipes must still be drained to end of file
        // (a handler that returns without reading is called again at once, forever).
        // Each handler clears itself at end of file, which breaks the cycle.
        outHandle = out.fileHandleForReading
        out.fileHandleForReading.readabilityHandler = { h in
            let data = h.availableData
            if data.isEmpty {
                self.endStdout()
            } else {
                for line in self.lock.withLock({ self.outSplit.append(data) }) { self.onLine(line) }
            }
        }
        err.fileHandleForReading.readabilityHandler = { h in
            let data = h.availableData
            let lines = self.lock.withLock {
                data.isEmpty ? [self.errSplit.finish()].compactMap { $0 } : self.errSplit.append(data)
            }
            if data.isEmpty { h.readabilityHandler = nil }
            for l in lines { self.onStderr(String(decoding: l, as: UTF8.self)) }
        }
        process.terminationHandler = { p in
            self.lock.withLock { self.exitStatus = p.terminationStatus }
            self.reportExitIfDone()
            DispatchQueue.global().asyncAfter(deadline: .now() + self.eofGrace) {
                self.endStdout()
            }
            p.terminationHandler = nil
        }
        try process.run()
    }

    /// SIGINT (streamprobe writes its final report), then SIGKILL if it's still running.
    public func interrupt(killAfter: Duration = .seconds(3)) {
        guard process.isRunning else { return }
        process.interrupt()
        let pid = process.processIdentifier
        let s = killAfter.components
        let delay = Double(s.seconds) + Double(s.attoseconds) / 1e18
        DispatchQueue.global().asyncAfter(deadline: .now() + delay) { [process] in
            if process.isRunning { kill(pid, SIGKILL) }
        }
    }

    /// End of stdout (or given up waiting for it): flush the tail once, then report the exit.
    private func endStdout() {
        let tail: Data?? = lock.withLock {
            guard !stdoutDone else { return .none }
            stdoutDone = true
            outHandle?.readabilityHandler = nil
            return .some(outSplit.finish())
        }
        if case .some(.some(let t)) = tail { onLine(t) }
        reportExitIfDone()
    }

    private func reportExitIfDone() {
        let status: Int32? = lock.withLock {
            guard stdoutDone, let s = exitStatus, !exitReported else { return nil }
            exitReported = true
            return s
        }
        if let status { onExit(status) }
    }
}
