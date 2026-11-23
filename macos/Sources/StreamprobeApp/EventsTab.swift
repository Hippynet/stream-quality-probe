import StreamprobeKit
import SwiftUI

/// The timestamped event log. Follows new events while you're at the bottom;
/// scroll up and it stays put.
struct EventsTab: View {
    let events: [ProbeEvent]
    let isRunning: Bool

    @State private var atBottom = true

    /// In time order, as the terminal shows them: start events are backdated to when a condition began.
    private var sorted: [ProbeEvent] {
        events.enumerated().sorted { ($0.element.offsetS, $0.offset) < ($1.element.offsetS, $1.offset) }.map(\.element)
    }

    var body: some View {
        if events.isEmpty {
            Text(isRunning ? "No events yet" : "No events")
                .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(sorted.enumerated()), id: \.offset) { i, e in
                            HStack(alignment: .firstTextBaseline, spacing: 10) {
                                Text(verbatim: e.t).font(.callout.monospacedDigit()).foregroundStyle(.secondary)
                                Image(systemName: e.level.symbol).foregroundStyle(e.level.color).font(.caption)
                                untrusted(e.message).frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .padding(.vertical, 4)
                            .padding(.horizontal, 12)
                            .background(i.isMultiple(of: 2) ? Color.clear : Color.primary.opacity(0.03))
                        }
                        Color.clear.frame(height: 1).id("bottom")
                            .onAppear { atBottom = true }
                            .onDisappear { atBottom = false }
                    }
                    .textSelection(.enabled)
                    .padding(.vertical, 8)
                }
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
                .onChange(of: events.count) {
                    if atBottom && isRunning { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
        }
    }
}
