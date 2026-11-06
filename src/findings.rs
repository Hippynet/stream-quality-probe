//! Turns measurements into findings: (level, stable code, message).
//! The codes are a stable interface (documented in docs/FINDINGS.md) that
//! scripts consuming `--json` rely on: don't rename them casually.

use crate::state::*;

pub struct Finding {
    pub level: Level,
    pub code: String,
    pub msg: String,
}

pub fn codec_name(st: &State) -> Option<String> {
    // our decoder sniffs the bytes (ignores Content-Type); ffprobe may trust the header
    if let Some(c) = &st.in_codec {
        return c.split_whitespace().next().map(String::from);
    }
    st.ffprobe_audio.as_ref()?.get("codec_name")?.as_str().map(String::from)
}

/// Codec profile (e.g. "LC", "HE-AAC"). ffprobe's value is only used when it
/// identified the same codec as our decoder (it may trust a wrong Content-Type).
pub fn codec_profile(st: &State) -> Option<String> {
    let codec = codec_name(st);
    let a = st.ffprobe_audio.as_ref();
    let same = a.and_then(|a| a.get("codec_name")).and_then(|v| v.as_str()) == codec.as_deref();
    let probe = a
        .and_then(|a| a.get("profile"))
        .and_then(|v| v.as_str())
        .filter(|p| same && *p != "unknown")
        .map(String::from);
    probe.or_else(|| {
        st.in_codec
            .as_ref()
            .and_then(|c| regex!(r"\(([^)]*AAC[^)]*|LC|Main|LTP)\)").captures(c))
            .map(|c| c[1].to_string())
    })
}

