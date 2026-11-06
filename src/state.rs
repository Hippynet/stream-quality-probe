//! Shared state, thresholds and small helpers.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use serde_json::Value;

use crate::frames::FrameScan;

// ---------------------------------------------------------------------------
// Thresholds (also reported in the JSON output)
// ---------------------------------------------------------------------------
pub const SR: u32 = 44100; // analysis sample rate (ffmpeg resamples to this)
pub const BLOCK_SEC: f64 = 0.25; // analysis block length
pub const BLOCK_FRAMES: usize = 11025;
pub const FFT_N: usize = 4096; // spectrum window for the bandwidth estimate
pub const SILENCE_DB: f64 = -60.0; // RMS below this on both channels = silence
pub const DEAD_AIR_SEC: f64 = 3.0; // silence run that counts as dead air
pub const MONO_CORR: f64 = 0.998; // L/R correlation above this = mono content
pub const PHASE_CORR: f64 = -0.5; // correlation below this = out of phase
pub const ONE_SIDED_DB: f64 = 30.0; // one channel this much quieter = dead channel
pub const PERSIST_SEC: f64 = 10.0; // channel condition must last this long to log an event
pub const STALL_SEC: f64 = 5.0; // no bytes for this long = stall (some CDNs/proxies legitimately send a chunk every ~2-4 s)
pub const DEAD_CONN_SEC: f64 = 15.0; // no bytes for this long = drop + reconnect
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const RECONNECT_DELAY: f64 = 2.0;
pub const PLAYER_PREBUFFER: f64 = 2.0; // simulated player buffers this much before playing
pub const STARTUP_GRACE_SEC: f64 = 3.0; // decoder resync noise ignored after (re)connect
pub const DEFAULT_NONTTY_SEC: f64 = 60.0; // default run length when not attached to a terminal
pub const JSON_TIMELINE_POINTS: usize = 120;
pub const PROBE_SAMPLE_BYTES: usize = 256 * 1024; // audio handed to ffprobe (it never connects itself)
pub const READ_DEADLINE: Duration = Duration::from_secs(20); // TLS + headers must arrive within this
pub const MAX_EVENTS: usize = 5000; // memory bound for very long runs / chatty servers
pub const MAX_TITLES: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Bad,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Bad => "bad",
        }
    }
}

/// Channel state of an analysis block.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Class {
    #[default]
    Stereo = 0,
    MonoContent = 1,
    MonoStream = 2,
    OutOfPhase = 3,
    LeftOnly = 4,
    RightOnly = 5,
    Silence = 6,
}

pub const CLASSES: [Class; 7] = [
    Class::Stereo,
    Class::MonoContent,
    Class::MonoStream,
    Class::OutOfPhase,
    Class::LeftOnly,
    Class::RightOnly,
    Class::Silence,
];

impl Class {
    pub fn idx(self) -> usize {
        self as usize
    }
    pub fn code(self) -> &'static str {
        match self {
            Class::Stereo => "stereo",
            Class::MonoContent => "mono_content",
            Class::MonoStream => "mono_stream",
            Class::OutOfPhase => "out_of_phase",
            Class::LeftOnly => "left_only",
            Class::RightOnly => "right_only",
            Class::Silence => "silence",
        }
    }
    pub fn glyph(self) -> char {
        match self {
            Class::Stereo => 'S',
            Class::MonoContent => 'm',
            Class::MonoStream => '1',
            Class::OutOfPhase => 'Ø',
            Class::LeftOnly => 'L',
            Class::RightOnly => 'R',
            Class::Silence => '-',
        }
    }
    /// (event level, message) for conditions worth logging when they persist.
    pub fn alert(self) -> Option<(Level, &'static str)> {
        match self {
            Class::MonoContent => Some((Level::Warn, "Mono content (left = right) in a stereo stream")),
            Class::OutOfPhase => Some((
                Level::Bad,
                "Out of phase: one channel is polarity-inverted (cancels when summed to mono)",
            )),
            Class::LeftOnly => Some((Level::Bad, "Only the LEFT channel has audio - right channel is dead")),
            Class::RightOnly => Some((Level::Bad, "Only the RIGHT channel has audio - left channel is dead")),
            _ => None,
        }
    }
}

/// Everything that happened during one wall-clock second.
#[derive(Default, Clone)]
pub struct Bucket {
    pub rms_sum: f64,
    pub rms_n: u32,
    pub lufs_sum: f64,
    pub lufs_n: u32,
    pub classes: [u32; 7],
    pub clip: u64,
    pub silent_blocks: u32,
    pub blocks: u32,
    pub net_bytes: u64,
    pub stall: bool,
    pub disconnected: bool,
    pub underrun: bool,
    pub title: bool,
    pub decode_err: u32,
}

