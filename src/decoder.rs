//! ffmpeg (decoding) and ffprobe (structured metadata) child processes.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::net::USER_AGENT;
use crate::state::*;

/// Look for a tool on PATH, then in the usual Homebrew / system locations
/// (GUI apps and launchd jobs often have a minimal PATH).
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/snap/bin"] {
        dirs.push(PathBuf::from(d));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Network protocols ffmpeg/ffprobe may use when they read an HLS URL
/// themselves: nothing local (`file:`, `pipe:`, `concat:`, …) that a
/// hostile playlist could point them at.
pub const PROTOCOL_WHITELIST: &str = "http,https,tcp,tls,crypto";

/// ffmpeg decoding to f32le stereo 44.1 kHz on stdout. Reads stdin (our ICY
/// client feeds it) or, for HLS, the URL directly.
pub fn spawn_ffmpeg(ffmpeg: &Path, direct_url: Option<&str>) -> io::Result<Child> {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"]);
    match direct_url {
        Some(u) => {
            c.args(["-protocol_whitelist", PROTOCOL_WHITELIST]);
            c.args(["-user_agent", USER_AGENT, "-rw_timeout", "15000000", "-i", u]);
            c.stdin(Stdio::null());
        }
        None => {
            // stdin carries untrusted bytes: if they sniff as a playlist/manifest
            // (HLS, DASH, concat), ffmpeg must not be able to open what it lists
            c.args(["-protocol_whitelist", "pipe"]);
            c.args(["-probesize", "65536", "-analyzeduration", "1500000", "-i", "pipe:0"]);
            c.stdin(Stdio::piped());
        }
    }
    c.args([
        "-vn",
        "-map",
        "0:a:0",
        "-ac",
        "2",
        "-ar",
        &SR.to_string(),
        "-f",
        "f32le",
        "pipe:1",
    ]);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
    c.spawn()
}

/// Parses ffmpeg's stderr: the input stream description and decoder errors.
pub fn stderr_reader(sh: Arc<Shared>, stderr: ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut r = BufReader::new(stderr);
    let mut raw = Vec::new();
    let mut in_input = false;
    let mut last_err_event = -100.0;
    let mut last_was_err = false;
    loop {
        raw.clear();
        match r.read_until(b'\n', &mut raw) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&raw).to_string();
        for line in text.split('\r').map(str::trim_end).filter(|l| !l.is_empty()) {
            {
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.push_back(line.to_string());
                if t.len() > 15 {
                    t.pop_front();
                }
            }
            if line.starts_with("Input #0") {
                in_input = true;
                continue;
            }
            if line.starts_with("Output #0") || line.starts_with("Stream mapping") {
                in_input = false;
            }
            if in_input {
                // anchored: ffmpeg prints tags in this block as "  title : <value>", and a
                // station-controlled tag must not be able to pose as the stream line
                if let Some(c) =
                    regex!(r"^\s*Stream #0:\d+(?:\[0x[0-9a-fA-F]+\])?(?:\([^)]*\))?: Audio: (.+)").captures(line)
                {
                    let mut st = sh.lock();
                    if st.in_line.is_none() {
                        parse_stream(&mut st, &c[1]);
                    }
                }
                continue;
            }
            if sh.stopped() {
                continue; // shutting down: truncated packets are our doing
            }
            let now = sh.now();
            if let Some(c) = regex!(r"Last message repeated (\d+) times").captures(line) {
                if last_was_err {
                    sh.lock().decode_errors += c[1].parse::<u64>().unwrap_or(0);
                }
                continue;
            }
            last_was_err = false;
            let is_err = line.starts_with('[')
                && regex!(r"(?i)(error|invalid|corrupt|missing|overread|skipping|incomplete|damaged|concealing|mismatch|failed)").is_match(line)
                && !regex!(r"(?i)(deprecated|Estimating duration|non-monotonic|Opening '|Skip \('|Error closing|invalid concatenated file|using bitrate for duration)").is_match(line);
            if !is_err {
                continue;
            }
            let mut st = sh.lock();
            let starting = st.first_audio.is_none_or(|fa| now - fa < STARTUP_GRACE_SEC);
            if starting || now - st.last_connect < STARTUP_GRACE_SEC {
                st.sync_errors += 1; // resync at stream start / after reconnect: harmless
                continue;
            }
            st.decode_errors += 1;
            st.bucket(now).decode_err += 1;
            last_was_err = true;
            if now - last_err_event > 10.0 {
                last_err_event = now;
                let msg = regex!(r"^\[[^\]]*@ 0x[0-9a-f]+\]\s*").replace(line, "");
                let msg: String = msg.chars().take(110).collect();
                st.event(now, Level::Warn, "decode_error", format!("Decoder: {msg}"));
            }
        }
    }
}

