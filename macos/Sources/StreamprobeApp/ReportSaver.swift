import AppKit
import StreamprobeKit
import UniformTypeIdentifiers

@MainActor
enum ReportSaver {
    enum Format { case text, json }

    /// Writes the final report exactly as streamprobe produced it (`-o report.txt` / `report.json`).
    static func save(_ session: CheckSession, as format: Format) {
        guard let result = session.result else { return }
        let panel = NSSavePanel()
        panel.allowedContentTypes = [format == .json ? .json : .plainText]
        panel.canCreateDirectories = true
        let stamp = Date.now.formatted(.iso8601.year().month().day().dateSeparator(.dash)
            .time(includingFractionalSeconds: false).timeSeparator(.omitted))
            .replacingOccurrences(of: "T", with: "-")
        let host = URL(string: session.startInfo?.url ?? "")?.host() ?? "stream"
        panel.nameFieldStringValue = "streamprobe-\(host)-\(stamp).\(format == .json ? "json" : "txt")"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let body = (format == .json ? result.jsonText : result.text) + "\n"
        do {
            try body.write(to: url, atomically: true, encoding: .utf8)
        } catch {
            NSAlert(error: error).runModal()
        }
    }
}
