import SwiftUI

/// A horizontal level meter, -60…0 dBFS, with a peak-hold tick.
struct MeterView: View {
    let label: String
    let rms: Double?
    let peak: Double?

    @State private var held: Double = -200
    @State private var heldAt = Date.distantPast

    private static let floor = -60.0

    var body: some View {
        HStack(spacing: 8) {
            Text(label).font(.caption.weight(.semibold)).foregroundStyle(.secondary).frame(width: 12)
            GeometryReader { g in
                let w = g.size.width
                ZStack(alignment: .leading) {
                    Capsule().fill(.quaternary)
                    LinearGradient(stops: [
                        .init(color: .green, location: 0),
                        .init(color: .green, location: 0.7),
                        .init(color: .yellow, location: 0.85),
                        .init(color: .red, location: 1),
                    ], startPoint: .leading, endPoint: .trailing)
                    .mask(alignment: .leading) {
                        Capsule().frame(width: w * Self.fraction(rms))
                    }
                    if held > Self.floor {
                        Rectangle().fill(held > -1 ? Color.red : .primary)
                            .frame(width: 2)
                            .offset(x: max(0, w * Self.fraction(held) - 2))
                    }
                }
            }
            .frame(height: 10)
            .animation(.linear(duration: 0.2), value: rms)
            Text(db(rms)).font(.caption.monospacedDigit()).frame(width: 40, alignment: .trailing)
            Text("pk \(db(peak))").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                .frame(width: 58, alignment: .trailing)
        }
        .onChange(of: peak) { _, p in
            guard let p else {
                held = -200 // silence / no audio: no stale tick
                return
            }
            if p >= held || Date().timeIntervalSince(heldAt) > 1.5 {
                held = p
                heldAt = Date()
            }
        }
    }

    static func fraction(_ v: Double?) -> CGFloat {
        guard let v else { return 0 }
        return CGFloat(min(1, max(0, (v - floor) / -floor)))
    }
}
