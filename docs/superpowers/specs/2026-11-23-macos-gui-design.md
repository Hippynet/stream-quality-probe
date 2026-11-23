# streamprobe for Mac: design

Date: 2026-11-23 · Status: draft for approval

## Intent

streamprobe (by Oliver Wilkinson) is a Rust CLI that connects to an internet radio stream like a listener's player, decodes it with ffmpeg, and reports what's wrong: connection/TLS problems, format and header issues, mono/out-of-phase/one-sided audio, clipping, poor quality, dead air, stalls, underruns and disconnects. It already runs on macOS in a terminal.

Ben wants a **native macOS app with a proper Mac GUI**, built on this machine, for **spot-checking a stream**: someone reports a problem, you paste the URL, watch it for a bit, and see what's wrong. It's **for Ben only, for now**.

## Goals

- One window per check: paste a URL, press Check, watch it live, stop, read the verdict, save the report.
- The verdict ("what's wrong?") is always visible while the check runs (layout B, below).
- Diagnostics identical to the CLI: same measurements, same finding codes and messages, same thresholds.
- Builds and runs on this Mac with one command.

## Non-goals

- Monitoring several streams, menu-bar mode, background checks, notifications, scheduling.
- Distribution to other people: no Developer ID signing, notarisation, DMG or auto-update. Ad-hoc signed local build only.
- Bundling ffmpeg. The app uses the Homebrew ffmpeg already on this Mac.
- A Linux or Windows GUI.
- Changing any analysis, threshold or finding in the Rust code.

## Anti-goals (would be failure even if it "works")

