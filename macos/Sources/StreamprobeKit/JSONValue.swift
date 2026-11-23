import Foundation

/// Any JSON value, for the report sections the app shows as-is (Details tab).
public enum JSONValue: Decodable, Sendable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let b = try? c.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? c.decode(Double.self) {
            self = .number(n)
        } else if let s = try? c.decode(String.self) {
            self = .string(s)
        } else if let a = try? c.decode([JSONValue].self) {
            self = .array(a)
        } else {
            self = .object(try c.decode([String: JSONValue].self))
        }
    }

    /// Plain text for display: integers without ".0", yes/no, arrays comma-joined, null empty.
    public var displayString: String {
        switch self {
        case .null: return ""
        case .bool(let b): return b ? "yes" : "no"
        case .number(let n):
            return n.rounded() == n && abs(n) < 1e15 ? String(Int64(n)) : String(n)
        case .string(let s): return s
        case .array(let a): return a.map(\.displayString).joined(separator: ", ")
        case .object(let o):
            return o.keys.sorted().map { "\($0): \(o[$0]!.displayString)" }.joined(separator: ", ")
        }
    }

    public subscript(key: String) -> JSONValue? {
        if case .object(let o) = self { return o[key] }
        return nil
    }

    public var stringValue: String? {
        if case .string(let s) = self { return s }
        return nil
    }

    public var numberValue: Double? {
        if case .number(let n) = self { return n }
        return nil
    }
}
