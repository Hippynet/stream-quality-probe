//! Terminal output: live dashboard, status lines and the text report.

use crate::findings::{codec_name, codec_profile, declared_kbps, findings, overall};
use crate::state::*;
use crate::timeline::{column, pick_zoom, Delivery};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy)]
pub struct Pal {
    pub on: bool,
}

impl Pal {
    fn c(&self, s: &'static str) -> &'static str {
        if self.on {
            s
        } else {
            ""
        }
    }
    pub fn reset(&self) -> &'static str {
        self.c("\x1b[0m")
    }
    pub fn bold(&self) -> &'static str {
        self.c("\x1b[1m")
    }
    pub fn dim(&self) -> &'static str {
        self.c("\x1b[2m")
    }
    pub fn red(&self) -> &'static str {
        self.c("\x1b[31m")
    }
    pub fn green(&self) -> &'static str {
        self.c("\x1b[32m")
    }
    pub fn yellow(&self) -> &'static str {
        self.c("\x1b[33m")
    }
    pub fn magenta(&self) -> &'static str {
        self.c("\x1b[35m")
    }
    pub fn cyan(&self) -> &'static str {
        self.c("\x1b[36m")
    }
    pub fn icon(&self, l: Level) -> String {
        let (ch, col) = match l {
            Level::Ok => ("✔", self.green()),
            Level::Warn => ("▲", self.yellow()),
            Level::Bad => ("✖", self.red()),
            Level::Info => ("•", self.cyan()),
        };
        format!("{col}{ch}{}", self.reset())
    }
    fn class_col(&self, c: Class) -> &'static str {
        match c {
            Class::Stereo => self.green(),
            Class::MonoContent => self.yellow(),
            Class::MonoStream => self.cyan(),
            Class::Silence => self.dim(),
            _ => self.red(),
        }
    }
}

/// Truncate a string containing ANSI codes to `width` visible characters.
pub fn clip_line(s: &str, width: usize, p: Pal) -> String {
    let mut out = String::new();
    let mut n = 0;
    let mut chars = s.chars().peekable();
    let mut truncated = false;
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            out.push(ch);
            for c in chars.by_ref() {
                out.push(c);
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        let w = UnicodeWidthChar::width(ch).unwrap_or(0); // CJK/emoji take 2 columns
        if n + w > width {
            truncated = true;
            break;
        }
        out.push(ch);
        n += w;
    }
    if truncated {
        out.push_str(p.reset());
    }
    out
}