/// e.g. "mp3 (mp3float), 44100 Hz, stereo, fltp, 128 kb/s"
fn parse_stream(st: &mut State, desc: &str) {
    st.in_line = Some(cap(desc, 200));
    let parts: Vec<&str> = desc.split(',').map(str::trim).collect();
    st.in_codec = parts.first().map(|s| cap(s, 60));
    for p in parts.iter().skip(1) {
        if let Some(hz) = p.strip_suffix("Hz") {
            st.in_rate = hz.trim().parse().ok();
        } else if *p == "mono" || *p == "stereo" || p.contains("channels") || regex!(r"^\d\.\d").is_match(p) {
            st.in_layout = Some(cap(p, 40));
            st.in_channels = Some(match *p {
                "mono" => 1,
                "stereo" => 2,
                _ => regex!(r"(\d+) channels")
                    .captures(p)
                    .and_then(|c| c[1].parse().ok())
                    .unwrap_or(6),
            });
        } else if let Some(k) = p.strip_suffix("kb/s") {
            st.in_bitrate = k.trim().parse().ok();
        }
    }
}

/// ffprobe in the background. For ICY/HTTP streams it reads the first
/// PROBE_SAMPLE_BYTES we received on stdin, so it never opens a connection of
/// its own: no second listener on the server, no way around the
/// internal-address guard, and it identifies the codec from the bytes rather
/// than the Content-Type. HLS (`direct_url`) has no byte stream of ours, so
/// ffprobe reads the URL, restricted to network protocols.
pub fn run_ffprobe(sh: Arc<Shared>, ffprobe: PathBuf, direct_url: Option<String>) {
    let mut cmd = Command::new(&ffprobe);
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ]);
    let sample = match &direct_url {
        Some(u) => {
            cmd.args(["-protocol_whitelist", PROTOCOL_WHITELIST]);
            cmd.args(["-rw_timeout", "15000000", "-user_agent", USER_AGENT, "-i", u]);
            cmd.stdin(Stdio::null());
            None
        }
        None => match wait_for_sample(&sh) {
            Some(s) => {
                cmd.args(["-protocol_whitelist", "pipe", "-i", "pipe:0"])
                    .stdin(Stdio::piped());
                Some(s)
            }
            None => {
                if !sh.stopped() {
                    sh.lock().ffprobe_err = Some("no audio received to probe".into());
                }
                return;
            }
        },
    };
    let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            sh.lock().ffprobe_err = Some(e.to_string());
            return;
        }
    };
    if let (Some(sample), Some(mut stdin)) = (sample, child.stdin.take()) {
        // ffprobe may stop reading early: a broken pipe here is fine
        std::thread::spawn(move || {
            let _ = stdin.write_all(&sample);
        });
    }
    let (Some(mut out), Some(mut errp)) = (child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        return;
    };
    // owned by Shared from here on, so the main thread can kill it on exit
    *sh.ffprobe_child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
    let o = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let e = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = errp.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(25);
    let status = loop {
        let polled = match sh.ffprobe_child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            Some(c) => c.try_wait(),
            None => return, // killed by the main thread on exit
        };
        match polled {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline && !sh.stopped() => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                sh.kill_ffprobe();
                break None;
            }
        }
    };
    sh.ffprobe_child.lock().unwrap_or_else(|e| e.into_inner()).take();
    let (stdout, stderr) = (o.join().unwrap_or_default(), e.join().unwrap_or_default());
    let mut st = sh.lock();
    match status {
        Some(s) if s.success() => match serde_json::from_str::<serde_json::Value>(&stdout) {
            Ok(v) => {
                st.ffprobe_audio = v
                    .get("streams")
                    .and_then(|s| s.as_array())
                    .and_then(|a| {
                        a.iter()
                            .find(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"))
                    })
                    .map(|a| capped_object(a, 40));
                let format = v.get("format");
                st.ffprobe_tags = format.and_then(|f| f.get("tags")).map(|t| capped_object(t, 20));
                st.ffprobe_format = format.map(|f| {
                    let keep = [
                        "format_name",
                        "format_long_name",
                        "probe_score",
                        "bit_rate",
                        "nb_streams",
                    ];
                    let m: serde_json::Map<String, serde_json::Value> = keep
                        .iter()
                        .filter_map(|k| f.get(*k).map(|v| (k.to_string(), v.clone())))
                        .collect();
                    capped_object(&serde_json::Value::Object(m), 10)
                });
            }
            Err(e) => st.ffprobe_err = Some(format!("bad JSON: {e}")),
        },
        Some(_) => {
            let msg = stderr.trim();
            let tail: String = msg
                .chars()
                .rev()
                .take(200)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            st.ffprobe_err = Some(if tail.is_empty() { "failed".into() } else { clean(&tail) });
        }
        None => st.ffprobe_err = Some("timed out".into()),
    }
}

