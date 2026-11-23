import Foundation
import Testing
@testable import StreamprobeKit

func fixtureLines(_ name: String) throws -> [Data] {
    let url = try #require(Bundle.module.url(forResource: "Fixtures/\(name)", withExtension: nil))
    let text = try String(contentsOf: url, encoding: .utf8)
    return text.split(separator: "\n").map { Data($0.utf8) }
}

@Test func decodesEveryFixtureLine() throws {
    let msgs = try fixtureLines("faults.ndjson").map { try #require(ProbeMessage.decode($0)) }
    guard case .start(let start) = msgs.first else { Issue.record("first is not start"); return }
    #expect(start.url == "http://127.0.0.1:18780/faults")
    guard case .finished(let fin) = msgs.last else { Issue.record("last is not final"); return }
    #expect(fin.exitCode == 2)
    #expect(fin.report.result == "problems")
    #expect(!fin.text.isEmpty)
    #expect(fin.jsonText.hasPrefix("{"))
    #expect(!fin.report.timeline.points.isEmpty)
    let lives = msgs.compactMap { if case .live(let l) = $0 { l } else { nil } }
    let reports = msgs.compactMap { if case .report(let r) = $0 { r } else { nil } }
    #expect(!lives.isEmpty && !reports.isEmpty)
    let live = try #require(lives.last)
    #expect(live.loudness.integratedLufs != nil)
    #expect(live.delivery.listenerBufferS >= 0)
    #expect(!live.findings.isEmpty)
}

@Test func missingMountIsFinalWithExit3() throws {
    let last = try #require(fixtureLines("missing.ndjson").last)
    guard case .finished(let fin) = ProbeMessage.decode(last) else { Issue.record("not final"); return }
    #expect(fin.exitCode == 3)
    #expect(fin.report.findings.map(\.code).contains("http_404"))
}

@Test func errorLineDecodes() {
    let m = ProbeMessage.decode(Data(#"{"type":"error","code":"ffmpeg_missing","message":"no ffmpeg"}"#.utf8))
    guard case .error(let e) = m else { Issue.record("not error"); return }
    #expect(e.code == "ffmpeg_missing")
}

@Test func unknownTypeIsIgnored() {
    guard case .unknown(let t) = ProbeMessage.decode(Data(#"{"type":"future","x":1}"#.utf8)) else {
        Issue.record("not unknown"); return
    }
    #expect(t == "future")
    #expect(ProbeMessage.decode(Data("not json".utf8)) == nil)
}

@Test func splitterHandlesChunks() {
    var s = LineSplitter()
    #expect(s.append(Data("{\"a\":1}\n{\"b\"".utf8)) == [Data(#"{"a":1}"#.utf8)])
    #expect(s.append(Data(":2}\n{\"c\":3}".utf8)) == [Data(#"{"b":2}"#.utf8)])
    #expect(s.finish() == Data(#"{"c":3}"#.utf8))
    #expect(s.finish() == nil)
    var t = LineSplitter()
    #expect(t.append(Data("x\ny\n".utf8)).count == 2)
    #expect(t.finish() == nil)
}

@Test func levelOrder() {
    #expect(Level.bad > .warn && Level.warn > .info && Level.info > .ok)
}

@Test func jsonValueDisplay() throws {
    let v = try JSONDecoder().decode([String: JSONValue].self, from: Data(#"{"a":128,"b":1.5,"c":"x","d":null,"e":true,"f":[1,"y"]}"#.utf8))
    #expect(v["a"]?.displayString == "128")
    #expect(v["b"]?.displayString == "1.5")
    #expect(v["c"]?.displayString == "x")
    #expect(v["d"]?.displayString == "")
    #expect(v["e"]?.displayString == "yes")
    #expect(v["f"]?.displayString == "1, y")
}
