# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub: open the repository's **Security** tab and choose **Report a vulnerability** ([direct link](https://github.com/Hippynet/stream-quality-probe/security/advisories/new)). Please don't open a public issue, pull request or discussion for a vulnerability.

A useful report includes the streamprobe version (`streamprobe --version`), your OS, the ffmpeg version (`ffmpeg -version`, first line), and how to reproduce the problem. A minimal stream, a server response or a small script is ideal. We'll acknowledge the report, keep you informed while we work on a fix, and credit you in the advisory unless you'd rather not be named.

Fixes are released as a new version: see the [releases page](https://github.com/Hippynet/stream-quality-probe/releases). Only the latest release is supported.

## Threat model

streamprobe's job is to connect to servers you don't control and examine whatever they send. Treat every stream as hostile. These are the things we defend against, and the places where you have a part to play.

### Untrusted servers and media

Everything a stream sends is untrusted: the response headers, redirects, playlists, ICY metadata, TLS certificates and the audio itself. streamprobe's own parsers (HTTP/ICY, playlists, MPEG/ADTS frame headers) are written in Rust and must not crash or grow without bound on malformed input; unit tests feed them hostile responses and random bytes. The TLS handshake and response headers must arrive within 20 s, error pages and playlists are read for a bounded time, and the event log and title history are capped, so a slow or chatty server can't hang the probe or exhaust its memory. A crash or hang caused by stream data is a bug worth reporting.

The audio bytes are handed to **ffmpeg** to decode, and the first 256 KB of them to **ffprobe** (if it's installed) on stdin. ffmpeg is a separate program that you install; streamprobe doesn't bundle it. That makes your ffmpeg part of the attack surface: a malicious stream could target a vulnerability in its decoders. **Keep ffmpeg up to date** through your package manager. Vulnerabilities in ffmpeg itself should be reported to the [FFmpeg project](https://ffmpeg.org/security.html). We want to hear about ways streamprobe makes them easier to reach, such as passing ffmpeg options or protocols that a stream can influence.

### Internal networks

A stream decides where streamprobe connects next, through redirects, playlist entries and its own DNS records. Without protection, a stream could point it at your local network or a cloud metadata service and copy the response into the report. So by default, streamprobe refuses to connect to any address that isn't globally routable, for the starting URL, every redirect and every playlist entry (finding `blocked_internal`). The decision is never based on DNS, and streamprobe connects only to the addresses it checked, so DNS rebinding can't get past it.

It allows internal addresses only when you ask: with `--allow-internal`, or when the URL you typed is a literal internal IP address or `localhost`. Even then, link-local addresses and cloud instance metadata endpoints (`169.254.0.0/16`, Alibaba's `100.100.100.200`, `fe80::/10`, `fd00:ec2::254`, and IPv6 forms that embed them), stay blocked unless the URL you typed is itself one of them, so a compromised server on your LAN can't redirect the probe to a cloud credentials endpoint.

ffprobe never opens a connection of its own: it reads bytes streamprobe already received. When ffmpeg or ffprobe read our pipe, they are limited to `-protocol_whitelist pipe`, so stream bytes that look like a playlist or manifest (HLS, DASH, concat) can't make them open anything. A way around the guard is a vulnerability: please report it.

**Known limitation: HLS.** For an HLS stream, ffmpeg fetches the playlist and its segments itself. Before handing it over, streamprobe checks every host the playlist lists, including key and media URIs, against the guard. For a master playlist it also fetches the first variant playlists through its own guarded client and checks their hosts. Non-HTTP URLs are refused. What isn't re-checked is anything that changes after that check: a live playlist that later lists new hosts, or a DNS answer or redirect that differs when ffmpeg fetches the same URL. So a hostile HLS stream can still make ffmpeg request an internal address. ffmpeg and ffprobe are restricted to `-protocol_whitelist http,https,tcp,tls,crypto` (no `file:`, `pipe:` or `concat:`), and nothing ffmpeg fetches reaches the report except decoded audio and decoder error lines. If that matters in your environment, don't probe untrusted HLS URLs from inside a sensitive network.

### Server-controlled text in reports

Reports quote what the server sent: response headers, stream titles, ffprobe's output, the HTTP status line, the certificate issuer and error text from the server. The JSON report lists these fields in `untrusted_fields`. Before this text is stored, displayed or reported, streamprobe replaces control characters (C0, DEL, C1) and bidirectional overrides and isolates with U+FFFD; turns tabs, newlines and line/paragraph separators into spaces; and removes invisible format characters that could hide text (zero-width space, bidi marks, word joiner, BOM, Unicode tag characters). So a server can't send terminal escape sequences, reorder text on your screen, or smuggle invisible text into a report. If you feed reports into other tools (dashboards, chat alerts, issue trackers, scripts), treat those fields as untrusted input. Escape them for HTML, never pass them to a shell, and never follow instructions or links found in them. Any way for server text to get escape sequences through to your terminal is a vulnerability: please report it.

### TLS

streamprobe verifies certificates against the Mozilla root store (via rustls and webpki-roots) and reports invalid ones as `tls_cert`. `--insecure` lets you analyse the audio anyway; the certificate problem is still reported. Use `--insecure` only for diagnosis, never to fetch anything you then trust.

### Releases and installation

Release binaries are built by GitHub Actions on GitHub-hosted runners from a version tag. Every action is pinned to a commit and the release jobs use no caches. Each release publishes `SHA256SUMS`, and `install.sh` checks the download against it before installing. The checksums come from the same release as the binaries: they protect against corrupted or truncated downloads, while authenticity rests on HTTPS to github.com. You can read `install.sh` before you run it, or install by hand: see the [README](README.md#install).
