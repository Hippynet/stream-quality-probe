import Testing
@testable import StreamprobeKit

private func ev(_ code: String, _ t: Double) -> ProbeEvent {
    ProbeEvent(t: "", offsetS: t, level: .info, code: code, message: "")
}

private func pt(_ off: Int, clip: Int = 0, stereo: String? = "stereo") -> TimelinePoint {
    TimelinePoint(t: "", offsetS: off, lufsShort: nil, rmsDbfs: nil, silentS: 0, stereo: stereo,
                  clippedSamples: clip, delivery: "ok", decodeErrors: 0, titleChange: false)
}

private func h(_ code: String, _ events: [ProbeEvent] = [], _ points: [TimelinePoint] = [], res: Int = 1, now: Double = 100)
    -> FindingHighlight {
    FindingHighlight.highlight(for: code, events: events, timeline: Timeline(resolutionS: res, points: points), now: now)
}

@Test func pairedEventsGiveRanges() {
    #expect(h("stall", [ev("stall", 10), ev("stall_end", 18), ev("stall", 30), ev("stall_end", 35)])
        == FindingHighlight(row: .delivery, ranges: [10...18, 30...35]))
    #expect(h("dead_air", [ev("dead_air", 5), ev("dead_air_end", 11)]).row == .level)
    #expect(h("out_of_phase", [ev("out_of_phase", 1), ev("out_of_phase_end", 2)]) == FindingHighlight(row: .stereo, ranges: [1...2]))
    #expect(h("mono_content", [ev("mono_content", 3), ev("mono_content_end", 4)]) == FindingHighlight(row: .stereo, ranges: [3...4]))
}

@Test func openRangeRunsToNow() {
    #expect(h("stall", [ev("stall", 40)], now: 50) == FindingHighlight(row: .delivery, ranges: [40...50]))
}

@Test func underrunsAndDecodeErrorsAreInstants() {
    #expect(h("underrun", [ev("underrun", 7), ev("underrun", 9)]) == FindingHighlight(row: .delivery, ranges: [7...7, 9...9]))
    #expect(h("decode_errors", [ev("decode_error", 3)]) == FindingHighlight(row: .delivery, ranges: [3...3]))
}

@Test func deadChannelUsesLeftAndRightEvents() {
    let e = [ev("left_only", 10), ev("left_only_end", 20), ev("right_only", 30), ev("right_only_end", 31)]
    #expect(h("dead_channel", e) == FindingHighlight(row: .stereo, ranges: [10...20, 30...31]))
}

@Test func disconnectEndsAtReconnectOrFailure() {
    let e = [ev("disconnect", 10), ev("reconnected", 12), ev("disconnect", 20), ev("reconnect_failed", 25)]
    #expect(h("disconnect", e) == FindingHighlight(row: .delivery, ranges: [10...12, 20...25]))
}

@Test func clipPointsMerge() {
    let pts = [pt(9), pt(10, clip: 5), pt(11, clip: 1), pt(12, clip: 3), pt(13), pt(20, clip: 1)]
    #expect(h("clipping", [], pts) == FindingHighlight(row: .clip, ranges: [10...13, 20...21]))
    #expect(h("full_scale", [], [pt(0, clip: 1)], res: 5) == FindingHighlight(row: .clip, ranges: [0...5]))
}

@Test func stereoRowPoints() {
    let pts = [pt(0, stereo: "mono_content"), pt(1, stereo: "stereo"), pt(2, stereo: "silence")]
    #expect(h("partly_mono", [], pts) == FindingHighlight(row: .stereo, ranges: [0...1]))
    #expect(h("mostly_silent", [], pts) == FindingHighlight(row: .stereo, ranges: [2...3]))
}

@Test func slowDeliveryOutlinesTheRow() {
    #expect(h("slow_delivery") == FindingHighlight(row: .delivery, ranges: []))
}

@Test func othersHaveNoHighlight() {
    #expect(h("too_loud").row == nil)
    #expect(h("tls_cert").row == nil)
}

@Test func hoverLooksUpPointsByTime() {
    // the timeline zooms out (1 s → 2 s columns) between reports: a hovered time must stay valid
    let fine = Timeline(resolutionS: 1, points: (0..<120).map { pt($0) })
    #expect(fine.pointIndex(at: 119.5) == 119)
    let coarse = Timeline(resolutionS: 2, points: (0..<61).map { pt($0 * 2) })
    #expect(coarse.pointIndex(at: 119.5) == 59)
    #expect(coarse.pointIndex(at: 500) == nil)
    #expect(coarse.pointIndex(at: -1) == nil)
}
