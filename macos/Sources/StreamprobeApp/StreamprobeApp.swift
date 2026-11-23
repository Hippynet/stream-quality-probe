import AppKit
import StreamprobeKit
import SwiftUI

@main
struct StreamprobeApp: App {
    init() {
        // `swift run` starts us as a plain process: make it a proper foreground app.
        NSApplication.shared.setActivationPolicy(.regular)
        DispatchQueue.main.async { NSApplication.shared.activate() }
        #if DEBUG
        DispatchQueue.main.async { DebugHooks.startSnapshots() }
        #endif
    }

    var body: some Scene {
        WindowGroup {
            CheckWindow(initialURL: LaunchURL.take())
        }
        .defaultSize(width: 1120, height: 780)
        .commands {
            CommandGroup(replacing: .saveItem) { SaveCommands() }
        }
    }
}

/// `Streamprobe <url>` (or `open -a Streamprobe --args <url>`): the first window checks it at once.
@MainActor
enum LaunchURL {
    private static var pending: String? = CommandLine.arguments.dropFirst().first { !$0.hasPrefix("-") }

    static func take() -> String? {
        defer { pending = nil }
        return pending
    }
}

/// Finding explanations from the bundled docs/FINDINGS.md, read once.
@MainActor
let explanations = FindingExplanations.load()

struct CheckSessionKey: FocusedValueKey {
    typealias Value = CheckSession
}

extension FocusedValues {
    var checkSession: CheckSession? {
        get { self[CheckSessionKey.self] }
        set { self[CheckSessionKey.self] = newValue }
    }
}

struct SaveCommands: View {
    @FocusedValue(\.checkSession) private var session

    var body: some View {
        Button("Save Report as Text…") { if let s = session { ReportSaver.save(s, as: .text) } }
            .keyboardShortcut("s")
            .disabled(session?.result == nil)
        Button("Save Report as JSON…") { if let s = session { ReportSaver.save(s, as: .json) } }
            .keyboardShortcut("s", modifiers: [.command, .shift])
            .disabled(session?.result == nil)
    }
}