pub struct Event {
    pub t: f64,
    pub level: Level,
    pub code: String,
    pub msg: String,
}

#[derive(Clone, Default, Debug)]
pub struct ConnInfo {
    pub host: String,
    pub port: u16,
    pub scheme: String,
    pub dns_ms: f64,
    pub addrs: Vec<String>,
    pub ip: String,
    pub connect_ms: f64,
    pub tls_ms: Option<f64>,
    pub tls_version: Option<String>,
    pub cert_days_left: Option<f64>,
    pub cert_expires: Option<String>, // YYYY-MM-DD
    pub cert_issuer: Option<String>,
    pub tls_cipher: Option<String>,
    pub headers_ms: f64,
    pub status: String,
}

/// Latest block measurements, for the live meters.
#[derive(Clone, Copy, Default)]
pub struct Current {
    pub rms_l: f64,
    pub rms_r: f64,
    pub pk_l: f64,
    pub pk_r: f64,
    pub corr: f64,
    pub side_db: f64,
    pub class: Class,
}

#[derive(Default)]
pub struct State {
    pub url: String,
    pub final_url: String,
    pub started_wall: DateTime<Local>,
    pub events: Vec<Event>,
    pub buckets: Vec<Bucket>,

    // connection
    pub conn: Option<ConnInfo>,
    pub first_conn: Option<ConnInfo>,
    pub headers: Vec<(String, String)>,
    pub mode_direct: bool, // HLS: ffmpeg reads the URL itself
    pub connected: bool,
    pub disconnects: u32,
    pub reconnect_fail: u32,
    pub redirects: Vec<(u16, String)>,
    pub tls_error: Option<String>,
    pub playlist_from: Option<String>,
    pub fatal: Option<(String, String)>, // (code, message)
    pub no_audio: bool,
    pub net_bytes_total: u64,
    pub audio_bytes_total: u64,
    pub stall_active: bool,
    pub stalls: u32,
    pub allow: crate::net::Allow, // non-public addresses allowed this session (net::start_allows_internal)
    pub net_window: VecDeque<(f64, u64)>,
    pub last_connect: f64,

    // metadata
    pub metaint: usize,
    pub title: Option<String>,
    pub title_since: f64,
    pub titles: Vec<(f64, String)>,
    pub meta_non_utf8: bool,
    pub meta_packets: u64,
    pub titles_dropped: u64,
    pub events_dropped: u64,
    pub events_total: u64,
    pub decoder_gone: bool, // ffmpeg stopped reading our stream (it exited)

    // format
    pub ffprobe_audio: Option<Value>,
    pub ffprobe_tags: Option<Value>,
    pub ffprobe_err: Option<String>,
    pub ffprobe_format: Option<Value>,
    pub probe_sample: Vec<u8>,
    pub frames: FrameScan,
    pub in_codec: Option<String>,
    pub in_rate: Option<u32>,
    pub in_channels: Option<u32>,
    pub in_layout: Option<String>,
    pub in_bitrate: Option<u32>,
    pub in_line: Option<String>,

    // loudness (EBU R128)
    pub lufs_m: Option<f64>,
    pub lufs_s: Option<f64>,
    pub lufs_i: Option<f64>,
    pub lra: Option<f64>,
    pub tpk: Option<f64>,
    pub tpk_max: f64,

    // audio analysis
    pub audio_sec: f64,
    pub first_audio: Option<f64>,
    pub last_block: Option<f64>,
    pub cur: Option<Current>,
    pub class_sec: [f64; 7],
    pub rms_sum: f64,
    pub rms_n: u64,
    pub peak_max: f64,
    pub clip_total: u64,
    pub overs_total: u64,
    pub dc: [f64; 2],
    pub dc_n: u64,
    pub silence_run: f64,
    pub silence_start: Option<f64>,
    pub silence_reported: bool,
    pub silence_events: u32,
    pub silence_longest: f64,
    pub silence_total: f64,
    pub class_run: Option<Class>,
    pub class_run_start: f64,
    pub class_run_len: f64,
    pub class_reported: bool,
    pub spec_n: u32,
    pub bandwidth: Option<f64>,
    pub decode_errors: u64,
    pub sync_errors: u64,
    pub ffmpeg_alive: bool,

    // simulated listener buffer
    pub playing: bool,
    pub play_t0: f64,
    pub play_base: f64,
    pub buffer_sec: f64,
    pub buffer_min: Option<f64>,
    pub underruns: u32,
    pub rt_window: VecDeque<(f64, f64)>,
}

