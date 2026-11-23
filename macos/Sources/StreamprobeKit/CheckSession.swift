import Foundation
import Observation

/// One check in one window: the process, and everything it has reported.
@MainActor @Observable
public final class CheckSession {
    public enum Phase: Equatable, Sendable {
        case idle
        case connecting
        case running
        case finished(exitCode: Int)
        case stoppedWhileConnecting
        case failed(ProbeError)
        case endedUnexpectedly(exitStatus: Int32)
    }

    /// Mirrors the CLI's event cap (MAX_EVENTS): keep the first 100 and the most recent.
    public static let maxEvents = 5000
    static let keepFirstEvents = 100
    static let stderrLines = 20

    public private(set) var phase: Phase = .idle
    public private(set) var request: CheckRequest?
    public private(set) var startInfo: StartInfo?
    public private(set) var live: LiveSnapshot?
    public private(set) var report: ProbeReport?
    public private(set) var result: FinalResult?
    public private(set) var events: [ProbeEvent] = []
    public private(set) var stderrTail: [String] = []
    /// Audio has been decoded at some point (meters say "no audio arriving" rather than "waiting").
    public private(set) var sawAudio = false
    public var killAfter: Duration = .seconds(3)

    @ObservationIgnored private var process: ProbeProcess?
    @ObservationIgnored private var run = 0
    @ObservationIgnored private var stopRequested = false

    public init() {}

    public var isActive: Bool { phase == .connecting || phase == .running }

    /// The final report's findings once finished, else the live (in-progress) ones.
    public var findings: [Finding] { result?.report.findings ?? live?.findings ?? [] }

    /// `healthy`, `warnings` or `problems`, by the same rule as `findings`.
    public var verdict: String? { result?.report.result ?? live?.result }

    public func start(_ r: CheckRequest, binary: URL) {
        stop()
        run += 1
        let thisRun = run
        phase = .connecting
        request = r
        startInfo = nil; live = nil; report = nil; result = nil
        events = []; stderrTail = []; sawAudio = false
        stopRequested = false

        enum Incoming: Sendable { case line(Data), stderr(String), exit(Int32) }
        let (stream, cont) = AsyncStream.makeStream(of: Incoming.self)
        let p = ProbeProcess(
            executable: binary, arguments: r.arguments,
            onLine: { cont.yield(.line($0)) },
            onStderr: { cont.yield(.stderr($0)) },
            onExit: { cont.yield(.exit($0)); cont.finish() })
        do {
            try p.start()
        } catch {
            phase = .failed(ProbeError(code: "launch", message: "Could not start streamprobe: \(error.localizedDescription)"))
            cont.finish()
            return
        }
        process = p
        Task { @MainActor [weak self] in
            for await item in stream {
                guard let self, self.run == thisRun else { return }
                switch item {
                case .line(let d): if let m = ProbeMessage.decode(d) { self.handle(m) }
                case .stderr(let s): self.appendStderr(s)
                case .exit(let status): self.processExited(status: status)
                }
            }
        }
    }

    /// Ask streamprobe to stop and write its final report.
    public func stop() {
        guard isActive, let process else { return }
        stopRequested = true
        process.interrupt(killAfter: killAfter)
    }

    func handle(_ m: ProbeMessage) {
        switch m {
        case .start(let s):
            startInfo = s
        case .live(let l):
            live = l
            if l.levels != nil { sawAudio = true }
            appendEvents(l.events)
            if phase == .connecting || phase == .idle { phase = .running }
        case .report(let r):
            report = r
        case .finished(let f):
            result = f
            report = f.report
            phase = .finished(exitCode: f.exitCode)
        case .error(let e):
            phase = .failed(e)
        case .unknown:
            break
        }
    }

    func processExited(status: Int32) {
        process = nil
        guard isActive else { return } // a final or error line already settled it
        phase = stopRequested && live == nil ? .stoppedWhileConnecting : .endedUnexpectedly(exitStatus: status)
    }

    private func appendEvents(_ new: [ProbeEvent]) {
        events.append(contentsOf: new)
        let over = events.count - Self.maxEvents
        if over > 0 {
            events.removeSubrange(Self.keepFirstEvents..<(Self.keepFirstEvents + over))
        }
    }

    private func appendStderr(_ line: String) {
        stderrTail.append(line)
        if stderrTail.count > Self.stderrLines { stderrTail.removeFirst(stderrTail.count - Self.stderrLines) }
    }
}
