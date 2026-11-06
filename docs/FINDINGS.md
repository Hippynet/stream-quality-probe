# Finding codes

Each check streamprobe makes produces a **finding** with three parts: a level, a stable code and a human-readable message.
- **JSON:** findings are in `findings[]`, worst first. `problem_codes` lists the codes at `warn` or `bad`, and `result` is the overall verdict.
- **Dashboard:** the CHECKS section shows them live.
- **Event codes:** the event log (`events[]`) uses some of the same codes, plus a few event-only ones, listed at the end.

**The codes are a stable interface.** Scripts can match on them. Messages are for people and may change wording between versions.

| Level | Meaning | Affects `result` |
|---|---|---|
| `bad` | listeners are affected now, or the stream can't be played | `problems` (exit code 2) |
| `warn` | likely to cause trouble or a sign of a misconfiguration | `warnings` (exit code 1) |
| `info` | worth knowing, often deliberate | no |
| `ok` | a check that passed | no |

Exit code 3 means streamprobe could not connect or got no audio at all, so no audio checks ran.

A probe measures from **the machine it runs on**, and only for **the time it ran**. If a stream is healthy in a 60-second run but listeners report problems, run it for 10–30 minutes and look at `timeline` and `events`. Run it from a different network, or on the streaming server itself, to separate server problems from network problems.

## Connection

| Code | Level | Meaning | Usual causes / what to check |
|---|---|---|---|
| `bad_url` | bad | the URL can't be used (syntax, or a scheme other than http/https) | typo; a redirect to a `file:` or other non-HTTP URL |
| `dns_failed` | bad | the hostname doesn't resolve | typo, expired domain, DNS outage |
| `tcp_failed` | bad | can't connect to the host and port | server down, wrong port, firewall |
| `tls_failed` | bad | the TLS handshake failed (not a certificate problem) | server only speaks old TLS versions, or plain HTTP on an https port |
| `no_response` | bad | connected, but no valid HTTP response headers within 20 s | server overloaded, or deliberately blocking this client |
| `bad_response` | bad | the reply isn't HTTP or ICY | wrong port (e.g. the source port), not a streaming server |
| `blocked_internal` | bad | the URL, a redirect or a playlist entry resolved to a private, loopback or link-local address, so streamprobe refused to connect | misconfigured DNS, relay or playlist (e.g. a LAN address in a `.pls`), or an attempt to make the probe fetch internal pages. If you meant to probe an internal server, re-run with `--allow-internal`. Link-local / cloud-metadata addresses stay blocked unless you typed one yourself |
| `redirect_loop` | bad | more than 8 redirects, or more than 4 nested playlists | a redirect loop in the server or CDN configuration |
| `http_404` | bad | mount not found | the encoder isn't connected to that mount (Icecast removes a mount when its source drops), or the URL is wrong |
| `http_403`, `http_401` | bad | refused, or authentication required | listener limit reached, IP/geo block, a restricted mount |
| `http_503` and other `http_NNN` | bad | the server returned that HTTP status | server full, source down, server misconfiguration |
| `not_audio` | bad | the URL returned a web page, or something that's neither audio nor a playlist | the station's website or the server status page was given instead of the stream URL |
| `tls_cert` | bad | the certificate is invalid (expired, wrong name, untrusted issuer, missing intermediate) | renew or fix the certificate chain. Browsers and apps refuse the stream. `--insecure` lets the probe carry on, but the problem is still reported |
| `tls_expiring` | warn | the certificate expires in under 14 days | renew it before it lapses |
| `connect_slow` | warn | TCP connect over 500 ms, or response headers over 2 s | server load, distance, or a slow first hop. `connect` (ok) when fast |
| `many_redirects` | warn | more than 2 redirects before the stream | long CDN or load-balancer chains slow down every listener's start |
| `plain_http` | info | the stream is served over `http://` | web players on https sites block it (mixed content). Offer an https URL |

## Format and metadata

| Code | Level | Meaning | Usual causes / what to check |
|---|---|---|---|
| `content_type_mismatch` | warn | the `Content-Type` header doesn't match the codec actually in the stream | encoder or mount misconfigured. Smart speakers and browsers may refuse the stream |
| `bitrate_mismatch` | warn | measured bitrate differs from `icy-br` by more than 25% | the encoder's bitrate setting differs from what's advertised (not checked for VBR codecs: Vorbis, Opus, FLAC) |
| `low_sample_rate` | warn | sample rate below 32 kHz | a low-bitrate encoder preset. Fine for speech, dull for music |
| `mono_stream` | info | the stream is encoded with one channel | a deliberate encoder setting |
| `no_icy_metadata` | info | the server doesn't offer ICY metadata (`icy-metaint`) | metadata disabled on the mount. Players show no now-playing title |
| `no_title` | warn | ICY metadata is enabled, but no `StreamTitle` arrived within 45 s | the encoder isn't sending metadata, or the playout system isn't feeding it |
| `empty_title` | warn | the now-playing title is empty or a placeholder (`-`, `unknown`, `n/a`) | playout isn't passing track data to the encoder |
| `metadata_not_utf8` | warn | titles aren't UTF-8 (e.g. Latin-1) | accented characters show garbled in apps. Set the encoder/server charset to UTF-8 |
| `frame_sync` | info / warn / bad | the received bytes stopped being valid MP3/AAC frames (`format.frames.sync_losses`, `junk_bytes`). Info for a one-off; warn at 3+ losses or 4 KB+ of junk; bad at 20+ losses or 64 KB+ | corruption, a relay or proxy injecting data, or a wrong `icy-metaint` that leaves metadata inside the audio |
| `format_change` | warn | the stream re-synced with different frame parameters (e.g. a new sample rate) | the encoder restarted with other settings. Some players stop or play at the wrong speed |

