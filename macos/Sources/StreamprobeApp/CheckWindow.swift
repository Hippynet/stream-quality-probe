import StreamprobeKit
import SwiftUI

enum DetailTab: String, CaseIterable, Identifiable {
    case live = "Live", details = "Details", events = "Events"
    var id: String { rawValue }
}

struct CheckWindow: View {
    let initialURL: String?

    @State private var session = CheckSession()
    @State private var urlText = ""
    @State private var options = CheckOptions()
    @State private var showOptions = false
    @State private var selectedFinding: String?
    @State private var tab: DetailTab = .live
    @State private var helperMissing = false
    @State private var started = false

    var body: some View {
        NavigationSplitView {
            SidebarView(session: session, selection: $selectedFinding)
                .navigationSplitViewColumnWidth(min: 260, ideal: 310, max: 440)
        } detail: {
            detail
        }
        .navigationTitle(windowTitle)
        .modifier(HideToolbarTitle())
        .toolbar { toolbar }
        .focusedSceneValue(\.checkSession, session)
        .onAppear {
            guard !started else { return }
            started = true
            #if DEBUG
            if let t = DebugHooks.tab { tab = t }
            #endif
            if let u = initialURL {
                urlText = u
                check()
            }
        }
        .onDisappear { session.stop() }
        .onChange(of: session.phase) { _, new in
            if new == .connecting { selectedFinding = nil }
            #if DEBUG
            if new != .connecting, selectedFinding == nil, let code = DebugHooks.select { selectedFinding = code }
            #endif
        }
    }

    // MARK: detail

    @ViewBuilder private var detail: some View {
        VStack(spacing: 0) {
            if let banner = banner {
                banner.padding([.horizontal, .top], 14)
            }
            if session.phase == .idle && !helperMissing {
                ContentUnavailableView(
                    "Check a stream",
                    systemImage: "waveform.badge.magnifyingglass",
                    description: Text("Paste an Icecast, Shoutcast, playlist or HLS URL above and press Return."))
            } else if session.phase != .idle {
                Picker("View", selection: $tab) {
                    ForEach(DetailTab.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .padding(.top, 12)
                switch tab {
                case .live: LiveTab(session: session, highlight: highlight)
                case .details: DetailsTab(report: session.report, startInfo: session.startInfo)
                case .events: EventsTab(events: session.events, isRunning: session.isActive)
                }
            }
            Spacer(minLength: 0)
        }
        .frame(minWidth: 560, minHeight: 480)
    }

    private var highlight: FindingHighlight {
        guard let code = selectedFinding else { return .none }
        let now = session.live?.elapsedS ?? session.report?.durationS ?? 0
        return .highlight(for: code, events: session.events, timeline: session.report?.timeline, now: now)
    }

    private var banner: PhaseBanner? {
        if helperMissing {
            return PhaseBanner(title: "streamprobe helper not found",
                               message: Text("Build the app with `make mac`, or set STREAMPROBE_BIN."), tail: [])
        }
        switch session.phase {
        case .failed(let e) where e.code == "ffmpeg_missing":
            return PhaseBanner(title: "ffmpeg isn't installed",
                               message: Text("Install it with `brew install ffmpeg`, then check again."), tail: [])
        case .failed(let e):
            return PhaseBanner(title: "Couldn't run the check", message: untrusted(e.message), tail: session.stderrTail)
        case .stoppedWhileConnecting:
            return PhaseBanner(title: "Stopped while connecting",
                               message: Text("No report: the stream hadn't connected yet."), tail: session.stderrTail)
        case .endedUnexpectedly(let status):
            return PhaseBanner(title: "Check ended unexpectedly",
                               message: Text("streamprobe exited (status \(status)) without a final report. The last report so far is shown."),
                               tail: session.stderrTail)
        default:
            return nil
        }
    }

    // MARK: toolbar

    @ToolbarContentBuilder private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            HStack(spacing: 10) {
                TextField("Stream URL", text: $urlText, prompt: Text("https://stream.example.com/live"))
                    .textFieldStyle(.roundedBorder)
                    .frame(minWidth: 240, idealWidth: 380, maxWidth: 520)
                    .disabled(session.isActive)
                    .onSubmit(check)
                    .dropDestination(for: URL.self) { urls, _ in
                        guard let u = urls.first, ["http", "https"].contains(u.scheme?.lowercased() ?? "") else { return false }
                        urlText = u.absoluteString
                        return true
                    }
                StatusLabel(session: session)
            }
        }
        ToolbarItemGroup(placement: .primaryAction) {
            Button { showOptions.toggle() } label: { Label("Options", systemImage: "slider.horizontal.3") }
                .help("Duration and connection options")
                .popover(isPresented: $showOptions, arrowEdge: .bottom) { OptionsPopover(options: $options) }
                .disabled(session.isActive)
            if session.isActive {
                Button(action: session.stop) { Label("Stop", systemImage: "stop.fill") }
                    .keyboardShortcut(".", modifiers: .command)
                    .help("Stop and show the final report (⌘.)")
            } else {
                Button(action: check) {
                    Label(session.result != nil ? "Check Again" : "Check", systemImage: "play.fill")
                }
                .disabled(!CheckRequest(url: urlText).isValid)
                .help("Check this stream (Return)")
                if session.result != nil {
                    Menu {
                        Button("Save Report as Text…") { ReportSaver.save(session, as: .text) }
                        Button("Save Report as JSON…") { ReportSaver.save(session, as: .json) }
                    } label: {
                        Label("Save Report", systemImage: "square.and.arrow.down")
                    }
                    .help("Save the report (⌘S text, ⇧⌘S JSON)")
                }
            }
        }
    }

