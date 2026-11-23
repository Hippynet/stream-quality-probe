import Foundation

/// What the user asked to check, and the streamprobe arguments for it.
public struct CheckRequest: Sendable, Equatable {
    public var url: String
    public var durationS: Int?
    public var insecure: Bool
    public var allowInternal: Bool

    public init(url: String, durationS: Int? = nil, insecure: Bool = false, allowInternal: Bool = false) {
        self.url = url
        self.durationS = durationS
        self.insecure = insecure
        self.allowInternal = allowInternal
    }

    public var trimmedURL: String { url.trimmingCharacters(in: .whitespacesAndNewlines) }

    public var isValid: Bool { !trimmedURL.isEmpty }

    /// A URL without a scheme is passed on as typed: streamprobe adds `http://` itself.
    public var arguments: [String] {
        var a = [trimmedURL, "--live-json"]
        if let d = durationS { a += ["-t", String(d)] }
        if insecure { a.append("--insecure") }
        if allowInternal { a.append("--allow-internal") }
        return a
    }
}