impl State {
    pub fn bucket(&mut self, t: f64) -> &mut Bucket {
        let i = if t > 0.0 { t as usize } else { 0 };
        if self.buckets.len() <= i {
            self.buckets.resize_with(i + 1, Bucket::default);
        }
        &mut self.buckets[i]
    }

    pub fn event(&mut self, t: f64, level: Level, code: &str, msg: impl Into<String>) {
        if self.events.len() >= MAX_EVENTS {
            // keep the first 100 (how the session started) and the most recent
            self.events.remove(100);
            self.events_dropped += 1;
        }
        self.events_total += 1;
        self.events.push(Event {
            t,
            level,
            code: code.to_string(),
            msg: clean(&msg.into()),
        });
    }

    pub fn push_title(&mut self, t: f64, title: String) {
        if self.titles.len() >= MAX_TITLES {
            self.titles.remove(0);
            self.titles_dropped += 1;
        }
        self.titles.push((t, title));
    }

    pub fn sorted_events(&self) -> Vec<&Event> {
        let mut v: Vec<&Event> = self.events.iter().collect();
        v.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    pub fn header(&self, k: &str) -> Option<&str> {
        header(&self.headers, k)
    }

    /// Audio seconds decoded per wall-clock second over the last ~15 s.
    pub fn realtime_ratio(&self) -> Option<f64> {
        if self.mode_direct {
            return None; // HLS arrives in segment bursts; a short-window ratio is meaningless
        }
        let w = &self.rt_window;
        let (a, b) = (w.front()?, w.back()?);
        if w.len() < 2 || b.0 - a.0 < 3.0 {
            return None;
        }
        Some((b.1 - a.1) / (b.0 - a.0))
    }

    /// Network bytes/s over the last ~10 s.
    pub fn net_rate(&self, now: f64) -> f64 {
        let recent: Vec<&(f64, u64)> = self.net_window.iter().filter(|(t, _)| now - t <= 10.0).collect();
        match recent.first() {
            None => 0.0,
            Some(first) => recent.iter().map(|(_, b)| *b as f64).sum::<f64>() / (now - first.0).max(1.0),
        }
    }

    pub fn measured_kbps(&self) -> Option<f64> {
        if self.mode_direct || self.audio_sec < 5.0 || self.audio_bytes_total == 0 {
            return None;
        }
        Some(self.audio_bytes_total as f64 * 8.0 / 1000.0 / self.audio_sec)
    }

    pub fn wall(&self, t: f64) -> DateTime<Local> {
        self.started_wall + chrono::Duration::milliseconds((t * 1000.0) as i64)
    }

    pub fn hms(&self, t: f64) -> String {
        self.wall(t).format("%H:%M:%S").to_string()
    }
}

/// Make server-controlled text safe to show: control characters (C0, DEL,
/// C1, so ESC-based terminal escape sequences, BEL, CR) and bidi overrides
/// (which can visually reorder a line) become U+FFFD; tabs and newlines
/// become spaces. Station text must never reach the terminal raw: it could
/// retitle the window, forge or hide dashboard lines, or write the clipboard
/// (OSC 52) on terminals that allow it.
pub fn clean(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\t' | '\n' | '\u{2028}' | '\u{2029}' => Some(' '),
            // bidi overrides/isolates: can make a line read differently from its bytes
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => Some('\u{FFFD}'),
            // invisible format characters that can hide text: zero-width space,
            // bidi marks, word joiner/invisible operators, BOM, interlinear
            // annotation, deprecated format chars, Unicode "tag" characters.
            // (ZWNJ/ZWJ 200C/200D and variation selectors stay: scripts and emoji need them.)
            '\u{200B}' | '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}' => None,
            '\u{2060}'..='\u{2064}' | '\u{206A}'..='\u{206F}' | '\u{FFF9}'..='\u{FFFB}' => None,
            '\u{E0000}'..='\u{E007F}' => None,
            c if c.is_control() => Some('\u{FFFD}'),
            c => Some(c),
        })
        .collect()
}

