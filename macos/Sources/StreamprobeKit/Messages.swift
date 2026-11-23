import Foundation

// The lines `streamprobe --live-json` writes (see README "Live JSON for front ends").

public enum Level: String, Decodable, Sendable, Comparable, CaseIterable {
    case ok, info, warn, bad

    private var rank: Int { Level.allCases.firstIndex(of: self)! }
    public static func < (a: Level, b: Level) -> Bool { a.rank < b.rank }
}

public struct Finding: Decodable, Sendable, Equatable, Hashable {
    public var level: Level
    public var code: String
    public var message: String
}

public struct ProbeEvent: Decodable, Sendable, Equatable {
    public var t: String
    public var offsetS: Double
    public var level: Level
    public var code: String
    public var message: String
}

public struct StartInfo: Decodable, Sendable, Equatable {
    public var version: String
    public var url: String
}

public struct ProbeError: Decodable, Sendable, Equatable {
    public var code: String
    public var message: String
}

public struct LiveSnapshot: Decodable, Sendable, Equatable {
    public struct Levels: Decodable, Sendable, Equatable {
        public var rmsL, rmsR, peakL, peakR: Double?
    }
    public struct Loudness: Decodable, Sendable, Equatable {
        public var momentaryLufs, shortTermLufs, integratedLufs, lraLu, truePeakDbtp: Double?
    }
    public struct Stereo: Decodable, Sendable, Equatable {
        public var state: String?
        public var correlation: Double?
        public var sideDb: Double?
    }
    public struct Quality: Decodable, Sendable, Equatable {
        public var bandwidthHz: Double?
        public var samplesOver0Dbfs: Int
        public var samplesAtFullScale: Int
        public var maxPeakDbfs: Double?
        public var decodeErrors: Int
    }
    public struct Delivery: Decodable, Sendable, Equatable {
        public var realtimeRatio: Double?
        public var inputKbps: Double?
        public var listenerBufferS: Double
        public var underruns, stalls, disconnects: Int
    }
    public struct NowPlaying: Decodable, Sendable, Equatable {
        public var title: String?
        public var sinceS: Double?
        public var previous: String?
    }

    public var elapsedS: Double
    public var phase: String
    public var streamUrl: String
    public var levels: Levels?
    public var loudness: Loudness
    public var stereo: Stereo
    public var quality: Quality
    public var delivery: Delivery
    public var nowPlaying: NowPlaying
    public var result: String
    public var findings: [Finding]
    public var events: [ProbeEvent]
    public var eventsTotal: Int
}

public struct TimelinePoint: Decodable, Sendable, Equatable {
    public var t: String
    public var offsetS: Int
    public var lufsShort: Double?
    public var rmsDbfs: Double?
    public var silentS: Double
    public var stereo: String?
    public var clippedSamples: Int
    public var delivery: String?
    public var decodeErrors: Int
    public var titleChange: Bool
}

public struct Timeline: Decodable, Sendable, Equatable {
    public var resolutionS: Int
    public var points: [TimelinePoint]
}

/// The `--json` report: typed where the app computes with it, raw sections for display.
public struct ProbeReport: Sendable, Equatable {
    public var result: String
    public var findings: [Finding]
    public var timeline: Timeline
    public var durationS: Double
    public var stopReason: String
    /// The report's event log (its most recent 200 events).
    public var events: [ProbeEvent]
    public var connection: [String: JSONValue]
    public var headers: [String: JSONValue]
    public var format: [String: JSONValue]
    public var audio: [String: JSONValue]
    public var delivery: [String: JSONValue]
    public var metadata: [String: JSONValue]
}

public struct FinalResult: Sendable, Equatable {
    public var report: ProbeReport
    public var jsonText: String
    public var text: String
    public var exitCode: Int
}

public enum ProbeMessage: Sendable, Equatable {
    case start(StartInfo)
    case live(LiveSnapshot)
    case report(ProbeReport)
    case finished(FinalResult)
    case error(ProbeError)
    case unknown(String)

    /// One line of output; nil if it isn't a JSON object with a `type`.
    public static func decode(_ line: Data) -> ProbeMessage? {
        // Typed fields use snake_case conversion; report sections are decoded with
        // the plain decoder so their keys (headers, ICY fields) stay exactly as sent.
        let snake = JSONDecoder()
        snake.keyDecodingStrategy = .convertFromSnakeCase
        let plain = JSONDecoder()
        guard let kind = try? plain.decode(Kind.self, from: line) else { return nil }
        do {
            switch kind.type {
            case "start": return .start(try snake.decode(StartInfo.self, from: line))
            case "live": return .live(try snake.decode(LiveSnapshot.self, from: line))
            case "error": return .error(try snake.decode(ProbeError.self, from: line))
            case "report":
                return .report(try report(snake.decode(Typed.self, from: line).report,
                                           plain.decode(Raw.self, from: line).report))
            case "final":
                let t = try snake.decode(Typed.self, from: line)
                let r = try report(t.report, plain.decode(Raw.self, from: line).report)
                return .finished(FinalResult(report: r, jsonText: t.jsonText ?? "", text: t.text ?? "",
                                             exitCode: t.exitCode ?? 0))
            default: return .unknown(kind.type)
            }
        } catch {
            return nil
        }
    }

    private struct Kind: Decodable { var type: String }

    private struct TypedReport: Decodable {
        var result: String
        var findings: [Finding]
        var timeline: Timeline
        var durationS: Double
        var stopReason: String
        var events: [ProbeEvent]
    }
    private struct Typed: Decodable {
        var report: TypedReport
        var jsonText: String?
        var text: String?
        var exitCode: Int?
    }
    private struct RawReport: Decodable {
        var connection, headers, format, audio, delivery, metadata: [String: JSONValue]
    }
    private struct Raw: Decodable { var report: RawReport }

    private static func report(_ t: TypedReport, _ r: RawReport) -> ProbeReport {
        ProbeReport(result: t.result, findings: t.findings, timeline: t.timeline, durationS: t.durationS,
                    stopReason: t.stopReason, events: t.events, connection: r.connection, headers: r.headers, format: r.format,
                    audio: r.audio, delivery: r.delivery, metadata: r.metadata)
    }
}
