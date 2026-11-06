# Contributing to streamprobe

Thanks for helping. Bug reports, stream URLs that confuse streamprobe, and pull requests are all welcome. For anything bigger than a small fix, please open an issue first so we can agree on the approach.

Security problems go through private reporting instead: see [SECURITY.md](SECURITY.md).

## Setting up

You need:

- **Rust**, stable, via [rustup](https://rustup.rs/). The minimum supported version is 1.88 (`rust-version` in `Cargo.toml`), and CI checks it.
- **ffmpeg** (with ffprobe) on your `PATH`, to run streamprobe and the end-to-end suite.
- **python3**, for the end-to-end suite's fake server and for `scripts/third_party_licenses.py`. The standard library is enough.
- Optional: [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) for `make deny`, and [ShellCheck](https://www.shellcheck.net/) for `make lint`.

```bash
git clone https://github.com/Hippynet/stream-quality-probe.git
cd stream-quality-probe
make build
make run URL=https://ice2.somafm.com/groovesalad-128-mp3
```

## Make targets

| Target | What it does |
| --- | --- |
| `make build` | Release build: `target/release/streamprobe` |
| `make test` | Unit tests. No network, no ffmpeg |
| `make lint` | `cargo fmt --check`, `clippy -D warnings`, and `sh -n` / ShellCheck on the shell scripts. Writes nothing |
| `make fmt` | Format the code |
| `make run URL=…` | Probe a stream with a release build |
| `make test-faults` | The end-to-end suite (see below). Needs ffmpeg and python3; takes about 2 minutes |
| `make deny` | cargo-deny: security advisories, licences, duplicate crates, sources |
| `make notices` | Write `THIRD_PARTY_LICENSES.txt` for this machine, or for `TARGETS="…"` |
| `make install` | Install into `$(PREFIX)/bin` (default `/usr/local`) |

CI runs formatting, clippy and the unit tests on Linux and macOS, `cargo check` with the minimum Rust version, cargo-deny, and the licence-notice script. All CI jobs run on GitHub-hosted runners.

## The end-to-end suite

`make test-faults` is the real regression test. `tests/make_fixtures.sh` uses ffmpeg to generate synthetic MP3s with known faults, plus HLS segments in `tests/fixtures/hls/`. `tests/fake_icecast.py` serves them as live Icecast/Shoutcast and HLS streams, and `tests/run_faults.sh` runs streamprobe against each one. It asserts that every expected finding code appears, and that clean streams don't raise false alarms such as `frame_sync` or `decode_errors`.

The main `/faults` stream plays stereo, then dual mono, out of phase, silence, left channel only and clipping. It stalls at 78 s and drops the connection at 100 s. Other mounts cover a low-quality source re-encoded at a higher bitrate, a mono-encoded stream, a wrong `Content-Type`, a wrong `icy-metaint` (metadata left inside the audio), a live sliding-window HLS stream, a `.pls` playlist, a 404 and an HTML page.

It isn't part of CI because it needs ffmpeg and streams in real time. Run it locally before every release, and whenever you change analysis, networking, decoding or findings.

## Pull requests

- Work on a short-lived branch and open a pull request against `main`. PRs are squash-merged.
- Write commit messages and PR titles as [Conventional Commits](https://www.conventionalcommits.org/): `feat: …`, `fix(net): …`, `docs: …`, `ci: …`, `chore(deps): …`. The squash-merge commit takes the PR title.
- Before you push: `make lint test`. If you changed dependencies, also run `make deny` and `make notices`. If you changed behaviour, run `make test-faults`.
- Add a line to the `Unreleased` section of [CHANGELOG.md](CHANGELOG.md) for anything a user would notice.
- **Finding codes are a public interface.** Scripts match on them. Don't rename a code or change its meaning; add a new one instead. Every code is documented in [docs/FINDINGS.md](docs/FINDINGS.md), and new codes go there too.

### Dependencies

- Crates come from crates.io only (cargo-deny rejects git and other registries).
- A new dependency's licence must be on the allow list in `deny.toml`. Adding a licence to that list is a decision: give it its own PR.
- Every crate needs a licence text for `THIRD_PARTY_LICENSES.txt`. If a crate ships no licence file, `scripts/third_party_licenses.py` fails. Vendor the upstream texts in `scripts/license-fallbacks/<crate>/` and add an entry to `FALLBACKS` in the script.
- Take new versions (of crates, actions and build tools) only once they're at least two weeks old. Dependabot's cooldown does this for its update PRs.

## How the code fits together

streamprobe is a single binary crate.

| File | What it does |
| --- | --- |
| `src/main.rs` | Command-line options, wiring the threads together, the main loop (dashboard or non-interactive), exit codes, saving reports |
| `src/net.rs` | A raw-socket HTTP/ICY client. It isn't an HTTP library because Shoutcast answers `ICY 200 OK`. Handles redirects, TLS with timings, certificate errors and expiry, `.pls` / `.m3u` / `.xspf` / HLS detection, the internal-address guard, ICY metadata demuxing, and the reader thread that feeds ffmpeg and reconnects |
| `src/frames.rs` | Frame-header analysis of the bytes as received, before decoding: MPEG-1/2/2.5 Layer I/II/III and AAC ADTS (version, layer, CBR/VBR, channel mode, CRC, emphasis), sync losses, ID3 tags, format changes; recognises Ogg and FLAC containers |
| `src/decoder.rs` | Spawns ffmpeg (f32le stereo 44.1 kHz on stdout) and parses its stderr for the input description and decoder errors; runs ffprobe on the first 256 KB received, via stdin |
| `src/analysis.rs` | 0.25 s blocks: per-channel RMS/peak, L/R correlation → stereo state, overs, DC, EBU R128, the FFT bandwidth estimate, dead-air and condition tracking; `tick()` runs the simulated listener buffer |
| `src/state.rs` | `State` (all measurements) behind `Shared` (a mutex plus a stop flag); per-second buckets for the timeline; **all the thresholds** |
| `src/findings.rs` | Turns `State` into findings: level, stable code, message |
| `src/timeline.rs` | Aggregates the buckets into auto-zoomed columns for the dashboard, the text report and the JSON |
| `src/ui.rs` | Dashboard, status line, text report |
| `src/report.rs` | The JSON report |
| `tests/` | The end-to-end suite: `fake_icecast.py`, `make_fixtures.sh`, `run_faults.sh` |
| `scripts/` | `third_party_licenses.py` and its vendored fallback texts |

The Rust dependencies are deliberately few: `rustls` (with `ring`) and `webpki-roots` for TLS without OpenSSL, so the Linux build can be fully static; `x509-parser` for certificate expiry; `ebur128` for loudness and true peak; `rustfft` for bandwidth; `crossterm` for the dashboard; plus `clap`, `serde_json`, `chrono`, `regex`, `url` and `ctrlc`. ffmpeg is a runtime dependency that we run as a separate process, never link.

### Things to know

- **Thresholds live in `src/state.rs`.** `STALL_SEC` is 5 s on purpose. Some CDNs and proxies (openresty-based ones, for example) legitimately send a 16 KB chunk only every 2 s or so. Gaps that listeners actually hear are caught by the buffer model (`underrun`), not by stalls.
- **Decoder errors right after a start are noise.** Errors within `STARTUP_GRACE_SEC` of audio starting, or of a reconnect, come from joining the stream mid-frame. They're counted separately as `startup_sync_errors_ignored`. Errors logged after a stop was requested are ignored too, because we cut the stream off ourselves.
- **The codec comes from the bytes**, as decoded by ffmpeg and parsed by `frames.rs`, never from the `Content-Type` header. ffprobe also reads bytes (on stdin, never its own connection), and its profile is used only when its codec agrees with ours. That's what makes `content_type_mismatch` possible.
- **HLS is different.** In HLS mode (`mode: direct` in the report), ffmpeg reads the URL itself, restricted by `PROTOCOL_WHITELIST`: there are no ICY headers, no metadata and no byte counts, and the realtime ratio is `None` because segments arrive in bursts. The internal-address guard doesn't cover the segment URLs ffmpeg fetches (see SECURITY.md).
- **Everything from the network is hostile.** Code that parses server data (`net.rs`, `frames.rs`) must never panic, must keep its buffers bounded and must give up after a deadline. Server text goes through `state::cap()` (which also replaces control and bidi-override characters via `clean()`) before it's stored, and any JSON path it reaches must be listed in `untrusted_fields`. Decisions in the internal-address guard must never depend on DNS.
- **Dashboard output:** with the terminal in raw mode, lines need `\r\n`. `term_size()` falls back to `COLUMNS`/`LINES` when a pseudo-terminal reports 0×0.
- **Audio caveats:** HE-AAC's SBR makes the bandwidth look full even at low bitrates. Mono content is normal for speech.

## Releases

Releases are tag-driven and immutable. Maintainers cut them; contributors don't need to do anything.

The tag is `MAJOR.MINOR.PATCH.BUILD`, with no leading `v` (the first release is `0.1.0.0`). Its first three parts must equal `version` in `Cargo.toml`. The `BUILD` part allows another release of the same code version, for example after a packaging fix.

1. On `main`: set `version` in `Cargo.toml` if it changes, and move the `Unreleased` changelog entries under the new version and date. Merge that through a PR.
2. Check out the merged `main` and make sure everything is green locally: `make lint test test-faults deny`.
3. Tag and push:

   ```bash
   git tag 0.1.0.0
   git push origin 0.1.0.0
   ```

4. The **Release** workflow (`.github/workflows/release.yml`) checks the tag against `Cargo.toml`. It builds static Linux x86_64 and arm64 binaries (musl, through cargo-zigbuild) and a universal macOS binary, smoke-tests each one on its own architecture, and packages each one with the README, both licence files and `THIRD_PARTY_LICENSES.txt`. It then creates the GitHub release with `SHA256SUMS`. The install script picks up the new release from `releases/latest`.

The workflow never overwrites an existing release: to change anything, cut a new tag. To rehearse, run the Release workflow by hand (**Actions → Release → Run workflow**) with an existing tag. It builds everything and keeps the archives as workflow artifacts for 30 days, but publishes nothing.

Release jobs use no caches, so nothing written by other workflows can reach a release build. Every action is pinned to a full commit SHA, and the zig toolchain is pinned by hash in `.github/zigbuild-requirements.txt`.
