import StreamprobeKit
import SwiftUI

struct LiveTab: View {
    let session: CheckSession
    let highlight: FindingHighlight

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                let live = session.live
                Grid(horizontalSpacing: 12, verticalSpacing: 12) {
                    GridRow {
                        Card("Levels") { levels(live) }.gridCellColumns(2)
                        Card("Loudness") { loudness(live?.loudness) }
                    }
                    GridRow {
                        Card("Stereo") { stereo(live?.stereo) }
                        Card("Delivery") { delivery(live?.delivery) }
                        Card("Quality") { quality(live?.quality) }
                    }
                }
                Card("Timeline") {
                    if let t = session.report?.timeline, !t.points.isEmpty {
                        TimelineView(timeline: t, highlight: highlight,
                                     duration: max(session.report?.durationS ?? 0, session.live?.elapsedS ?? 0))
                    } else {
                        Text("Starts once audio arrives").foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, minHeight: 120)
                    }
                }
                Card("Now playing") { nowPlaying(live?.nowPlaying) }
            }
            .padding(16)
        }
    }

    // MARK: cards

    @ViewBuilder private func levels(_ live: LiveSnapshot?) -> some View {
        if let l = live?.levels {
            VStack(spacing: 8) {
                MeterView(label: "L", rms: l.rmsL, peak: l.peakL)
                MeterView(label: "R", rms: l.rmsR, peak: l.peakR)
            }
        } else if session.sawAudio && session.isActive {
            Label("No audio arriving", systemImage: "speaker.slash.fill")
                .foregroundStyle(.red).font(.headline)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        } else {
            Text(session.isActive ? "Waiting for audio…" : "—").foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
        }
    }

    private func loudness(_ l: LiveSnapshot.Loudness?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Big(db(l?.integratedLufs), unit: "LUFS integrated")
            Small([("Momentary", db(l?.momentaryLufs)), ("Short-term", db(l?.shortTermLufs))])
            Small([("LRA", db(l?.lraLu, "LU")), ("True peak", db(l?.truePeakDbtp, "dBTP"))])
        }
    }

    private func stereo(_ s: LiveSnapshot.Stereo?) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(Palette.stereoName(s?.state))
                .font(.title3.weight(.semibold))
                .foregroundStyle(s?.state == nil ? .secondary : Palette.stereo(s?.state))
            Small([("Correlation", s?.correlation.map { String(format: "%+.3f", $0) } ?? "—"),
                   ("Side/mid", db(s?.sideDb, "dB"))])
        }
    }

    private func delivery(_ d: LiveSnapshot.Delivery?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            let buf = d?.listenerBufferS ?? 0
            Big(d.map { String(format: "%.1f s", $0.listenerBufferS) } ?? "—", unit: "listener buffer",
                color: d == nil ? .secondary : buf > 1 ? .green : buf > 0 ? .yellow : .red)
            Small([("Realtime", d?.realtimeRatio.map { String(format: "%.2f×", $0) } ?? "—"),
                   ("In", d?.inputKbps.map { String(format: "%.0f kb/s", $0) } ?? "—")])
            Counts([("Underruns", d?.underruns, (d?.underruns ?? 0) > 0), ("Stalls", d?.stalls, (d?.stalls ?? 0) > 0),
                    ("Disconnects", d?.disconnects, (d?.disconnects ?? 0) > 0)])
        }
    }

    private func quality(_ q: LiveSnapshot.Quality?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Big(q?.bandwidthHz.map { String(format: "~%.1f kHz", $0 / 1000) } ?? (session.isActive ? "measuring…" : "—"), unit: "bandwidth")
            Small([("Max peak", db(q?.maxPeakDbfs, "dBFS")), ("Full-scale", q.map { "\($0.samplesAtFullScale)" } ?? "—")])
            Counts([("Overs >0 dBFS", q?.samplesOver0Dbfs, (q?.samplesOver0Dbfs ?? 0) > 50),
                    ("Decode errors", q?.decodeErrors, (q?.decodeErrors ?? 0) > 0)])
        }
    }

    @ViewBuilder private func nowPlaying(_ n: LiveSnapshot.NowPlaying?) -> some View {
        if let title = n?.title {
            VStack(alignment: .leading, spacing: 3) {
                untrusted(title).font(.title3.weight(.medium)).foregroundStyle(.purple).textSelection(.enabled)
                HStack(spacing: 12) {
                    if let s = n?.sinceS { Text("since \(clock(s)) into the check") }
                    if let p = n?.previous { Text("previous: ") + untrusted(p) }
                }
                .font(.caption).foregroundStyle(.secondary)
            }
        } else {
            Text("No title").foregroundStyle(.secondary)
        }
    }
}

// MARK: building blocks

struct Card<Content: View>: View {
    let title: String
    let content: Content

    init(_ title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title.uppercased()).font(.caption2.weight(.semibold)).foregroundStyle(.secondary).kerning(0.6)
            content
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .padding(12)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.separator.opacity(0.6)))
    }
}

private struct Big: View {
    let value: String
    let unit: String
    var color: Color = .primary

    init(_ value: String, unit: String, color: Color = .primary) {
        self.value = value
        self.unit = unit
        self.color = color
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(value).font(.title2.weight(.semibold).monospacedDigit()).foregroundStyle(color)
            Text(unit).font(.caption).foregroundStyle(.secondary)
        }
    }
}

private struct Small: View {
    let items: [(String, String)]
    init(_ items: [(String, String)]) { self.items = items }

    var body: some View {
        HStack(spacing: 12) {
            ForEach(items, id: \.0) { k, v in
                HStack(spacing: 4) {
                    Text(k).foregroundStyle(.secondary)
                    Text(v).monospacedDigit()
                }
                .fixedSize()
            }
        }
        .font(.caption)
    }
}

/// Counters, red where the dashboard shows them red.
private struct Counts: View {
    let items: [(String, Int?, Bool)]
    init(_ items: [(String, Int?, Bool)]) { self.items = items }

    var body: some View {
        HStack(spacing: 12) {
            ForEach(Array(items.enumerated()), id: \.offset) { _, item in
                HStack(spacing: 4) {
                    Text(item.0).foregroundStyle(.secondary)
                    Text(item.1.map(String.init) ?? "—").monospacedDigit()
                        .foregroundStyle(item.2 ? .red : .primary)
                }
            }
        }
        .font(.caption)
    }
}
