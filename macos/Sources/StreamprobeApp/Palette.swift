import StreamprobeKit
import SwiftUI

/// Colours and names shared by the live cards and the timeline (matching the terminal dashboard).
enum Palette {
    static func stereo(_ state: String?) -> Color {
        switch state {
        case "stereo": .green
        case "mono_content": .yellow
        case "mono_stream": .cyan
        case "silence", nil: .gray.opacity(0.5)
        default: .red // out_of_phase, left_only, right_only
        }
    }

    static func stereoName(_ state: String?) -> String {
        switch state {
        case "stereo": "Stereo"
        case "mono_content": "Mono content (L = R)"
        case "mono_stream": "Mono stream"
        case "out_of_phase": "Out of phase"
        case "left_only": "Left only: right channel dead"
        case "right_only": "Right only: left channel dead"
        case "silence": "Silence"
        default: "—"
        }
    }

    static func delivery(_ code: String?) -> Color {
        switch code {
        case "ok": .green
        case "stall", "decode_error": .yellow
        case "underrun", "disconnect": .red
        default: .clear
        }
    }

    static func deliveryName(_ code: String?) -> String {
        switch code {
        case "ok": "OK"
        case "stall": "Stall"
        case "underrun": "Listener underrun"
        case "disconnect": "Disconnect"
        case "decode_error": "Decoder error"
        default: "—"
        }
    }

    /// The terminal's level colour: green inside -24…-7 LUFS, yellow outside.
    static func loudness(_ v: Double) -> Color { (-24...(-7)).contains(v) ? .green : .yellow }
}

func db(_ v: Double?, _ unit: String = "") -> String {
    guard let v else { return "—" }
    return String(format: "%.1f", v) + (unit.isEmpty ? "" : " \(unit)")
}