pub fn declared_kbps(st: &State) -> Option<u32> {
    if let Some(v) = st.header("icy-br") {
        if let Ok(n) = v.split(',').next().unwrap_or("").trim().parse() {
            return Some(n);
        }
    }
    if let Some(c) = st
        .header("ice-audio-info")
        .and_then(|i| regex!(r"bitrate=(\d+)").captures(i))
    {
        if let Ok(n) = c[1].parse() {
            return Some(n);
        }
    }
    let br = st
        .ffprobe_audio
        .as_ref()
        .and_then(|a| a.get("bit_rate"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());
    br.map(|b| (b / 1000) as u32).or(st.in_bitrate)
}

/// Percent of audible (non-silent) time in each channel state.
pub fn channel_pct(st: &State) -> ([f64; 7], f64) {
    let audible: f64 = CLASSES
        .iter()
        .filter(|c| **c != Class::Silence)
        .map(|c| st.class_sec[c.idx()])
        .sum();
    let mut p = [0.0; 7];
    if audible > 0.0 {
        for c in CLASSES.iter().filter(|c| **c != Class::Silence) {
            p[c.idx()] = 100.0 * st.class_sec[c.idx()] / audible;
        }
    }
    (p, audible)
}

const CTYPES: &[(&str, &[&str])] = &[
    ("mp3", &["audio/mpeg", "audio/mp3", "audio/mpeg3"]),
    (
        "aac",
        &["audio/aac", "audio/aacp", "audio/x-aac", "audio/mp4", "audio/x-m4a"],
    ),
    ("vorbis", &["application/ogg", "audio/ogg", "audio/vorbis"]),
    ("opus", &["application/ogg", "audio/ogg", "audio/opus", "audio/webm"]),
    ("flac", &["audio/flac", "audio/x-flac", "application/ogg", "audio/ogg"]),
];

/// All findings, worst first.
pub fn findings(st: &State, now: f64, final_: bool) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut add = |level: Level, code: &str, msg: String| {
        out.push(Finding {
            level,
            code: code.to_string(),
            msg,
        })
    };

    if let Some((code, msg)) = &st.fatal {
        add(Level::Bad, code, msg.clone());
        if let (Some(t), false) = (&st.tls_error, code == "tls_cert") {
            add(
                Level::Bad,
                "tls_cert",
                format!("TLS certificate problem: {t} - browsers and apps will refuse this stream"),
            );
        }
        return out;
    }
    let elapsed = now;
    let c = st.first_conn.as_ref().or(st.conn.as_ref());

    // connection ------------------------------------------------------------
    if let Some(c) = c {
        let slow = c.connect_ms > 500.0 || c.headers_ms > 2000.0;
        add(
            if slow { Level::Warn } else { Level::Ok },
            if slow { "connect_slow" } else { "connect" },
            format!(
                "Connected: tcp {:.0} ms, headers {:.0} ms{}",
                c.connect_ms,
                c.headers_ms,
                if slow { " (slow)" } else { "" }
            ),
        );
    }
    if let Some(t) = &st.tls_error {
        add(
            Level::Bad,
            "tls_cert",
            format!("TLS certificate problem: {t} - browsers and apps will refuse this stream"),
        );
    } else if let Some(d) = c.and_then(|c| c.cert_days_left).filter(|d| *d < 14.0) {
        add(
            Level::Warn,
            "tls_expiring",
            format!("TLS certificate expires in {d:.0} days"),
        );
    }
    if st.final_url.starts_with("http://") {
        add(
            Level::Info,
            "plain_http",
            "Plain HTTP - web players on HTTPS sites will block it (mixed content)".into(),
        );
    }
    if st.redirects.len() > 2 {
        add(
            Level::Warn,
            "many_redirects",
            format!("{} redirects before the stream", st.redirects.len()),
        );
    }

    // headers / format --------------------------------------------------------
    let ctype = st
        .header("content-type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase();
    let codec = codec_name(st);
    if let (Some(codec), false, false) = (&codec, ctype.is_empty(), st.mode_direct) {
        if let Some((_, exp)) = CTYPES.iter().find(|(c, _)| c == codec) {
            if !exp.contains(&ctype.as_str()) {
                add(
                    Level::Warn,
                    "content_type_mismatch",
                    format!(
                        "Content-Type '{ctype}' doesn't match codec {codec} (expected {}) - some players/smart speakers will refuse it",
                        exp[0]
                    ),
                );
            }
        }
    }
    if !st.mode_direct && !st.headers.is_empty() && st.metaint == 0 {
        add(
            Level::Info,
            "no_icy_metadata",
            "Server sent no ICY metadata (no now-playing titles)".into(),
        );
    } else if st.metaint > 0 && (final_ || elapsed > 45.0) && st.titles.is_empty() {
        add(
            Level::Warn,
            "no_title",
            "ICY metadata enabled but no StreamTitle received".into(),
        );
    }
    if let Some(t) = &st.title {
        if ["", "-", "unknown", "n/a"].contains(&t.trim().to_lowercase().as_str()) {
            add(
                Level::Warn,
                "empty_title",
                format!("Now-playing title is empty/placeholder: '{t}'"),
            );
        }
    }
    if st.meta_non_utf8 {
        add(
            Level::Warn,
            "metadata_not_utf8",
            "Metadata isn't UTF-8 - accented characters may show garbled in apps".into(),
        );
    }
    if st.in_channels == Some(1) {
        add(Level::Info, "mono_stream", "Stream is encoded MONO (1 channel)".into());
    }
    if let Some(r) = st.in_rate.filter(|r| *r < 32000) {
        add(Level::Warn, "low_sample_rate", format!("Low sample rate {r} Hz"));
    }
    let decl = declared_kbps(st);
    if let (Some(d), Some(m)) = (decl, st.measured_kbps()) {
        let vbr = matches!(codec.as_deref(), Some("vorbis" | "opus" | "flac"));
        if st.audio_sec > 20.0 && !vbr && d > 0 && (m - d as f64).abs() / d as f64 > 0.25 {
            add(
                Level::Warn,
                "bitrate_mismatch",
                format!("Measured bitrate {m:.0} kb/s vs declared {d} kb/s (icy-br / encoder mismatch)"),
            );
        }
    }

    // delivery -----------------------------------------------------------------
    // nothing decoded yet: a problem at the end of any run of 5 s+, or live after 10 s
    let nothing_yet = st.fatal.is_none() && st.first_audio.is_none() && (elapsed > 10.0 || (final_ && elapsed >= 5.0));
    if st.no_audio || nothing_yet {
        add(
            Level::Bad,
            "no_audio",
            "Connected but no audio could be decoded - wrong format, corrupt stream or source sending nothing".into(),
        );
    }
    if st.audio_sec > 0.0 {
        if st.underruns > 0 {
            add(
                Level::Bad,
                "underrun",
                format!(
                    "{} listener buffer underrun(s) - listeners hear drop-outs / buffering",
                    st.underruns
                ),
            );
        } else if elapsed > 15.0 {
            add(
                Level::Ok,
                "delivery",
                "Delivered in realtime, no listener buffer underruns".into(),
            );
        }
        if let Some(rt) = st.realtime_ratio().filter(|r| *r < 0.97 && elapsed > 20.0) {
            add(
                Level::Warn,
                "slow_delivery",
                format!("Data arriving slower than realtime ({rt:.2}x)"),
            );
        }
    }
    if st.stalls > 0 {
        add(
            Level::Bad,
            "stall",
            format!("{} stall(s) where the server stopped sending data", st.stalls),
        );
    }
    if st.disconnects > 0 {
        add(
            Level::Bad,
            "disconnect",
            format!("{} disconnect(s) during the test", st.disconnects),
        );
    }
    let fr = &st.frames.stats;
    if fr.sync_losses > 0 {
        let level = if fr.sync_losses >= 20 || fr.junk_bytes >= 65536 {
            Level::Bad
        } else if fr.sync_losses >= 3 || fr.junk_bytes >= 4096 {
            Level::Warn
        } else {
            Level::Info // a one-off is usually harmless
        };
        let cause = if st.metaint > 0 {
            "corruption, a relay or proxy injecting data, or a wrong icy-metaint leaving metadata in the audio"
        } else {
            "corruption, or a relay or proxy injecting data"
        };
        add(
            level,
            "frame_sync",
            format!(
                "Lost frame sync {} time(s): {} byte(s) of non-audio data inside the stream - {cause}",
                fr.sync_losses, fr.junk_bytes
            ),
        );
    }
    if fr.format_changes > 0 {
        add(
            Level::Warn,
            "format_change",
            format!(
                "Stream format changed {} time(s) mid-stream ({}) - the encoder restarted with different settings; some players stop or play at the wrong speed",
                fr.format_changes,
                fr.summary().unwrap_or_default()
            ),
        );
    }
    if st.decode_errors > 0 {
        add(
            if st.decode_errors > 20 { Level::Bad } else { Level::Warn },
            "decode_errors",
            format!(
                "{} decoder error(s) - corrupt frames from the encoder, a broken relay, or reconnect splices",
                st.decode_errors
            ),
        );
    }

    // audio -------------------------------------------------------------------------
    let total: f64 = st.class_sec.iter().sum();
    if total >= 5.0 {
        let (pct, audible) = channel_pct(st);
        let sil = 100.0 * st.class_sec[Class::Silence.idx()] / total;
        if st.silence_events > 0 {
            let longest = st.silence_longest.max(st.silence_run);
            add(
                Level::Bad,
                "dead_air",
                format!("Dead air: {} silence run(s), longest {longest:.0}s", st.silence_events),
            );
        } else if sil > 50.0 {
            add(
                Level::Bad,
                "mostly_silent",
                format!("Mostly silent ({sil:.0}% of the time)"),
            );
        } else if total > 10.0 {
            add(Level::Ok, "no_dead_air", "No dead air".into());
        }
        if audible >= 5.0 && st.in_channels != Some(1) {
            let p = |c: Class| pct[c.idx()];
            let one = p(Class::LeftOnly) + p(Class::RightOnly);
            if one > 20.0 {
                let side = if p(Class::LeftOnly) > p(Class::RightOnly) {
                    "left"
                } else {
                    "right"
                };
                add(
                    Level::Bad,
                    "dead_channel",
                    format!("Only the {side} channel carries audio ({one:.0}% of the time) - check encoder input cabling / channel mapping"),
                );
            }
            if p(Class::OutOfPhase) > 10.0 {
                add(
                    Level::Bad,
                    "out_of_phase",
                    format!(
                        "Out of phase {:.0}% of the time - one channel is inverted; mono listeners (phones, smart speakers) hear it thin or vanish",
                        p(Class::OutOfPhase)
                    ),
                );
            }
            if p(Class::MonoContent) > 80.0 {
                add(
                    Level::Warn,
                    "mono_content",
                    format!(
                        "Mono content in a stereo stream ({:.0}% of the time) - encoder input is mono, or the programme is speech from a mono mic",
                        p(Class::MonoContent)
                    ),
                );
            } else if p(Class::MonoContent) > 30.0 {
                add(
                    Level::Info,
                    "partly_mono",
                    format!(
                        "Mono content {:.0}% of the time (normal for speech, odd for music)",
                        p(Class::MonoContent)
                    ),
                );
            } else if p(Class::Stereo) > 50.0 {
                add(
                    Level::Ok,
                    "stereo",
                    format!("True stereo ({:.0}% of the time)", p(Class::Stereo)),
                );
            }
        }
        if st.dc_n > 0 {
            let (l, r) = (st.dc[0] / st.dc_n as f64, st.dc[1] / st.dc_n as f64);
            if l.abs().max(r.abs()) > 0.01 {
                add(
                    Level::Warn,
                    "dc_offset",
                    format!("DC offset (L {l:+.3}, R {r:+.3}) - faulty sound card / mixer output"),
                );
            }
        }
    }

    if let (Some(i), true) = (st.lufs_i, st.audio_sec > 15.0) {
        if i < -26.0 {
            add(
                Level::Warn,
                "too_quiet",
                format!("Very quiet: {i:.1} LUFS integrated (stations are typically -16 to -9)"),
            );
        } else if i > -7.0 {
            add(
                Level::Warn,
                "too_loud",
                format!("Extremely loud: {i:.1} LUFS - likely clipping / over-processing"),
            );
        } else {
            add(Level::Ok, "loudness", format!("Loudness {i:.1} LUFS integrated"));
        }
    }
    let samples = (st.audio_sec * SR as f64 * 2.0).max(1.0);
    let over_pct = 100.0 * st.overs_total as f64 / samples;
    if over_pct > 0.1 {
        add(
            Level::Bad,
            "clipping",
            format!(
                "Heavy clipping: {} samples over 0 dBFS ({over_pct:.2}% of samples), true peak {:+.1} dBTP - turn the source/processing down",
                st.overs_total, st.tpk_max
            ),
        );
    } else if st.overs_total > 50 || st.tpk_max > 1.0 {
        add(
            Level::Warn,
            "clipping",
            format!(
                "Peaks exceed 0 dBFS after decoding (true peak {:+.1} dBTP, {} samples over) - encoder input too hot; set the limiter ceiling to about -1 dBTP",
                st.tpk_max, st.overs_total
            ),
        );
    } else if st.clip_total > 200 {
        add(
            Level::Warn,
            "full_scale",
            format!("{} samples at full scale - audio is hitting 0 dBFS", st.clip_total),
        );
    }

    if let (Some(bw), true) = (st.bandwidth, st.spec_n >= 10) {
        let bw = bw / 1000.0;
        if bw < 11.0 {
            let why = match decl {
                Some(d) if d >= 96 => {
                    format!("; at {d} kb/s this means the source was a very low-quality stream re-encoded up")
                }
                _ => " (very low bitrate or poor source)".into(),
            };
            add(
                Level::Bad,
                "low_bandwidth",
                format!("Audio bandwidth only ~{bw:.1} kHz - dull / phone-like{why}"),
            );
        } else if let (Some(d), true) = (decl, bw < 15.5) {
            if d >= 160 {
                add(
                    Level::Warn,
                    "upscaled_source",
                    format!("Bandwidth ~{bw:.1} kHz is low for {d} kb/s - source was probably a lower-quality stream re-encoded up"),
                );
            } else if bw < 15.0 {
                add(
                    Level::Info,
                    "reduced_bandwidth",
                    format!("Audio bandwidth ~{bw:.1} kHz (typical of low bitrate)"),
                );
            } else {
                add(Level::Ok, "bandwidth", format!("Audio bandwidth ~{bw:.1} kHz"));
            }
        } else if bw < 15.0 {
            add(
                Level::Info,
                "reduced_bandwidth",
                format!("Audio bandwidth ~{bw:.1} kHz (typical of low bitrate)"),
            );
        } else {
            add(Level::Ok, "bandwidth", format!("Audio bandwidth ~{bw:.1} kHz"));
        }
    }

    out.sort_by_key(|f| std::cmp::Reverse(f.level)); // stable: keeps order within a level
    out
}

pub fn overall(fs: &[Finding]) -> &'static str {
    match fs.iter().map(|f| f.level).max() {
        Some(Level::Bad) => "problems",
        Some(Level::Warn) => "warnings",
        _ => "healthy",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> State {
        State {
            tpk_max: -200.0,
            peak_max: -200.0,
            final_url: "https://example.com/live".into(),
            ..Default::default()
        }
    }

    fn codes(fs: &[Finding]) -> Vec<&str> {
        fs.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn fatal_error_still_reports_a_bad_certificate() {
        let mut st = base();
        st.fatal = Some(("not_audio".into(), "web page".into()));
        st.tls_error = Some("certificate has expired".into());
        assert_eq!(codes(&findings(&st, 1.0, true)), ["not_audio", "tls_cert"]);
    }

    #[test]
    fn content_type_is_checked_against_the_decoders_codec() {
        let mut st = base();
        st.in_codec = Some("mp3 (mp3float)".into());
        st.headers = vec![("content-type".into(), "audio/aac".into())];
        st.ffprobe_audio = Some(json!({"codec_name": "aac", "profile": "SSR"})); // fooled by the header
        assert!(codes(&findings(&st, 10.0, false)).contains(&"content_type_mismatch"));
        assert_eq!(codec_profile(&st), None);
    }

    #[test]
    fn mono_content_and_dead_air_are_problems_worst_first() {
        let mut st = base();
        st.class_sec[Class::MonoContent.idx()] = 90.0;
        st.class_sec[Class::Stereo.idx()] = 5.0;
        st.class_sec[Class::Silence.idx()] = 5.0;
        st.silence_events = 1;
        st.silence_longest = 5.0;
        let fs = findings(&st, 100.0, true);
        let c = codes(&fs);
        assert!(c.contains(&"mono_content") && c.contains(&"dead_air"), "{c:?}");
        assert_eq!(fs[0].level, Level::Bad);
        assert_eq!(overall(&fs), "problems");
    }

    #[test]
    fn connected_but_nothing_decoded_is_a_problem() {
        let mut st = base();
        st.headers = vec![("content-type".into(), "audio/mpeg".into())];
        assert!(!codes(&findings(&st, 3.0, true)).contains(&"no_audio")); // too short to judge
        let fs = findings(&st, 8.0, true);
        assert!(codes(&fs).contains(&"no_audio"));
        assert_eq!(overall(&fs), "problems");
    }

    #[test]
    fn clean_stereo_stream_is_healthy() {
        let mut st = base();
        st.audio_sec = 60.0;
        st.first_audio = Some(0.5);
        st.class_sec[Class::Stereo.idx()] = 60.0;
        st.lufs_i = Some(-14.0);
        let fs = findings(&st, 60.0, true);
        assert_eq!(overall(&fs), "healthy", "{:?}", codes(&fs));
    }
}