/// Wait until we've received PROBE_SAMPLE_BYTES of audio, or audio has been
/// arriving for 8 s (low-bitrate streams), then hand over a copy.
fn wait_for_sample(sh: &Shared) -> Option<Vec<u8>> {
    let start = Instant::now();
    loop {
        {
            let st = sh.lock();
            let n = st.probe_sample.len();
            let enough_time = st.first_audio.is_some_and(|fa| sh.now() - fa > 8.0) || start.elapsed().as_secs() > 20;
            if n >= PROBE_SAMPLE_BYTES || (n > 0 && enough_time) {
                return Some(st.probe_sample.clone());
            }
            if sh.stopped() || start.elapsed().as_secs() > 45 {
                return (n > 0).then(|| st.probe_sample.clone());
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A copy of a JSON object with at most `max` entries, every string capped
/// (ffprobe output includes station-controlled tag text).
fn capped_object(v: &serde_json::Value, max: usize) -> serde_json::Value {
    let Some(o) = v.as_object() else {
        return serde_json::Value::Null;
    };
    o.iter()
        .take(max)
        .map(|(k, v)| {
            let v = match v {
                serde_json::Value::String(s) => serde_json::Value::String(cap(s, 200)),
                serde_json::Value::Object(_) | serde_json::Value::Array(_) => serde_json::Value::Null,
                other => other.clone(),
            };
            (cap(k, 60), v)
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_cannot_pose_as_the_stream_line() {
        let re = regex!(r"^\s*Stream #0:\d+(?:\[0x[0-9a-fA-F]+\])?(?:\([^)]*\))?: Audio: (.+)");
        assert!(!re.is_match("    title           : Stream #0:0: Audio: mp3, 8000 Hz, mono, 1 kb/s"));
        for real in [
            "  Stream #0:0: Audio: mp3 (mp3float), 44100 Hz, stereo, fltp, 128 kb/s",
            "  Stream #0:0[0x100]: Audio: aac (LC), 48000 Hz, stereo, fltp",
            "    Stream #0:1(eng): Audio: opus, 48000 Hz, stereo, fltp",
        ] {
            assert!(re.is_match(real), "{real}");
        }
    }

    #[test]
    fn parses_ffmpeg_stream_description() {
        let mut st = State::default();
        parse_stream(&mut st, "aac (HE-AAC), 44100 Hz, stereo, fltp, 64 kb/s");
        assert_eq!(st.in_codec.as_deref(), Some("aac (HE-AAC)"));
        assert_eq!(
            (st.in_rate, st.in_channels, st.in_bitrate),
            (Some(44100), Some(2), Some(64))
        );
        let mut st = State::default();
        parse_stream(&mut st, "mp3 (mp3float), 22050 Hz, mono, fltp, 32 kb/s");
        assert_eq!((st.in_rate, st.in_channels), (Some(22050), Some(1)));
    }
}
