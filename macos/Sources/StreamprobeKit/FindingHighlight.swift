/// Rows of the timeline, top to bottom.
public enum TimelineRow: Sendable, CaseIterable {
    case level, stereo, clip, delivery, track
}

/// Where on the timeline a selected finding happened. Display only: it never
/// decides whether something is a problem, it only points at what streamprobe logged.
public struct FindingHighlight: Sendable, Equatable {
    /// nil: nothing to highlight.
    public var row: TimelineRow?
    /// Seconds from the start of the check. Empty with a row: outline the whole row.
    public var ranges: [ClosedRange<Double>]

    public static let none = FindingHighlight(row: nil, ranges: [])

    public static func highlight(for code: String, events: [ProbeEvent], timeline: Timeline?, now: Double)
        -> FindingHighlight {
        func paired(_ start: String, ends: Set<String>) -> [ClosedRange<Double>] {
            var out: [ClosedRange<Double>] = []
            var open: Double?
            for e in events.sorted(by: { $0.offsetS < $1.offsetS }) {
                if e.code == start, open == nil {
                    open = e.offsetS
                } else if ends.contains(e.code), let s = open {
                    out.append(s...max(s, e.offsetS))
                    open = nil
                }
            }
            if let s = open { out.append(s...max(s, now)) }
            return out
        }
        func instants(_ c: String) -> [ClosedRange<Double>] {
            events.filter { $0.code == c }.map { $0.offsetS...$0.offsetS }.sorted { $0.lowerBound < $1.lowerBound }
        }
        func points(_ match: (TimelinePoint) -> Bool) -> [ClosedRange<Double>] {
            guard let timeline else { return [] }
            let w = Double(timeline.resolutionS)
            var out: [ClosedRange<Double>] = []
            for p in timeline.points where match(p) {
                let r = Double(p.offsetS)...(Double(p.offsetS) + w)
                if let last = out.last, last.upperBound >= r.lowerBound {
                    out[out.count - 1] = last.lowerBound...max(last.upperBound, r.upperBound)
                } else {
                    out.append(r)
                }
            }
            return out
        }

        switch code {
        case "stall": return .init(row: .delivery, ranges: paired("stall", ends: ["stall_end"]))
        case "dead_air": return .init(row: .level, ranges: paired("dead_air", ends: ["dead_air_end"]))
        case "out_of_phase", "mono_content":
            return .init(row: .stereo, ranges: paired(code, ends: ["\(code)_end"]))
        case "underrun": return .init(row: .delivery, ranges: instants("underrun"))
        case "dead_channel":
            let r = paired("left_only", ends: ["left_only_end"]) + paired("right_only", ends: ["right_only_end"])
            return .init(row: .stereo, ranges: r.sorted { $0.lowerBound < $1.lowerBound })
        case "decode_errors": return .init(row: .delivery, ranges: instants("decode_error"))
        case "disconnect":
            return .init(row: .delivery, ranges: paired("disconnect", ends: ["reconnected", "reconnect_failed"]))
        case "clipping", "full_scale": return .init(row: .clip, ranges: points { $0.clippedSamples > 0 })
        case "partly_mono": return .init(row: .stereo, ranges: points { $0.stereo == "mono_content" })
        case "mostly_silent": return .init(row: .stereo, ranges: points { $0.stereo == "silence" })
        case "slow_delivery": return .init(row: .delivery, ranges: [])
        default: return .none
        }
    }
}