## Delivery

| Code | Level | Meaning | Usual causes / what to check |
|---|---|---|---|
| `no_audio` | bad | connected, but nothing could be decoded | wrong format, garbage data, or the source sends nothing |
| `underrun` | bad | the simulated listener buffer (2 s prebuffer) ran dry | **listeners hear buffering**. `stall` and `slow_delivery` show whether the server or the network is the cause |
| `stall` | bad | the server stopped sending for 5 s or more | the station's upload connection or source encoder, or a server/relay problem. Some CDNs legitimately send a chunk every 2–4 s, which is why the threshold is 5 s |
| `slow_delivery` | warn | data arriving below realtime (under 0.97×) after 20 s | the station's upload is too slow for the bitrate, or the server/network path is congested |
| `disconnect` | bad | the connection dropped during the test (streamprobe reconnects automatically) | server restart, source dropped, network. A failed reconnect is logged as a `reconnect_failed` event |
| `decode_errors` | warn / bad | the decoder hit corrupt frames (bad when more than 20) | broken encoder, a bad relay, or a source reconnecting mid-stream. Errors in the first 3 s after audio starts or after a reconnect are normal resync and only counted in `delivery.startup_sync_errors_ignored` |
| `delivery` | ok | delivered in realtime with no underruns | |

## Audio

| Code | Level | Meaning | Usual causes / what to check |
|---|---|---|---|
| `dead_air` | bad | one or more silence runs of 3 s or more (below -60 dBFS on both channels) | playout stopped, wrong encoder input, muted fader, automation gap. `no_dead_air` (ok) otherwise |
| `mostly_silent` | bad | silent for more than half the test | the encoder is connected but nothing is playing |
| `dead_channel` | bad | only the left or right channel carries audio for more than 20% of the time | cabling, mixer pan, or a mono source on one input of a stereo sound card |
| `out_of_phase` | bad | left and right are polarity-inverted for more than 10% of the time | wiring: a balanced cable with pins swapped on one side. Mono listeners (smart speakers, phones, car radios) hear it thin or not at all |
| `mono_content` | warn | L = R for more than 80% of the time on a stereo stream | the encoder input is mono. Normal for talk radio, odd for music |
| `partly_mono` | info | L = R for 30–80% of the time | mono segments: speech, jingles, some mono tracks |
| `stereo` | ok | true stereo for most of the time | |
| `clipping` | warn / bad | samples over 0 dBFS or a true peak above +1 dBTP. Bad when more than 0.1% of samples are over | source or processing gain too high. Lower the encoder input or the limiter ceiling (about -1 dBTP) |
| `full_scale` | warn | more than 200 samples at exactly full scale | audio hitting 0 dBFS before encoding |
| `too_loud` | warn | integrated loudness above -7 LUFS | over-processing, likely clipping |
| `too_quiet` | warn | integrated loudness below -26 LUFS | encoder input too low. Stations typically sit around -16 to -9 LUFS. `loudness` (ok) otherwise |
| `low_bandwidth` | bad | audio bandwidth under 11 kHz | a very low bitrate, or (at 96 kb/s and up) a low-quality source re-encoded at a higher bitrate |
| `upscaled_source` | warn | bandwidth under 15.5 kHz at 160 kb/s or more | the station relays a lower-quality stream and re-encodes it at a higher bitrate |
| `reduced_bandwidth` | info | bandwidth under 15 kHz | typical of a low bitrate. HE-AAC's SBR makes bandwidth look full even at low bitrates. `bandwidth` (ok) otherwise |
| `dc_offset` | warn | a DC offset above 0.01 on either channel | faulty sound card or mixer output |

## Event-only codes

These appear in `events[]` but never as findings.

| Code | Meaning |
|---|---|
| `connected`, `reconnected` | a connection (re)opened |
| `reconnect_failed` | a reconnect attempt failed (the message includes the reason, e.g. a 404 when the mount vanished) |
| `playlist` | a `.pls` / `.m3u` / `.xspf` was followed to the stream URL |
| `title` | the now-playing title changed |
| `decode_error` | a decoder error (rate-limited to one event every 10 s) |
| `ffmpeg_exit` | the decoder process exited unexpectedly |
| `<code>_end` | a condition that started earlier ended (e.g. `stall_end`, `dead_air_end`); start events are backdated to when the condition began |

## Server-controlled text

Messages may quote text the stream's server sends: titles, header values, the HTTP status line, and "server said: …" error text. The JSON lists these fields under `untrusted_fields`. streamprobe removes control characters before storing them, but treat the text as data, never as instructions.