const SPARK: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub fn timeline_lines(st: &State, now: f64, width: usize, p: Pal) -> Vec<String> {
    let label_w = 9;
    let cols = width.saturating_sub(label_w + 1).max(10);
    let elapsed = now as usize + 1;
    let zoom = pick_zoom(elapsed, cols);
    let ncols = cols.min(elapsed.div_ceil(zoom));
    let (mut lvl, mut ste, mut clip, mut net, mut trk) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
    );
    let r = p.reset();
    for c in 0..ncols {
        let d = column(st, c * zoom, zoom);
        if d.blocks == 0 {
            lvl += &format!("{}·{r}", p.dim());
        } else if d.silent {
            lvl += &format!("{}_{r}", p.red());
        } else {
            let v = d.lufs.unwrap_or(d.rms.unwrap_or(-60.0) + 3.0);
            let k = (((v + 40.0) / 35.0 * 7.0).round() as i64 + 1).clamp(1, 8) as usize; // -40..-5 LUFS
            let col = if (-24.0..=-7.0).contains(&v) {
                p.green()
            } else {
                p.yellow()
            };
            lvl += &format!("{col}{}{r}", SPARK[k]);
        }
        match d.stereo {
            None => ste += &format!("{}·{r}", p.dim()),
            Some(s) => ste += &format!("{}{}{r}", p.class_col(s), s.glyph()),
        }
        clip += &if d.blocks == 0 {
            " ".to_string()
        } else if d.clip > 20 {
            format!("{}!{r}", p.red())
        } else if d.clip > 0 {
            format!("{}'{r}", p.yellow())
        } else {
            format!("{}·{r}", p.dim())
        };
        net += &match d.delivery {
            Some(Delivery::Disconnect) => format!("{}X{r}", p.red()),
            Some(Delivery::Underrun) => format!("{}u{r}", p.red()),
            Some(Delivery::Stall) => format!("{}~{r}", p.yellow()),
            Some(Delivery::DecodeError) => format!("{}e{r}", p.yellow()),
            Some(Delivery::Ok) => format!("{}━{r}", p.green()),
            None => " ".into(),
        };
        trk += &if d.title_change {
            format!("{}♪{r}", p.magenta())
        } else {
            " ".into()
        };
    }
    let mut axis: Vec<char> = vec![' '; ncols];
    let mut c = 0;
    while c < ncols {
        let full = st.hms((c * zoom) as f64);
        let label: String = if zoom >= 60 { full[..5].to_string() } else { full };
        if c + 1 + label.len() <= ncols {
            axis[c] = '|';
            for (j, ch) in label.chars().enumerate() {
                axis[c + 1 + j] = ch;
            }
        }
        c += label.len() + 4;
    }
    let zl = if zoom < 60 {
        format!("{zoom}s")
    } else {
        format!("{}m", zoom / 60)
    };
    let lab = |s: &str| format!("{}{s:<label_w$}{r}", p.dim());
    let pad = " ".repeat(label_w);
    vec![
        format!("{}TIMELINE{r}{}  1 column = {zl}, whole session, now on the right{r}", p.bold(), p.dim()),
        lab("level") + &lvl,
        lab("stereo") + &ste,
        lab("clip") + &clip,
        lab("delivery") + &net,
        lab("track") + &trk,
        lab("") + p.dim() + &axis.into_iter().collect::<String>() + r,
        format!(
            "{}{pad}level ▁..█ loudness, _ silence · stereo S ok, m mono content, Ø out of phase, L/R one channel only, 1 mono stream{r}",
            p.dim()
        ),
        format!(
            "{}{pad}clip ' some, ! heavy · delivery ━ ok, ~ stall, u listener underrun, X disconnect, e decoder error · ♪ title change{r}",
            p.dim()
        ),
    ]
}

fn meter(db: f64, width: usize, p: Pal) -> String {
    let (lo, hi) = (-60.0, 0.0);
    let n = (((db - lo) / (hi - lo)).clamp(0.0, 1.0) * width as f64).round() as usize;
    let mut s = String::new();
    for i in 0..width {
        let pos = lo + (hi - lo) * (i as f64 + 0.5) / width as f64;
        if i < n {
            s += if pos > -3.0 {
                p.red()
            } else if pos > -12.0 {
                p.yellow()
            } else {
                p.green()
            };
            s.push('█');
        } else {
            s += p.dim();
            s.push('░');
        }
    }
    s + p.reset()
}

fn lab(s: &str, p: Pal) -> String {
    format!("{}{s:<10}{}", p.bold(), p.reset())
}

pub fn format_text(st: &State, p: Pal) -> String {
    let a = st.ffprobe_audio.as_ref();
    let mut parts = Vec::new();
    if let Some(codec) = codec_name(st) {
        parts.push(match codec_profile(st) {
            Some(pr) => format!("{codec} ({pr})"),
            None => codec,
        });
    }
    let rate = st.in_rate.or_else(|| a?.get("sample_rate")?.as_str()?.parse().ok());
    if let Some(r) = rate {
        parts.push(format!("{} kHz", r as f64 / 1000.0));
    }
    if let Some(l) = st
        .in_layout
        .clone()
        .or_else(|| a?.get("channel_layout")?.as_str().map(String::from))
    {
        parts.push(l);
    }
    if let Some(d) = declared_kbps(st) {
        parts.push(format!("{d} kb/s declared"));
    }
    if let Some(m) = st.measured_kbps() {
        parts.push(format!("{m:.0} kb/s measured"));
    }
    if st.mode_direct {
        parts.push(format!("{}HLS (read by ffmpeg){}", p.dim(), p.reset()));
    }
    if parts.is_empty() {
        format!("{}probing…{}", p.dim(), p.reset())
    } else {
        parts.join("  ·  ")
    }
}