    private var windowTitle: String {
        if let u = session.startInfo?.url ?? session.request?.trimmedURL,
           let host = URL(string: u.contains("://") ? u : "http://\(u)")?.host() {
            return host
        }
        return "Streamprobe"
    }

    private func check() {
        let req = CheckRequest(url: urlText, durationS: options.durationS,
                               insecure: options.insecure, allowInternal: options.allowInternal)
        guard req.isValid, !session.isActive else { return }
        guard let bin = ProbeBinary.locate() else {
            helperMissing = true
            return
        }
        helperMissing = false
        session.start(req, binary: bin)
    }
}

struct StatusLabel: View {
    let session: CheckSession

    var body: some View {
        Group {
            switch session.phase {
            case .connecting:
                HStack(spacing: 6) {
                    ProgressView().controlSize(.small)
                    Text("Connecting…")
                }
            case .running:
                let live = session.live?.phase == "live"
                Text("● \(live ? "LIVE" : "DOWN")  \(clock(session.live?.elapsedS ?? 0))")
                    .foregroundStyle(live ? .green : .red)
            case .finished:
                Text("Finished · \(clock(session.result?.report.durationS ?? 0))")
                    .foregroundStyle(.secondary)
            default:
                EmptyView()
            }
        }
        .font(.callout.monospacedDigit().weight(.medium))
        .fixedSize()
    }
}

func clock(_ s: Double) -> String {
    let t = Int(max(0, s))
    return t >= 3600 ? String(format: "%d:%02d:%02d", t / 3600, t % 3600 / 60, t % 60)
        : String(format: "%d:%02d", t / 60, t % 60)
}

struct PhaseBanner: View {
    let title: String
    let message: Text
    let tail: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: "exclamationmark.triangle.fill")
                .font(.headline)
                .foregroundStyle(.orange)
            message
            if !tail.isEmpty {
                ScrollView(.horizontal) {
                    untrusted(tail.suffix(8).joined(separator: "\n"))
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(.orange.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }
}

/// The host is the window's title (Window menu, Mission Control) but the toolbar
/// needs the room for the URL field and buttons.
struct HideToolbarTitle: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 15, *) {
            content.toolbar(removing: .title)
        } else {
            content
        }
    }
}
