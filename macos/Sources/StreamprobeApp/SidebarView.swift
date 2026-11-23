import StreamprobeKit
import SwiftUI

struct SidebarView: View {
    let session: CheckSession
    @Binding var selection: String?

    private let groups: [(Level, String)] = [(.bad, "Problems"), (.warn, "Warnings"), (.info, "Info"), (.ok, "OK")]

    var body: some View {
        VStack(spacing: 0) {
            VerdictBadge(session: session)
                .padding(.horizontal, 10)
                .padding(.top, 8)
            List(selection: $selection) {
                ForEach(groups, id: \.0) { level, name in
                    let items = session.findings.filter { $0.level == level }
                    if !items.isEmpty {
                        Section(name) {
                            ForEach(Array(items.enumerated()), id: \.offset) { _, f in
                                FindingRow(finding: f).tag(f.code)
                            }
                        }
                    }
                }
            }
            .listStyle(.sidebar)
            if let code = selection, let f = session.findings.first(where: { $0.code == code }) {
                ExplanationPanel(finding: f, explanation: explanations[code])
                    .padding(10)
            }
        }
    }
}

struct FindingRow: View {
    let finding: Finding

    var body: some View {
        Label {
            untrusted(finding.message).lineLimit(3)
        } icon: {
            Image(systemName: finding.level.symbol).foregroundStyle(finding.level.color)
        }
        .help(Text(verbatim: finding.message))
    }
}

struct ExplanationPanel: View {
    let finding: Finding
    let explanation: Explanation?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            untrusted(finding.message).font(.callout.weight(.semibold))
            Text(verbatim: finding.code).font(.caption.monospaced()).foregroundStyle(.secondary)
            if let e = explanation {
                Text(verbatim: e.meaning.prefix(1).uppercased() + e.meaning.dropFirst() + ".")
                    .font(.callout)
                if !e.check.isEmpty {
                    Text("What to check").font(.caption.weight(.semibold)).foregroundStyle(.secondary).padding(.top, 2)
                    Text(verbatim: e.check.prefix(1).uppercased() + e.check.dropFirst() + ".").font(.callout)
                }
            }
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
    }
}
