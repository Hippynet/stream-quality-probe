import Foundation
import Testing
@testable import StreamprobeKit

private func fakeProbe() throws -> URL {
    let url = try #require(Bundle.module.url(forResource: "Fixtures/fake-probe.sh", withExtension: nil))
    try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: url.path)
    return url
}

@MainActor
private func waitFor(_ seconds: Double = 10, _ cond: () -> Bool) async {
    let end = Date().addingTimeInterval(seconds)
    while !cond() && Date() < end {
        try? await Task.sleep(for: .milliseconds(20))
    }
}

private func liveSnapshot(events: [ProbeEvent] = [], findings: [Finding] = []) -> LiveSnapshot {
    LiveSnapshot(
        elapsedS: 1, phase: "live", streamUrl: "http://x/",
        levels: nil,
        loudness: .init(momentaryLufs: nil, shortTermLufs: nil, integratedLufs: nil, lraLu: nil, truePeakDbtp: nil),
        stereo: .init(state: nil, correlation: nil, sideDb: nil),
        quality: .init(bandwidthHz: nil, samplesOver0Dbfs: 0, samplesAtFullScale: 0, maxPeakDbfs: nil, decodeErrors: 0),
        delivery: .init(realtimeRatio: nil, inputKbps: nil, listenerBufferS: 0, underruns: 0, stalls: 0, disconnects: 0),
        nowPlaying: .init(title: nil, sinceS: nil, previous: nil),
        result: "healthy", findings: findings, events: events, eventsTotal: events.count)
}

@MainActor @Test func sessionReplaysFixtureToFinished() async throws {
    let s = CheckSession()
    s.start(CheckRequest(url: "replay:faults.ndjson"), binary: try fakeProbe())
    #expect(s.phase == .connecting)
    await waitFor { s.phase == .finished(exitCode: 2) }
    #expect(s.phase == .finished(exitCode: 2))
    #expect(!s.findings.isEmpty)
    #expect(!s.events.isEmpty)
    #expect(s.verdict == "problems")
    #expect(s.result?.text.isEmpty == false)
}

@MainActor @Test func sessionEndsUnexpectedlyWithoutFinal() async throws {
    let s = CheckSession()
    s.start(CheckRequest(url: "die"), binary: try fakeProbe())
    await waitFor { s.phase == .endedUnexpectedly(exitStatus: 9) }
    #expect(s.phase == .endedUnexpectedly(exitStatus: 9))
    await waitFor { !s.stderrTail.isEmpty }
    #expect(s.stderrTail == ["fake: dying"])
}

@MainActor @Test func sessionFailsOnErrorLine() async throws {
    let s = CheckSession()
    s.start(CheckRequest(url: "error"), binary: try fakeProbe())
    await waitFor { if case .failed = s.phase { true } else { false } }
    #expect(s.phase == .failed(ProbeError(code: "ffmpeg_missing", message: "ffmpeg not found")))
}

@MainActor @Test func stopWhileConnectingKillsAStubbornProcess() async throws {
    let s = CheckSession()
    s.killAfter = .milliseconds(500)
    s.start(CheckRequest(url: "stubborn"), binary: try fakeProbe())
    await waitFor { s.startInfo != nil }
    let t0 = Date()
    s.stop()
    await waitFor(4) { s.phase == .stoppedWhileConnecting }
    #expect(s.phase == .stoppedWhileConnecting)
    #expect(Date().timeIntervalSince(t0) < 4)
}

@MainActor @Test func launchFailureIsReported() {
    let s = CheckSession()
    s.start(CheckRequest(url: "x"), binary: URL(fileURLWithPath: "/nonexistent/streamprobe"))
    if case .failed(let e) = s.phase { #expect(e.code == "launch") } else { Issue.record("\(s.phase)") }
}

@MainActor @Test func eventsAreBounded() {
    let s = CheckSession()
    var n = 0
    for _ in 0..<6 {
        let evs = (0..<1000).map { _ -> ProbeEvent in
            n += 1
            return ProbeEvent(t: "", offsetS: Double(n), level: .info, code: "title", message: "e\(n)")
        }
        s.handle(.live(liveSnapshot(events: evs)))
    }
    #expect(s.events.count == 5000)
    #expect(s.events.first?.message == "e1")
    #expect(s.events[99].message == "e100")
    #expect(s.events[100].message == "e1101")
    #expect(s.events.last?.message == "e6000")
}

@MainActor @Test func findingsSwitchToFinalReport() throws {
    let s = CheckSession()
    s.handle(.live(liveSnapshot(findings: [Finding(level: .warn, code: "x", message: "live")])))
    #expect(s.phase == .running)
    #expect(s.findings.map(\.code) == ["x"])
    #expect(s.verdict == "healthy")
    let last = try #require(fixtureLines("faults.ndjson").last)
    let msg = try #require(ProbeMessage.decode(last))
    s.handle(msg)
    guard case .finished(let fin) = msg else { Issue.record("not final"); return }
    #expect(s.findings == fin.report.findings)
    #expect(s.verdict == "problems")
    #expect(s.phase == .finished(exitCode: 2))
}

@MainActor @Test func sawAudioTracksLevels() {
    let s = CheckSession()
    s.handle(.live(liveSnapshot()))
    #expect(!s.sawAudio)
    var withLevels = liveSnapshot()
    withLevels.levels = .init(rmsL: -15, rmsR: -15, peakL: -7, peakR: -7)
    s.handle(.live(withLevels))
    #expect(s.sawAudio)
    s.handle(.live(liveSnapshot()))
    #expect(s.sawAudio) // stays true: audio stopped arriving
}

@MainActor @Test func finalReportEventsFillAnEmptyLog() throws {
    let s = CheckSession()
    let last = try #require(fixtureLines("missing.ndjson").last)
    s.handle(try #require(ProbeMessage.decode(last)))
    #expect(s.events.map(\.code).contains("http_404"))
    #expect(s.result?.report.events.isEmpty == false)
}
