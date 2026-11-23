import StreamprobeKit
import SwiftUI

struct VerdictBadge: View {
    let session: CheckSession

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 26, weight: .semibold))
                .foregroundStyle(color)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: title).font(.title3.weight(.semibold))
                if let subtitle { Text(verbatim: subtitle).font(.caption).foregroundStyle(.secondary) }
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .background(color.opacity(0.12), in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(color.opacity(0.35)))
    }

    private var verdict: String? {
        switch session.phase {
        case .running, .finished, .endedUnexpectedly: session.verdict
        default: nil
        }
    }

    private var title: String { session.verdictTitle }

    private var subtitle: String? {
        let counts = Dictionary(grouping: session.findings, by: \.level).mapValues(\.count)
        let parts = [(Level.bad, "problem"), (.warn, "warning")].compactMap { level, noun -> String? in
            guard let n = counts[level], n > 0 else { return nil }
            return "\(n) \(noun)\(n == 1 ? "" : "s")"
        }
        guard verdict != nil else { return nil }
        if !session.verdictIsFinal { return (parts.isEmpty ? "nothing wrong" : parts.joined(separator: " · ")) + " so far" }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private var symbol: String {
        switch verdict {
        case "healthy": "checkmark.seal.fill"
        case "warnings": "exclamationmark.triangle.fill"
        case "problems": "xmark.octagon.fill"
        default: "waveform"
        }
    }

    private var color: Color {
        switch verdict {
        case "healthy": .green
        case "warnings": .yellow
        case "problems": .red
        default: .secondary
        }
    }
}

extension Level {
    var symbol: String {
        switch self {
        case .bad: "xmark.octagon.fill"
        case .warn: "exclamationmark.triangle.fill"
        case .info: "info.circle.fill"
        case .ok: "checkmark.circle.fill"
        }
    }

    var color: Color {
        switch self {
        case .bad: .red
        case .warn: .yellow
        case .info: .blue
        case .ok: .green
        }
    }
}
