import Foundation
import Testing
@testable import StreamprobeKit

private func findingsDoc() throws -> String {
    let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    return try String(contentsOf: repo.appending(path: "docs/FINDINGS.md"), encoding: .utf8)
}

@Test func parsesRealFindingsDoc() throws {
    let t = FindingExplanations.parse(markdown: try findingsDoc())
    for code in ["underrun", "http_401", "http_403", "http_503", "tls_cert", "dead_air", "out_of_phase"] {
        #expect(t[code] != nil, "missing \(code)")
    }
    #expect(t["underrun"]?.level == "bad")
    #expect(t["underrun"]?.check.contains("listeners hear buffering") == true)
    #expect(t["http_401"] == t["http_403"])
    for (code, e) in t {
        for s in [e.level, e.meaning, e.check] {
            #expect(!s.contains("`") && !s.contains("**"), "\(code): \(s)")
        }
    }
    #expect(t["Code"] == nil)
}

@Test func eventOnlyTableIsIgnored() {
    let md = """
    | Code | Meaning |
    |---|---|
    | `connected`, `reconnected` | a connection (re)opened |
    """
    #expect(FindingExplanations.parse(markdown: md).isEmpty)
}
