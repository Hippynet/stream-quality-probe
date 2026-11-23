#if DEBUG
import AppKit
import SwiftUI

/// Development only (not in release builds): lets a script see the app without
/// screen-recording permission or clicks.
///   STREAMPROBE_SNAPSHOT=/dir   write /dir/window-<n>.png of the key window every 5 s
///   STREAMPROBE_SELECT=code     pre-select that finding in the sidebar
///   STREAMPROBE_TAB=details     start on that tab (live, details, events)
@MainActor
enum DebugHooks {
    static let env = ProcessInfo.processInfo.environment
    static var select: String? { env["STREAMPROBE_SELECT"] }
    static var tab: DetailTab? { env["STREAMPROBE_TAB"].flatMap { t in DetailTab.allCases.first { $0.rawValue.lowercased() == t } } }

    static func startSnapshots() {
        guard let dir = env["STREAMPROBE_SNAPSHOT"] else { return }
        var n = 0
        Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in
            MainActor.assumeIsolated {
                guard let win = NSApp.windows.first(where: { $0.isVisible && $0.contentView != nil }),
                      let frameView = win.contentView?.superview else { return }
                let rect = frameView.bounds
                guard let rep = frameView.bitmapImageRepForCachingDisplay(in: rect) else { return }
                frameView.cacheDisplay(in: rect, to: rep)
                n += 1
                try? rep.representation(using: .png, properties: [:])?
                    .write(to: URL(fileURLWithPath: dir).appending(path: String(format: "window-%03d.png", n)))
            }
        }
    }
}
#endif
