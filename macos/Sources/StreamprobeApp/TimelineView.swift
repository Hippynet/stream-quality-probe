import StreamprobeKit
import SwiftUI

/// The whole session, one column per timeline point, rows as in the terminal:
/// level, stereo, clip, delivery, track (title changes).
struct TimelineView: View {
    let timeline: Timeline
    let highlight: FindingHighlight
    let duration: Double

    /// Seconds into the check under the pointer (a time, not an index: see Timeline.pointIndex).
    @State private var hoverTime: Double?

    private var hover: Int? { hoverTime.flatMap { timeline.pointIndex(at: $0) } }

    private static let rows: [(TimelineRow, String)] =
        [(.level, "Level"), (.stereo, "Stereo"), (.clip, "Clip"), (.delivery, "Delivery"), (.track, "Track")]
    private static let rowH: CGFloat = 20
    private static let gap: CGFloat = 6
    private static let labelW: CGFloat = 64
    private static var plotH: CGFloat { CGFloat(rows.count) * (rowH + gap) }

    private var span: Double {
        let last = timeline.points.last.map { Double($0.offsetS + timeline.resolutionS) } ?? 1
        return max(duration, last, 1)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .top, spacing: 0) {
                VStack(alignment: .leading, spacing: Self.gap) {
                    ForEach(Self.rows, id: \.1) { _, name in
                        Text(name).font(.caption).foregroundStyle(.secondary).frame(height: Self.rowH)
                    }
                }
                .frame(width: Self.labelW, alignment: .leading)
                GeometryReader { g in
                    Canvas { ctx, size in draw(&ctx, size) }
                        .onContinuousHover { phase in
                            switch phase {
                            case .active(let p): hoverTime = Double(p.x / max(1, g.size.width)) * span
                            case .ended: hoverTime = nil
                            }
                        }
                        .overlay(alignment: .topLeading) { tooltip(width: g.size.width) }
                }
                .frame(height: Self.plotH)
            }
            axis.padding(.leading, Self.labelW)
            legend.padding(.leading, Self.labelW).padding(.top, 4)
        }
    }

    // MARK: drawing

    private func x(_ t: Double, _ w: CGFloat) -> CGFloat { CGFloat(t / span) * w }

    private func rowY(_ r: TimelineRow) -> CGFloat {
        CGFloat(Self.rows.firstIndex { $0.0 == r }!) * (Self.rowH + Self.gap)
    }

    private func draw(_ ctx: inout GraphicsContext, _ size: CGSize) {
        let w = size.width, h = Self.rowH
        let res = Double(timeline.resolutionS)
        for r in Self.rows.map(\.0) {
            ctx.fill(Path(roundedRect: CGRect(x: 0, y: rowY(r), width: w, height: h), cornerRadius: 3),
                     with: .color(.gray.opacity(0.08)))
        }
        for p in timeline.points {
            let x0 = x(Double(p.offsetS), w)
            let cw = max(1, x(Double(p.offsetS) + res, w) - x0 - (w / CGFloat(span / res) > 4 ? 1 : 0))
            // level
            let silent = (p.lufsShort != nil || p.rmsDbfs != nil) && p.silentS >= min(res * 0.5, 3)
            if silent {
                ctx.fill(Path(CGRect(x: x0, y: rowY(.level) + h - 3, width: cw, height: 3)), with: .color(.red))
            } else if let v = p.lufsShort ?? p.rmsDbfs.map({ $0 + 3 }) {
                let f = CGFloat(min(1, max(0.08, (v + 40) / 35)))
                ctx.fill(Path(CGRect(x: x0, y: rowY(.level) + h * (1 - f), width: cw, height: h * f)),
                         with: .color(Palette.loudness(v)))
            }
            // stereo
            if let s = p.stereo {
                ctx.fill(Path(CGRect(x: x0, y: rowY(.stereo) + 3, width: cw, height: h - 6)),
                         with: .color(Palette.stereo(s)))
            }
            // clip
            if p.clippedSamples > 0 {
                ctx.fill(Path(CGRect(x: x0, y: rowY(.clip) + 3, width: cw, height: h - 6)),
                         with: .color(p.clippedSamples > 20 ? .red : .yellow))
            }
            // delivery
            if let d = p.delivery {
                let tall = d != "ok"
                let hh: CGFloat = tall ? h - 6 : 3
                ctx.fill(Path(CGRect(x: x0, y: rowY(.delivery) + (h - hh) / 2, width: cw, height: hh)),
                         with: .color(Palette.delivery(d)))
            }
            // track
            if p.titleChange {
                let cx = x0 + cw / 2, cy = rowY(.track) + h / 2
                ctx.fill(Path(ellipseIn: CGRect(x: cx - 3.5, y: cy - 3.5, width: 7, height: 7)), with: .color(.purple))
            }
        }
        // selected finding
        if let row = highlight.row {
            let y = rowY(row) - 2
            if highlight.ranges.isEmpty {
                ctx.stroke(Path(roundedRect: CGRect(x: 0, y: y, width: w, height: h + 4), cornerRadius: 4),
                           with: .color(.accentColor), lineWidth: 2)
            }
            for r in highlight.ranges {
                let x0 = x(r.lowerBound, w), x1 = max(x(r.upperBound, w), x0 + 3)
                let rect = CGRect(x: x0 - 1, y: y, width: x1 - x0 + 2, height: h + 4)
                ctx.fill(Path(roundedRect: rect, cornerRadius: 3), with: .color(.accentColor.opacity(0.25)))
                ctx.stroke(Path(roundedRect: rect, cornerRadius: 3), with: .color(.accentColor), lineWidth: 1.5)
            }
        }
        if let i = hover {
            let p = timeline.points[i]
            let x0 = x(Double(p.offsetS), w)
            ctx.fill(Path(CGRect(x: x0, y: 0, width: max(1, x(Double(p.offsetS) + res, w) - x0), height: Self.plotH - Self.gap)),
                     with: .color(.primary.opacity(0.12)))
        }
    }

    // MARK: hover, axis, legend

    @ViewBuilder private func tooltip(width: CGFloat) -> some View {
        if let i = hover {
            let p = timeline.points[i]
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: p.t).font(.caption.weight(.semibold).monospacedDigit())
                Text("Loudness \(db(p.lufsShort, "LUFS")) · RMS \(db(p.rmsDbfs, "dBFS"))")
                if p.silentS > 0 { Text(String(format: "Silent %.1f s", p.silentS)) }
                Text("Stereo: \(Palette.stereoName(p.stereo))")
                if p.clippedSamples > 0 { Text("Clipped samples: \(p.clippedSamples)") }
                Text("Delivery: \(Palette.deliveryName(p.delivery))" + (p.decodeErrors > 0 ? " · \(p.decodeErrors) decode errors" : ""))
                if p.titleChange { Text("Title changed") }
            }
            .font(.caption)
            .padding(8)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
            .fixedSize()
            .offset(x: min(max(0, x(Double(p.offsetS), width) + 12), max(0, width - 230)), y: Self.plotH + 4)
            .allowsHitTesting(false)
        }
    }

    private var axis: some View {
        GeometryReader { g in
            let n = max(2, min(6, Int(g.size.width / 110)))
            let pts = timeline.points
            ZStack(alignment: .topLeading) {
                ForEach(0..<n, id: \.self) { k in
                    let p = pts[min(pts.count - 1, k * (pts.count - 1) / max(1, n - 1))]
                    Text(verbatim: p.t)
                        .font(.caption2.monospacedDigit()).foregroundStyle(.secondary)
                        .fixedSize()
                        .offset(x: min(x(Double(p.offsetS), g.size.width), g.size.width - 48))
                }
            }
        }
        .frame(height: 14)
    }

    private var legend: some View {
        HStack(spacing: 14) {
            Swatch(.green, "stereo / OK")
            Swatch(.yellow, "mono content / loud / stall")
            Swatch(.cyan, "mono stream")
            Swatch(.red, "phase, one channel, clipping, underrun, disconnect")
            Swatch(.purple, "title change")
            Text("1 column = \(timeline.resolutionS) s").foregroundStyle(.secondary)
        }
        .font(.caption2)
    }
}

private struct Swatch: View {
    let color: Color
    let text: String
    init(_ color: Color, _ text: String) {
        self.color = color
        self.text = text
    }

    var body: some View {
        HStack(spacing: 4) {
            RoundedRectangle(cornerRadius: 2).fill(color).frame(width: 9, height: 9)
            Text(text).foregroundStyle(.secondary)
        }
    }
}
