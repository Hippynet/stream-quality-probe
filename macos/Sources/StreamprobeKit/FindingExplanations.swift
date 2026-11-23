import Foundation

/// What a finding code means and what to check, from docs/FINDINGS.md.
public struct Explanation: Sendable, Equatable {
    public var level: String
    public var meaning: String
    public var check: String
}

public enum FindingExplanations {
    /// Reads every 4-column table row whose first cell names codes in backticks;
    /// each code in that cell maps to the row. Markdown emphasis is stripped.
    public static func parse(markdown: String) -> [String: Explanation] {
        var out: [String: Explanation] = [:]
        for raw in markdown.split(separator: "\n") {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix("|"), line.hasSuffix("|") else { continue }
            let cells = line.dropFirst().dropLast().split(separator: "|", omittingEmptySubsequences: false)
                .map { $0.trimmingCharacters(in: .whitespaces) }
            guard cells.count == 4 else { continue }
            let codes = backticked(cells[0])
            guard !codes.isEmpty else { continue }
            let e = Explanation(level: plain(cells[1]), meaning: plain(cells[2]), check: plain(cells[3]))
            for c in codes { out[c] = e }
        }
        return out
    }

    /// FINDINGS.md bundled in the app, else `STREAMPROBE_FINDINGS_MD` (development), else nothing.
    public static func load() -> [String: Explanation] {
        let candidates = [
            Bundle.main.url(forResource: "FINDINGS", withExtension: "md"),
            ProcessInfo.processInfo.environment["STREAMPROBE_FINDINGS_MD"].map { URL(fileURLWithPath: $0) },
        ]
        for case let url? in candidates {
            if let md = try? String(contentsOf: url, encoding: .utf8) { return parse(markdown: md) }
        }
        return [:]
    }

    private static func backticked(_ s: String) -> [String] {
        let parts = s.split(separator: "`", omittingEmptySubsequences: false)
        return stride(from: 1, to: parts.count, by: 2).map { String(parts[$0]) }.filter { !$0.isEmpty }
    }

    private static func plain(_ s: String) -> String {
        s.replacingOccurrences(of: "**", with: "").replacingOccurrences(of: "`", with: "")
    }
}