/// Clean and cap station-controlled free text (titles, headers, tags)
/// before it is stored: it ends up on the terminal and in reports that
/// people and other tools read.
pub fn cap(s: &str, max: usize) -> String {
    let s = clean(s);
    if s.chars().count() <= max {
        s
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

pub fn header<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
    h.iter().find(|(name, _)| name == k).map(|(_, v)| v.as_str())
}

/// State shared between the network, decoder, analysis and UI threads.
pub struct Shared {
    pub started: Instant,
    pub insecure: bool,
    pub allow_internal: bool, // --allow-internal
    pub stop: AtomicBool,
    /// Past connecting and into the main loop (Ctrl-C before this exits at once).
    pub streaming: AtomicBool,
    /// The dashboard has the terminal in raw mode / alternate screen.
    pub ui_active: AtomicBool,
    /// Running ffprobe, so every exit path can kill it.
    pub ffprobe_child: Mutex<Option<std::process::Child>>,
    st: Mutex<State>,
}

impl Shared {
    pub fn new(url: &str, insecure: bool, allow_internal: bool) -> Self {
        let st = State {
            url: url.to_string(),
            final_url: url.to_string(),
            started_wall: Local::now(),
            tpk_max: -200.0,
            peak_max: -200.0,
            ..Default::default()
        };
        Shared {
            started: Instant::now(),
            insecure,
            allow_internal,
            stop: AtomicBool::new(false),
            streaming: AtomicBool::new(false),
            ui_active: AtomicBool::new(false),
            ffprobe_child: Mutex::new(None),
            st: Mutex::new(st),
        }
    }
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Kill ffprobe if it's still running (called on every exit path).
    pub fn kill_ffprobe(&self) {
        if let Some(mut c) = self.ffprobe_child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn set_stop(&self) {
        self.stop.store(true, Ordering::Relaxed)
    }
    /// Sleep for `secs`, waking early if stop is requested.
    pub fn sleep(&self, secs: f64) {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        while !self.stopped() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

// ---------------------------------------------------------------------------
// Number helpers
// ---------------------------------------------------------------------------
pub fn to_db(p: f64) -> f64 {
    if p > 1e-20 {
        10.0 * p.log10()
    } else {
        -200.0
    }
}

pub fn amp_db(a: f64) -> f64 {
    if a > 1e-10 {
        20.0 * a.log10()
    } else {
        -200.0
    }
}

/// dB value for display.
pub fn fdb(v: Option<f64>) -> String {
    match v {
        None => "--".into(),
        Some(x) if x <= -199.0 || !x.is_finite() => "-inf".into(),
        Some(x) => format!("{x:.1}"),
    }
}

pub fn round(x: f64, nd: i32) -> f64 {
    let m = 10f64.powi(nd);
    (x * m).round() / m
}

/// Number for JSON: null when missing / -inf / NaN.
pub fn jnum(v: Option<f64>, nd: i32) -> Value {
    match v {
        Some(x) if x.is_finite() && x > -199.0 => serde_json::json!(round(x, nd)),
        _ => Value::Null,
    }
}

pub fn fmt_dur(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_text_cannot_carry_terminal_escapes() {
        for evil in [
            "\x1b]0;pwn\x07A - B",
            "\x1b[2J\x1b[HFAKE LINE",
            "\u{9b}31mC1 CSI",
            "a\u{202E}gnp.exe",
            "\x1b]52;c;ZXZpbA==\x07",
        ] {
            let c = cap(evil, 200);
            assert!(
                !c.chars()
                    .any(|ch| ch.is_control() || ('\u{202A}'..='\u{202E}').contains(&ch)),
                "{c:?}"
            );
        }
        assert_eq!(clean("A\tB\nC"), "A B C");
        // hidden text: zero-width and Unicode tag characters vanish; emoji ZWJ sequences survive
        assert_eq!(clean("a\u{200B}b\u{FEFF}c\u{E0041}\u{E0042}d"), "abcd");
        assert_eq!(clean("👩\u{200D}🎤"), "👩\u{200D}🎤");
        assert_eq!(cap("\x1b]0;pwn\x07A - B", 200), "\u{FFFD}]0;pwn\u{FFFD}A - B");
        let mut st = State::default();
        st.event(0.0, Level::Info, "title", "\x1b[31mred");
        assert!(!st.events[0].msg.contains('\x1b'));
    }

    #[test]
    fn events_and_titles_are_bounded() {
        let mut st = State::default();
        for i in 0..(MAX_EVENTS + 500) {
            st.event(i as f64, Level::Info, "title", format!("e{i}"));
        }
        assert_eq!(st.events.len(), MAX_EVENTS);
        assert_eq!(st.events_dropped, 500);
        assert_eq!(st.events_total, (MAX_EVENTS + 500) as u64);
        assert_eq!(st.events[0].msg, "e0"); // the start of the session is kept
        assert_eq!(st.events.last().unwrap().msg, format!("e{}", MAX_EVENTS + 499));
        for i in 0..(MAX_TITLES + 10) {
            st.push_title(i as f64, i.to_string());
        }
        assert_eq!((st.titles.len(), st.titles_dropped), (MAX_TITLES, 10));
    }
}