- Any diagnostic logic re-implemented in Swift, so the app and CLI could disagree. Swift only displays what streamprobe reports.
- Any change to the CLI's existing behaviour or output. The new mode is opt-in behind a new flag; existing flags, text/JSON reports and exit codes are untouched.
- A window that freezes or stutters during a long check.
- Rendering server-controlled text (titles, headers, ICY fields, issuer, ffprobe output) as anything but plain text: no Markdown, no clickable links (see the README's `untrusted_fields`).

## Constraints

- Machine: Apple silicon, macOS 27.2, Xcode installed at `/Applications/Xcode.app` but `xcode-select` points at the Command Line Tools. Don't change `xcode-select` (needs sudo, affects the whole machine); set `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer` in the build instead.
- Toolchain present: Swift 6.4, Rust 1.99 (project needs ≥ 1.88), ffmpeg + ffprobe at `/opt/homebrew/bin`. `decoder::find_tool` already searches `/opt/homebrew/bin`, so a Finder-launched app (minimal `PATH`) still finds ffmpeg.
- The repo's `make lint` (rustfmt check, clippy `-D warnings`) must keep passing.
- It's Oliver's codebase: keep the Rust change small, self-contained and upstreamable.

## Approaches considered

1. **SwiftUI app that runs the `streamprobe` binary as a subprocess (chosen).** Add one opt-in CLI flag that streams live state as JSON lines; the app launches the binary, decodes the lines and renders them. Oliver's analysis is untouched and stays the single source of truth; the CLI behaves exactly as before. Cost: two programs to build, handled by one build command.
2. **SwiftUI app with the Rust engine linked in as a library (FFI).** One process and a tidier bundle, but Oliver's orchestration in `main.rs` (thread start-up, reconnects, Ctrl-C handling) would have to move into a library and be wrapped for Swift. Roughly twice the work, more of his code touched, no visible benefit for spot-checks. Approach 1 can migrate to this later without redoing the GUI.
3. **Rust GUI toolkit or web-view shell (egui, iced, Tauri).** Rejected: none looks or behaves like a native Mac app, which is the point of the request.

Ben picked approach 1.

## Design

### Where it lives

A new `macos/` directory in this repo containing a Swift Package (`macos/Package.swift`) for the app, plus a `make mac` target. Work happens on a branch, not `main`. *(Claude's call: same repo so the CLI flag and the app evolve together.)*

### 1. The CLI change: `--live-json`

New flag `--live-json` (hidden from `--help` is acceptable, but documenting it in the README is preferred). When set:

- No dashboard, no ANSI colour, no progress text on stdout. stdout carries **only newline-delimited JSON objects**, one per line. Anything human goes to stderr.
- Implies no time limit unless `-t` is given (the app stops it), regardless of whether stdin/stdout are TTYs.
- Line types (each has a `"type"` field):
  - `{"type":"start", "version", "url"}` once, immediately.
  - `{"type":"live", ...}` every 250 ms: the *current* values the terminal dashboard shows, i.e. connection phase (connecting / live / reconnecting), elapsed seconds, L/R RMS and peak dBFS, momentary / short-term / integrated LUFS, loudness range, true peak, current stereo class and L/R correlation, bandwidth, overs, decode errors, realtime ratio, input kb/s, simulated listener buffer seconds, underruns / stalls / disconnects counts, current and previous title. Plus any **new events** since the previous line (incremental, not the whole log).
  - `{"type":"report", "report": <json_report(st, now, "in progress")>}` about once per second, containing the timeline points, connection, headers, format, frames and ffprobe data exactly as `--json` produces them. (Rate-limited because the report includes the timeline and is heavier.)
  - `{"type":"final", "report": <json_report(...)>, "text": <ui::text_report(..) with colour off>, "exit_code": n}` once at the end, after which the process exits with the usual exit code.
- Stopping: the app sends `SIGINT`. Once streaming has started, the existing Ctrl-C handler sets the stop flag and the normal finish path runs, so the `final` line is written. **While still connecting** (DNS, TCP, TLS, headers, redirects, playlists: up to ~30 s), today's handler exits at once with code 130 and no report. Keep that behaviour: the app shows "Stopped while connecting" with the stderr tail. *(Claude's call: making the connect phase interruptible would mean changing Oliver's networking code, and stopping during connect means you've given up on that check anyway.)*
- Live findings: `live` lines also carry the **in-progress findings list computed with `findings(st, now, false)`**, exactly as the terminal dashboard does (`ui.rs`). The sidebar uses that list while running and the `final` report's findings once stopped. (`json_report` always uses `final_ = true`, which fires `no_title` and the no-audio finding early, so the `report` lines are not used for the sidebar while running.)
- `-o FILE`, `--insecure`, `--allow-internal` and `-t` work as today alongside it. `--live-json` conflicts with `--json` and `--no-ui` (clap `conflicts_with`).
- The exact field names are the builder's to choose, reusing the names already in `json_report` wherever they exist. Add a Rust unit test that a `live` line and a `final` line are each a single valid JSON object with the expected keys.

### 2. The app

SwiftUI, Swift 6, macOS 14 or later *(Claude's call: Observation framework and modern SwiftUI; this Mac is far newer)*. Dark and light mode follow the system.

**Window (layout B, chosen by Ben):**

```
┌ ● ● ●  [ http://stream.example.com/live          ] ● LIVE 1:45  [Stop] ┐
├────────────────────────┬───────────────────────────────────────────────┤
│ ✖ Problems             │  [ Live | Details | Events ]                  │
│ PROBLEMS               │  ┌Levels────┐ ┌Loudness──┐ ┌Delivery──┐       │
│ ✖ Listener underrun (1)│  │ L ▓▓▓▓▓░ │ │ −6.4 LUFS│ │ 10.4 s   │       │
│ ✖ Out of phase 14%     │  │ R ▓▓▓▓▓░ │ │ M −12 S… │ │ 1.14×    │       │
│ WARNINGS               │  └──────────┘ └──────────┘ └──────────┘       │
│ ▲ 2 decoder errors     │  Timeline  level  ███▁▁██▆▆▆  ███             │
│ OK                     │            stereo SSmmØØ–LLLSSSSSS            │
│ ✔ Connected · stereo   │            clip   ……………!!!!……                │
│ ┌────────────────────┐ │            deliv  ━━━━━━━━~u━━X━━            │
│ │ what it means +    │ │  Now playing: Artist – Title  since 14:56     │
│ │ what to check      │ │                                               │
└─┴────────────────────┴─┴───────────────────────────────────────────────┘
```

- **Toolbar:** URL field (accepts a typed, pasted or dropped http(s) URL, including playlist links, which streamprobe follows itself; local playlist *files* are not supported, since the CLI only takes URLs and a local `.m3u8` usually has relative paths *(Claude's call)*), live status and elapsed time, Check/Stop button (Return starts, ⌘. stops). A small options popover: duration (off / 30 s / 1 / 2 / 5 / 10 min / custom), "Ignore bad TLS certificate" (`--insecure`), "Allow internal addresses" (`--allow-internal`).
- **Sidebar:** the overall verdict (Healthy / Warnings / Problems, or "Checking…") and the findings grouped by level (problems, warnings, info, OK), worst first, from the latest `report` line. Selecting a finding shows a panel under the list with the finding's message, its code, and the "what it means / what to check" text for that code from `docs/FINDINGS.md`, and highlights on the timeline when it happened.
- **Live tab:** level meters (L/R RMS with peak hold), loudness (integrated large; momentary, short-term, LRA, true peak small), stereo state + correlation, delivery (listener buffer, realtime ratio, underruns/stalls/disconnects), quality (bandwidth, overs, decode errors), the timeline, and now playing.
- **Timeline:** drawn natively (SwiftUI `Canvas`), rows for level, stereo, clip, delivery and title changes, covering the whole session and auto-zoomed like the CLI. Times along the bottom; hover shows the values for that column. Highlight for a selected finding, using a small **display-only** table in Swift (which events or timeline row belong to which finding; it never decides whether something is a problem) *(Claude's call)*:
  - Same-code events with `X_end` pairs give ranges: `stall`, `dead_air`, `out_of_phase`, `mono_content`; `underrun` is an instant.
  - Renamed events: `dead_channel` → `left_only` / `right_only` ranges; `decode_errors` → `decode_error` instants; `disconnect` → from `disconnect` to `reconnected` / `reconnect_failed`.
  - No events, but a matching timeline row: `clipping` and `full_scale` highlight the clip row's marked columns; `slow_delivery` the delivery row; `partly_mono` / `mostly_silent` the stereo row's matching columns.
  - Anything else (e.g. `too_loud`, `frame_sync`, `format_change`, bandwidth, connection and header findings): no highlight; the explanation panel alone.
- **Details tab:** station (ICY name, genre, URL, description), server (Server header, status, Content-Type, metaint, CORS), connection timing (DNS, TCP, TLS, headers), TLS (version, cipher, expiry, issuer), redirects, format (codec, profile, rate, channels, declared vs measured bitrate), frames, ffprobe, and all response headers. Selectable text.
- **Events tab:** the timestamped event log with level icons, newest at the bottom, auto-scrolling while live.
- **After it stops:** the window keeps the final state; the verdict loses "so far". **Save Report…** (⌘S) saves the `final` line's text report (`.txt`) or JSON report (`.json`). **Check Again** reruns the same URL in the same window.
- **Several checks:** ⌘N opens another window; each window runs its own check. Closing a window stops its check (SIGINT, then SIGKILL after 3 s).
- **Errors:** ffmpeg missing → a clear message with `brew install ffmpeg`. Could not connect / no audio (exit code 3) → show streamprobe's findings and stderr tail rather than a blank window. The process dying without a `final` line → "Check ended unexpectedly", with the last report kept and stderr available.

**Plain text only:** every string from the stream or server is shown with verbatim `Text` (never a `LocalizedStringKey`/Markdown), with link detection off.

**Finding explanations:** generate a code → explanation table from `docs/FINDINGS.md` at build time and bundle it as a resource, so the app never drifts from the docs. If a code isn't in the table, show the message alone. *(Claude's call.)*

### 3. Build

- `make mac`: `cargo build --release --locked`, then `swift build -c release` in `macos/` with `DEVELOPER_DIR` set to Xcode, then assemble `dist/Streamprobe.app`: the Swift executable in `Contents/MacOS/`, the `streamprobe` binary as a helper in `Contents/Helpers/` (or `Resources/`), an `Info.plist`, an icon, and the findings table. Ad-hoc sign the helper first, then the app (`codesign --sign -`). `make mac-install` copies it to `~/Applications`.
- The app finds its bundled `streamprobe`; for development, an environment variable `STREAMPROBE_BIN` overrides the path.
- App icon: simple and generated (e.g. an SF Symbol waveform on a gradient rendered to an `.icns`). *(Claude's call.)*
- Not in CI for now (macOS CI runners and Xcode setup can come later); Rust changes are still covered by the existing CI.

### 4. Testing

- Rust: unit tests for the new line types; `make lint` and `make test` pass.
- Swift: unit tests that decode recorded `live`/`report`/`final` lines (fixtures captured from a real run), and that map finding events to timeline ranges.
- End to end: run the app against the repo's fault-injection server (`tests/fake_icecast.py`, as used by `make test-faults`) and confirm the sidebar, timeline highlights, events and saved reports match the CLI's report for the same run. Then a real public stream.

## Left to the builder

- Exact JSON field names for `live` lines (reuse `json_report` names).
- SwiftUI view structure, colours, spacing and typography within the layout above.
- How the timeline auto-zoom and hover work, as long as they match the CLI's columns.
- Whether `--live-json` is listed in `--help`.
