import Foundation

/// Splits a byte stream into newline-terminated lines, whatever the chunk boundaries.
public struct LineSplitter: Sendable {
    private var buffer = Data()

    public init() {}

    /// Complete lines in `chunk` (plus what was buffered), without their `\n`.
    public mutating func append(_ chunk: Data) -> [Data] {
        buffer.append(chunk)
        var lines: [Data] = []
        while let nl = buffer.firstIndex(of: 0x0A) {
            let line = buffer[buffer.startIndex..<nl]
            if !line.isEmpty { lines.append(Data(line)) }
            buffer = Data(buffer[buffer.index(after: nl)...])
        }
        return lines
    }

    /// The unterminated tail at end of stream, if any.
    public mutating func finish() -> Data? {
        defer { buffer = Data() }
        return buffer.isEmpty ? nil : buffer
    }
}