/// ffprobe's view in one line: "mp3 (score 51) · mp3float · fltp · stereo · 128 kb/s".
pub fn ffprobe_text(st: &State) -> Option<String> {
    let a = st.ffprobe_audio.as_ref()?;
    let s = |k: &str| {
        a.get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty() && *v != "unknown")
    };
    let mut v = Vec::new();
    let fmt = st.ffprobe_format.as_ref();
    let fname = fmt.and_then(|f| f.get("format_name")).and_then(|v| v.as_str());
    if let Some(f) = fmt {
        let long = f
            .get("format_long_name")
            .and_then(|v| v.as_str())
            .or(fname)
            .unwrap_or("?");
        match f.get("probe_score").and_then(|v| v.as_u64()) {
            Some(sc) => v.push(format!("{long} (score {sc})")),
            None => v.push(long.to_string()),
        }
    }
    v.extend(s("codec_name").filter(|c| Some(*c) != fname).map(String::from));
    v.extend(s("profile").map(String::from));
    v.extend(s("sample_fmt").map(String::from));
    v.extend(s("channel_layout").map(String::from));
    if let Some(br) = s("bit_rate").and_then(|b| b.parse::<f64>().ok()) {
        v.push(format!("{:.0} kb/s", br / 1000.0));
    }
    (!v.is_empty()).then(|| v.join("  ·  "))
}

/// Frame-header analysis in one line, with sync health.
pub fn frames_text(st: &State, p: Pal) -> Option<String> {
    let fs = &st.frames.stats;
    let mut line = fs.summary()?;
    if fs.kind.is_some() {
        let losses = if fs.sync_losses > 0 {
            format!("{}{}{}", p.yellow(), fs.sync_losses, p.reset())
        } else {
            "0".into()
        };
        line += &format!("  ·  sync losses {losses}");
        if fs.id3_tags > 0 {
            line += &format!("  ·  {} ID3 tag(s)", fs.id3_tags);
        }
        if fs.format_changes > 0 {
            line += &format!("  ·  {}{} format change(s){}", p.yellow(), fs.format_changes, p.reset());
        }
    }
    Some(line)
}

/// The station / stream metadata panel at the top of the dashboard, as
/// (priority, line). Priority 0 is always shown; on a short terminal the
/// highest numbers are dropped first (after the event log has shrunk).
fn info_panel(st: &State, p: Pal) -> Vec<(u8, String)> {
    let r = p.reset();
    let dim = |s: &str| format!("{}{s}{r}", p.dim());
    let mut v = Vec::new();

    let mut bits = Vec::new();
    if let Some(n) = st.header("icy-name") {
        bits.push(format!("{}{n}{r}", p.bold()));
    }
    if let Some(g) = st.header("icy-genre") {
        bits.push(g.to_string());
    }
    if let Some(u) = st.header("icy-url") {
        bits.push(dim(u));
    }
    if let Some(pb) = st
        .header("icy-pub")
        .or(st.header("ice-public"))
        .or(st.header("icy-public"))
    {
        bits.push(dim(if pb.trim() == "1" { "listed" } else { "unlisted" }));
    }
    if !bits.is_empty() {
        v.push((0, lab("STATION", p) + &bits.join("  ·  ")));
    }
    if let Some(d) = st.header("icy-description").filter(|d| !d.trim().is_empty()) {
        v.push((4, " ".repeat(10) + &dim(&format!("\"{d}\""))));
    }

    let conn = st.conn.as_ref().or(st.first_conn.as_ref());
    let mut s = Vec::new();
    if let Some(x) = st.header("server") {
        s.push(x.to_string());
    }
    if let Some(c) = conn.filter(|c| !c.status.is_empty()) {
        s.push(c.status.clone());
    }
    if let Some(ct) = st.header("content-type") {
        s.push(ct.to_string());
    }
    if st.metaint > 0 {
        s.push(format!("metaint {}", st.metaint));
    } else if !st.mode_direct && !st.headers.is_empty() {
        s.push(dim("no ICY metadata"));
    }
    if let Some(a) = st.header("access-control-allow-origin") {
        s.push(format!("CORS {a}"));
    }
    if !s.is_empty() {
        v.push((1, lab("SERVER", p) + &s.join("  ·  ")));
    }
    if let Some(c) = conn.filter(|c| c.tls_version.is_some()) {
        let mut t: Vec<String> = c.tls_version.iter().chain(c.tls_cipher.iter()).cloned().collect();
        if let Some(d) = c.cert_days_left {
            let days = format!("cert {d:.0} days left");
            t.push(if d < 14.0 {
                format!("{}{days}{r}", p.yellow())
            } else {
                days
            });
        }
        if let Some(e) = &c.cert_expires {
            t.push(dim(&format!("expires {e}")));
        }
        if let Some(i) = &c.cert_issuer {
            t.push(dim(&format!("issuer {i}")));
        }
        v.push((2, lab("TLS", p) + &t.join("  ·  ")));
    }

    v.push((0, lab("FORMAT", p) + &format_text(st, p)));
    if let Some(f) = frames_text(st, p) {
        v.push((2, lab("FRAMES", p) + &f));
    } else if st.mode_direct {
        v.push((2, lab("FRAMES", p) + &dim("n/a for HLS (ffmpeg reads the segments)")));
    }
    if let Some(f) = ffprobe_text(st) {
        v.push((3, lab("FFPROBE", p) + &f));
    } else if let Some(e) = &st.ffprobe_err {
        v.push((3, lab("FFPROBE", p) + &dim(&e.chars().take(100).collect::<String>())));
    } else if !st.mode_direct {
        v.push((3, lab("FFPROBE", p) + &dim("waiting for audio sample…")));
    }

    if let Some(t) = &st.title {
        let t = if t.is_empty() { "(empty)" } else { t };
        let prev = st
            .titles
            .len()
            .checked_sub(2)
            .and_then(|i| st.titles.get(i))
            .map(|(_, x)| format!("   {}prev: {x}{r}", p.dim()))
            .unwrap_or_default();
        v.push((
            0,
            format!(
                "{}{}{t}{r}  {}since {}{r}{prev}",
                lab("NOW", p),
                p.magenta(),
                p.dim(),
                st.hms(st.title_since)
            ),
        ));
    }
    v
}

