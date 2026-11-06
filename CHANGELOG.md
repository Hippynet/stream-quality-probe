# Changelog

All notable changes to streamprobe are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions are release tags of the form `MAJOR.MINOR.PATCH.BUILD` (see [CONTRIBUTING.md](CONTRIBUTING.md#releases)).

## [Unreleased]

## [0.1.0.1] - 2026-11-06

The first public release. (`0.1.0.0` was tagged but never released: the release workflow's Linux smoke test misread a static binary's `ldd` exit status. Same code, fixed release workflow.)

### Added

- Live terminal dashboard with an always-visible metadata panel (station, server, TLS, format, frames, ffprobe, now playing), levels, loudness, stereo state, quality, delivery, an auto-zoomed session timeline and an event log. It adapts to the terminal size. `--no-ui` prints events and periodic status lines instead.
- Text and JSON reports (`--json`, `-o FILE`; the file is rewritten every 30 s during a run) with stable finding codes, documented in [docs/FINDINGS.md](docs/FINDINGS.md). The text report includes every response header. The JSON report lists its server-controlled fields in `untrusted_fields`.
- Exit codes for scripts: `0` healthy, `1` warnings, `2` problems, `3` could not connect or no audio.
- Connection checks: DNS, TCP, TLS and response timings; TLS version and cipher; redirects; `.pls`, `.m3u` and `.xspf` playlists; HLS; certificate validity, expiry date and issuer; HTTP errors; URLs that return a web page instead of audio. `--insecure` continues past an invalid certificate and still reports it.
- Format checks: the codec and profile identified from the audio bytes, `Content-Type` mismatches, declared vs measured bitrate and low sample rates.
- Frame-header analysis of the raw bytes before decoding, for MPEG-1/2/2.5 Layer I/II/III and AAC ADTS: version and layer, CBR or VBR with min/max/average bitrate, sample rate, channel mode, CRC and emphasis, and ID3 tags from autoDJs (`format.frames` in the JSON). It reports lost frame sync (`frame_sync`) and format changes mid-stream (`format_change`).
- ffprobe details (format, probe score, codec, profile, sample format, layout, bitrate), taken from the first 256 KB of audio streamprobe already received, so ffprobe never opens a second connection.
- ICY metadata: station fields, now-playing titles and their history; missing, empty and non-UTF-8 titles.
- Audio analysis: stereo, mono content, mono stream, out of phase, left or right channel only; EBU R128 loudness and true peak; samples over 0 dBFS; DC offset; an audio-bandwidth estimate that catches low-quality sources re-encoded at a higher bitrate; dead air.
- Delivery analysis: a simulated listener buffer and its underruns, stalls, slow delivery, disconnects with automatic reconnection, and decoder errors.
- An internal-address guard: connections to private, loopback, link-local and other non-global addresses are refused (`blocked_internal`) unless you pass `--allow-internal` or type a literal internal IP or `localhost`.
- Hardening against hostile servers: control and bidirectional-override characters in server text are replaced before display or reporting; the TLS handshake and response headers must arrive within 20 s; error pages and playlists are read for a bounded time; the event log (5000) and title history (2000) are capped; ffmpeg and ffprobe are limited to HTTP(S) protocols.
- Release binaries for macOS (universal) and static Linux (x86_64 and arm64), each archive with `THIRD_PARTY_LICENSES.txt`, and an install script that verifies downloads against `SHA256SUMS`.
- Licensed under MIT OR Apache-2.0.

[Unreleased]: https://github.com/Hippynet/stream-quality-probe/compare/0.1.0.1...HEAD
[0.1.0.1]: https://github.com/Hippynet/stream-quality-probe/releases/tag/0.1.0.1