fn class_text(c: Class) -> &'static str {
    match c {
        Class::Stereo => "STEREO",
        Class::MonoContent => "MONO CONTENT (L = R)",
        Class::MonoStream => "MONO STREAM",
        Class::OutOfPhase => "OUT OF PHASE",
        Class::LeftOnly => "LEFT ONLY - right channel dead",
        Class::RightOnly => "RIGHT ONLY - left channel dead",
        Class::Silence => "silence",
    }
}

/// Cached timeline (recomputing a multi-hour session 4x a second is wasteful).
pub struct TimelineCache {
    key: (u64, usize),
    lines: Vec<String>,
}

impl TimelineCache {
    pub fn new() -> Self {
        TimelineCache {
            key: (u64::MAX, 0),
            lines: Vec::new(),
        }
    }
}

pub fn render(st: &State, now: f64, width: usize, height: usize, p: Pal, cache: &mut TimelineCache) -> String {
    let r = p.reset();
    let mut out: Vec<String> = Vec::new();
    let alive = if st.mode_direct { st.ffmpeg_alive } else { st.connected };
    let status = if alive {
        format!("{}● LIVE{r}", p.green())
    } else {
        format!("{}● DOWN{r}", p.red())
    };
    out.push(format!(
        "{}{}streamprobe{r}  {status}  {}{}{r}  {}{}{r}   {}q / Ctrl-C = stop + report{r}",
        p.bold(),
        p.cyan(),
        p.bold(),
        fmt_dur(now),
        p.dim(),
        st.final_url,
        p.dim()
    ));
    out.push(format!("{}{}{r}", p.dim(), "─".repeat(width)));
    let panel_at = out.len(); // the info panel is spliced in here once we know the space left
    out.push(String::new());

    let fresh = st.last_block.is_some_and(|t| now - t < 2.0);
    let mw = width.saturating_sub(50).clamp(10, 40);
    match (st.cur, fresh) {
        (Some(c), true) => {
            for (i, (name, rms, pk)) in [("L", c.rms_l, c.pk_l), ("R", c.rms_r, c.pk_r)].into_iter().enumerate() {
                let head = if i == 0 { lab("LEVELS", p) } else { " ".repeat(10) };
                out.push(format!(
                    "{head}{name} {} RMS {:>6} dBFS   peak {:>6} dBFS",
                    meter(rms, mw, p),
                    fdb(Some(rms)),
                    fdb(Some(pk))
                ));
            }
        }
        _ => {
            let msg = if st.first_audio.is_none() {
                "waiting for audio…"
            } else {
                "NO AUDIO ARRIVING"
            };
            out.push(format!("{}{}{msg}{r}", lab("LEVELS", p), p.red()));
            out.push(String::new());
        }
    }
    out.push(format!(
        "{}momentary {}  short-term {}  integrated {}{}{r} LUFS   LRA {} LU   true-peak {} dBTP",
        lab("LOUDNESS", p),
        fdb(st.lufs_m),
        fdb(st.lufs_s),
        p.bold(),
        fdb(st.lufs_i),
        fdb(st.lra),
        fdb(st.tpk)
    ));
    match (st.cur, fresh) {
        (Some(c), true) => {
            let bal = c.rms_l - c.rms_r;
            let bal_s = if bal.abs() > 1.5 && c.class != Class::Silence {
                format!("   balance {bal:+.1} dB")
            } else {
                String::new()
            };
            out.push(format!(
                "{}{}{}{}{r}   correlation {:+.3}   side/mid {} dB{bal_s}",
                lab("STEREO", p),
                p.class_col(c.class),
                p.bold(),
                class_text(c.class),
                c.corr,
                fdb(Some(c.side_db))
            ));
        }
        _ => out.push(format!("{}{}--{r}", lab("STEREO", p), p.dim())),
    }
    let red = |v: String, bad: bool| if bad { format!("{}{v}{r}", p.red()) } else { v };
    let bw = st
        .bandwidth
        .map_or("measuring…".to_string(), |b| format!("~{:.1} kHz", b / 1000.0));
    out.push(format!(
        "{}bandwidth {bw}   overs >0dBFS {}   full-scale {}   max peak {} dBFS   decode errors {}",
        lab("QUALITY", p),
        red(st.overs_total.to_string(), st.overs_total > 50),
        st.clip_total,
        fdb(Some(st.peak_max)),
        red(st.decode_errors.to_string(), st.decode_errors > 0)
    ));
    let rts = match st.realtime_ratio() {
        None => "--".to_string(),
        Some(x) => format!("{}{x:.2}x{r}", if x >= 0.98 { p.green() } else { p.yellow() }),
    };
    let mut net = format!("realtime {rts}");
    if !st.mode_direct {
        net += &format!("   in {:.0} kb/s", st.net_rate(now) * 8.0 / 1000.0);
    }
    let bufc = if st.buffer_sec > 1.0 {
        p.green()
    } else if st.buffer_sec > 0.0 {
        p.yellow()
    } else {
        p.red()
    };
    net += &format!(
        "   listener buffer {bufc}{:.1}s{r}   underruns {}   stalls {}   disconnects {}",
        st.buffer_sec,
        red(st.underruns.to_string(), st.underruns > 0),
        red(st.stalls.to_string(), st.stalls > 0),
        red(st.disconnects.to_string(), st.disconnects > 0)
    );
    out.push(lab("DELIVERY", p) + &net);
    out.push(String::new());
    let key = (now as u64, width);
    if cache.key != key {
        cache.key = key;
        cache.lines = timeline_lines(st, now, width, p);
    }
    out.extend(cache.lines.iter().cloned());
    out.push(String::new());
    out.push(format!("{}CHECKS{r}", p.bold()));
    for f in findings(st, now, false) {
        out.push(format!("  {} {}", p.icon(f.level), f.msg));
    }
    out.push(String::new());
    let mut panel = info_panel(st, p);
    while out.len() + panel.len() > height {
        let drop = panel
            .iter()
            .enumerate()
            .filter(|(_, (prio, _))| *prio > 0)
            .max_by_key(|(i, (prio, _))| (*prio, *i))
            .map(|(i, _)| i);
        match drop {
            Some(i) => {
                panel.remove(i);
            }
            None => break,
        }
    }
    out.splice(panel_at..panel_at, panel.into_iter().map(|(_, l)| l));
    let remain = height.saturating_sub(out.len() + 1);
    if remain > 1 {
        out.push(format!("{}EVENTS{r}", p.bold()));
        let evs = st.sorted_events();
        let skip = evs.len().saturating_sub(remain - 1);
        for e in evs.into_iter().skip(skip) {
            out.push(format!("  {}{}{r} {} {}", p.dim(), st.hms(e.t), p.icon(e.level), e.msg));
        }
    }
    out.truncate(height);
    // raw mode: lines need explicit \r\n; \x1b[K clears leftovers from the previous frame
    out.iter()
        .map(|l| clip_line(l, width, p) + "\x1b[K")
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// One-line progress summary for non-interactive runs.
pub fn status_line(st: &State, now: f64) -> String {
    let cls = st.cur.map_or("--", |c| c.class.code());
    let bw = st.bandwidth.map_or("--".into(), |b| format!("{:.1}kHz", b / 1000.0));
    let rt = st.realtime_ratio().map_or("--".into(), |x| format!("{x:.2}x"));
    format!(
        "[{}] short-term {} LUFS, {cls}, bandwidth {bw}, realtime {rt}, buffer {:.1}s, underruns {}, stalls {}, disconnects {}, decode errors {}",
        fmt_dur(now),
        fdb(st.lufs_s),
        st.buffer_sec,
        st.underruns,
        st.stalls,
        st.disconnects,
        st.decode_errors
    )
}

pub fn text_report(st: &State, now: f64, p: Pal, stop_reason: &str) -> String {
    let b = |s: &str| format!("{}{s}{}", p.bold(), p.reset());
    let fs = findings(st, now, true);
    let mut r: Vec<String> = Vec::new();
    r.push(format!(
        "{}   {}   duration {}   result: {}",
        b("STREAM PROBE REPORT"),
        st.started_wall.format("%Y-%m-%d %H:%M:%S"),
        fmt_dur(now),
        b(&overall(&fs).to_uppercase())
    ));
    r.push("=".repeat(78));
    r.push(format!("URL        {}", st.url));
    if st.final_url != st.url {
        r.push(format!("Stream     {}", st.final_url));
    }
    for (s, to) in &st.redirects {
        r.push(format!("Redirect   {s} -> {to}"));
    }
    if let Some(c) = st.first_conn.as_ref().or(st.conn.as_ref()) {
        let all = if c.addrs.len() > 1 {
            format!("  (all: {})", c.addrs.join(", "))
        } else {
            String::new()
        };
        r.push(format!(
            "Server     {}:{}  ip {}{all}",
            c.host,
            c.port,
            if c.ip.is_empty() { "?" } else { &c.ip }
        ));
        let mut tim = format!("dns {:.0} ms, tcp {:.0} ms", c.dns_ms, c.connect_ms);
        if let Some(t) = c.tls_ms {
            tim += &format!(", tls {t:.0} ms ({}", c.tls_version.as_deref().unwrap_or("?"));
            if let Some(d) = c.cert_days_left {
                tim += &format!(", cert valid {d:.0} more days");
            }
            tim += ")";
        }
        tim += &format!(", headers {:.0} ms", c.headers_ms);
        if let Some(fa) = st.first_audio {
            tim += &format!(", first audio {fa:.1} s");
        }
        r.push(format!("Timing     {tim}"));
        if c.tls_version.is_some() {
            let mut t = vec![c.tls_version.clone().unwrap_or_default()];
            t.extend(c.tls_cipher.clone());
            t.extend(c.cert_issuer.as_ref().map(|i| format!("issuer {i}")));
            if let (Some(e), Some(d)) = (&c.cert_expires, c.cert_days_left) {
                t.push(format!("expires {e} ({d:.0} days)"));
            }
            r.push(format!("TLS        {}", t.join(", ")));
        }
        if !c.status.is_empty() {
            r.push(format!("Response   {}", c.status));
        }
    }
    const MAIN_HEADERS: [&str; 12] = [
        "content-type",
        "server",
        "icy-name",
        "icy-genre",
        "icy-description",
        "icy-url",
        "icy-br",
        "icy-sr",
        "ice-audio-info",
        "icy-metaint",
        "icy-pub",
        "access-control-allow-origin",
    ];
    for k in MAIN_HEADERS {
        if let Some(v) = st.header(k) {
            r.push(format!("  {k:<28} {}", v.chars().take(100).collect::<String>()));
        }
    }
    if !st.headers.is_empty() && st.header("access-control-allow-origin").is_none() {
        r.push(format!("  {:<28} (not set)", "access-control-allow-origin"));
    }
    for (k, v) in st
        .headers
        .iter()
        .filter(|(k, _)| !MAIN_HEADERS.contains(&k.as_str()))
        .take(40)
    {
        r.push(format!(
            "  {:<28} {}",
            cap(k, 28),
            v.chars().take(100).collect::<String>()
        ));
    }
    if st.fatal.is_none() {
        r.push(String::new());
        r.push(b("FORMAT"));
        r.push(format!("  {}", format_text(st, p)));
        if let Some(l) = &st.in_line {
            r.push(format!("  ffmpeg:  {l}"));
        }
        if let Some(f) = frames_text(st, p) {
            let fs = &st.frames.stats;
            r.push(format!("  frames:  {f}"));
            if fs.kind.is_some() {
                r.push(format!(
                    "           {} frames, {} bytes of junk, {} lead-in bytes skipped at (re)connect",
                    fs.frames, fs.junk_bytes, fs.lead_in_bytes
                ));
            }
        }
        if let Some(f) = ffprobe_text(st) {
            r.push(format!("  probe:   {f}"));
        }
        if let Some(a) = &st.ffprobe_audio {
            let keys = [
                "codec_long_name",
                "profile",
                "sample_fmt",
                "sample_rate",
                "channels",
                "channel_layout",
                "bit_rate",
            ];
            let kv: Vec<String> = keys
                .iter()
                .filter_map(|k| {
                    a.get(*k)
                        .map(|v| format!("{k}={}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                })
                .collect();
            r.push(format!("  ffprobe: {}", kv.join(", ")));
        } else if let Some(e) = &st.ffprobe_err {
            r.push(format!("  ffprobe: {e}"));
        }
        r.push(String::new());
        r.push(b("AUDIO"));
        r.push(format!(
            "  Loudness   integrated {} LUFS, LRA {} LU, max true peak {} dBTP",
            fdb(st.lufs_i),
            fdb(st.lra),
            fdb(Some(st.tpk_max))
        ));
        let avg = (st.rms_n > 0).then(|| to_db(st.rms_sum / st.rms_n as f64));
        r.push(format!(
            "  Levels     average RMS {} dBFS, max peak {} dBFS, {} overs, {} full-scale samples",
            fdb(avg),
            fdb(Some(st.peak_max)),
            st.overs_total,
            st.clip_total
        ));
        let tot: f64 = st.class_sec.iter().sum();
        if tot > 0.0 {
            let mut v: Vec<(Class, f64)> = CLASSES
                .iter()
                .map(|c| (*c, st.class_sec[c.idx()]))
                .filter(|x| x.1 > 0.0)
                .collect();
            v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let s: Vec<String> = v
                .iter()
                .map(|(c, s)| format!("{} {:.0}%", c.code().replace('_', " "), 100.0 * s / tot))
                .collect();
            r.push(format!("  Channels   {}", s.join(", ")));
        }
        r.push(format!(
            "  Bandwidth  {}",
            st.bandwidth.map_or("not measured".into(), |b| format!(
                "~{:.1} kHz ({} spectra)",
                b / 1000.0,
                st.spec_n
            ))
        ));
        r.push(format!(
            "  Dead air   {} run(s) of {DEAD_AIR_SEC:.0}s+ silence",
            st.silence_events
        ));
        r.push(String::new());
        r.push(b("DELIVERY"));
        let mut d = format!("  {:.0}s of audio in {:.0}s", st.audio_sec, now);
        if let Some(rt) = st.realtime_ratio() {
            d += &format!(", realtime {rt:.3}x");
        }
        if let Some(m) = st.buffer_min {
            d += &format!(", min listener buffer {m:.1}s");
        }
        r.push(d);
        r.push(format!(
            "  underruns {}, stalls {}, disconnects {}, decoder errors {}",
            st.underruns, st.stalls, st.disconnects, st.decode_errors
        ));
        if !st.titles.is_empty() {
            r.push(String::new());
            r.push(b("NOW-PLAYING HISTORY"));
            let skip = st.titles.len().saturating_sub(30);
            for (t, title) in st.titles.iter().skip(skip) {
                r.push(format!(
                    "  {}  {}",
                    st.hms(*t),
                    if title.is_empty() { "(empty)" } else { title }
                ));
            }
        }
    }
    r.push(String::new());
    r.push(b("VERDICT"));
    for f in &fs {
        r.push(format!("  {} {}", p.icon(f.level), f.msg));
    }
    if st.fatal.is_none() {
        r.push(String::new());
        r.extend(timeline_lines(st, now, 100, p));
    }
    r.push(String::new());
    r.push(b("EVENTS"));
    let evs = st.sorted_events();
    if evs.is_empty() {
        r.push("  (none)".into());
    }
    for e in evs {
        r.push(format!("  {} {} {}", st.hms(e.t), p.icon(e.level), e.msg));
    }
    if !stop_reason.is_empty() {
        r.push(String::new());
        r.push(format!("{}({stop_reason}){}", p.dim(), p.reset()));
    }
    r.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state with a bit of everything, as after a minute of a real stream.
    fn busy_state() -> State {
        let mut st = State {
            url: "https://example.com/live".into(),
            final_url: "https://example.com/live".into(),
            tpk_max: -1.0,
            peak_max: -1.0,
            ..Default::default()
        };
        st.headers = vec![
            ("icy-name".into(), "Example FM ".repeat(20)),
            ("icy-description".into(), "ünïcödé 🎵 text ".repeat(10)),
            ("server".into(), "Icecast 2.4.4".into()),
            ("content-type".into(), "audio/mpeg".into()),
            ("x-other".into(), "1".into()),
        ];
        st.conn = Some(ConnInfo {
            status: "HTTP/1.0 200 OK".into(),
            tls_version: Some("TLSv1.3".into()),
            cert_days_left: Some(3.0),
            ..Default::default()
        });
        st.title = Some("Artist - Title 🎶".repeat(8));
        for i in 0..80 {
            let t = i as f64;
            st.push_title(t, format!("Song {i}"));
            st.event(t, Level::Warn, "stall", format!("event {i} ✓"));
            let b = st.bucket(t);
            b.blocks = 4;
            b.lufs_sum = -14.0;
            b.lufs_n = 1;
        }
        st.first_audio = Some(0.5);
        st.last_block = Some(79.0);
        st.cur = Some(Current::default());
        st.frames.push(&[0xFF, 0xFB, 0x90, 0x64]);
        st
    }

    #[test]
    fn dashboard_and_reports_render_at_any_size() {
        let st = busy_state();
        for p in [Pal { on: true }, Pal { on: false }] {
            for (w, h) in [
                (0, 0),
                (1, 1),
                (5, 3),
                (20, 8),
                (40, 12),
                (80, 24),
                (132, 50),
                (400, 200),
            ] {
                let mut cache = TimelineCache::new();
                let out = render(&st, 80.0, w, h, p, &mut cache);
                assert!(out.split("\r\n").count() <= h.max(1), "{w}x{h}");
            }
            let _ = text_report(&st, 80.0, p, "time limit");
            let _ = status_line(&st, 80.0);
        }
        let _ = crate::report::json_report(&st, 80.0, "time limit");
    }

    #[test]
    fn wide_characters_are_clipped_by_display_width() {
        let p = Pal { on: false };
        assert_eq!(clip_line("日本語のタイトル", 7, p), "日本語"); // 6 columns; a 4th would make 8
        assert_eq!(clip_line("🎵🎵🎵", 4, p), "🎵🎵");
        assert_eq!(clip_line("\x1b[1mabc\x1b[0m", 2, p), "\x1b[1mab");
    }

    #[test]
    fn info_panel_keeps_essentials_on_short_terminals() {
        let st = busy_state();
        let mut cache = TimelineCache::new();
        let tall = render(&st, 80.0, 132, 60, Pal { on: false }, &mut cache);
        assert!(tall.contains("TLS") && tall.contains("SERVER") && tall.contains("FORMAT"));
        let short = render(&st, 80.0, 132, 30, Pal { on: false }, &mut cache);
        assert!(short.contains("FORMAT") && short.contains("NOW"));
    }
}
