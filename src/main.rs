//! streamprobe - live diagnostics for internet radio streams.
//!
//!   streamprobe <URL>                 live dashboard; q / Ctrl-C stops and prints a report
//!   streamprobe <URL> -t 120          run for 120 s, then report
//!   streamprobe <URL> -t 60 --json    machine-readable report on stdout (for scripts)
//!   streamprobe <URL> -o report.json  also save the report (.json = JSON, else text)
//!
//! Exit codes: 0 healthy, 1 warnings, 2 problems found, 3 could not connect / no audio.

#![allow(clippy::regex_creation_in_loops)] // regex!() compiles once (OnceLock)

/// Lazily-compiled static regex.
macro_rules! regex {
    ($re:literal) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($re).expect("valid regex"))
    }};
}

mod analysis;
mod decoder;
mod findings;
mod frames;
mod net;
mod report;
mod state;
mod timeline;
mod ui;

use std::collections::VecDeque;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::{cursor, execute, terminal};
use url::Url;

use state::*;
use ui::Pal;

/// Release builds get the four-part tag (e.g. 0.1.0.0) from CI via
/// STREAMPROBE_VERSION; local builds report the Cargo version.
pub const VERSION: &str = match option_env!("STREAMPROBE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(
    name = "streamprobe",
    version = VERSION,
    about = "Live diagnostics for an internet radio stream - just give it the URL.",
    after_help = "Exit codes: 0 healthy, 1 warnings, 2 problems found, 3 could not connect / no audio.\n\
                  Requires ffmpeg on PATH (macOS: brew install ffmpeg)."
)]
struct Args {
    /// Stream URL (Icecast/Shoutcast mount, .pls/.m3u playlist, HLS .m3u8)
    url: String,
    /// Stop after SEC seconds [default: until q/Ctrl-C in a terminal, otherwise 60]
    #[arg(short = 't', long = "time", value_name = "SEC")]
    time: Option<f64>,
    /// Save the report to FILE (.json = JSON, else text); rewritten every 30s during the run
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Print the final report as JSON on stdout (progress goes to stderr)
    #[arg(long)]
    json: bool,
    /// No live dashboard: print events and periodic status lines instead
    #[arg(long)]
    no_ui: bool,
    /// Continue even if the TLS certificate is invalid (the problem is still reported)
    #[arg(long)]
    insecure: bool,
    /// Allow connecting to private, loopback and link-local addresses (an
    /// internal server by hostname). Implied when the URL is a literal internal IP or localhost.
    #[arg(long)]
    allow_internal: bool,
}

/// ffmpeg exited: log it, and if it never produced audio, that's `no_audio`.
fn decoder_exited(sh: &Shared, now: f64, last_line: &str) {
    let mut st = sh.lock();
    st.event(
        now,
        Level::Bad,
        "ffmpeg_exit",
        format!("ffmpeg stopped decoding the stream: {last_line}"),
    );
    if st.first_audio.is_none() {
        st.no_audio = true;
    }
}

fn exit_code(st: &State, now: f64) -> i32 {
    if st.fatal.is_some() || st.no_audio || st.first_audio.is_none() {
        return 3; // couldn't connect, or nothing decodable arrived
    }
    match findings::overall(&findings::findings(st, now, true)) {
        "problems" => 2,
        "warnings" => 1,
        _ => 0,
    }
}

fn save_report(sh: &Shared, path: &Path, stop_reason: &str) {
    let now = sh.now();
    let body = {
        let st = sh.lock();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            serde_json::to_string_pretty(&report::json_report(&st, now, stop_reason)).unwrap_or_default()
        } else {
            ui::text_report(&st, now, Pal { on: false }, stop_reason)
        }
    };
    // A sibling temp file, then rename, so readers never see half a report.
    // Created exclusively (O_CREAT|O_EXCL) under an unpredictable name: in a
    // shared directory nobody can plant a symlink for us to write through.
    let base = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let mut written = None;
    for attempt in 0..16u32 {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
            ^ std::process::id().rotate_left(13)
            ^ attempt.wrapping_mul(0x9E37_79B9);
        let tmp = path.with_file_name(format!(".{base}.{nonce:08x}.tmp"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(mut f) => {
                if f.write_all(body.as_bytes()).and_then(|_| f.write_all(b"\n")).is_ok() {
                    written = Some(tmp);
                } else {
                    let _ = std::fs::remove_file(&tmp);
                }
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => break,
        }
    }
    let ok = written.is_some_and(|tmp| {
        std::fs::rename(&tmp, path).is_ok() || {
            let _ = std::fs::remove_file(&tmp);
            false
        }
    });
    if !ok {
        eprintln!("streamprobe: could not write {}", path.display());
    }
}

fn finish(sh: &Shared, args: &Args, stop_reason: &str, out_pal: Pal) -> ! {
    sh.set_stop();
    sh.kill_ffprobe(); // its result is no longer needed; never leave it running
    if let Some(p) = &args.output {
        save_report(sh, p, stop_reason);
    }
    let now = sh.now();
    let st = sh.lock();
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report::json_report(&st, now, stop_reason)).unwrap_or_default()
        );
    } else {
        println!("{}", ui::text_report(&st, now, out_pal, stop_reason));
    }
    if let Some(p) = &args.output {
        eprintln!("Report saved to {}", p.display());
    }
    let code = exit_code(&st, now);
    drop(st);
    let _ = std::io::stdout().flush();
    exit(code)
}

/// Terminal size, falling back to $COLUMNS/$LINES or 100x40 when the
/// terminal reports 0x0 (e.g. some pseudo-terminals).
fn term_size() -> (usize, usize) {
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(d)
    };
    match terminal::size() {
        Ok((w, h)) if w > 0 && h > 0 => (w as usize, h as usize),
        _ => (env("COLUMNS", 100), env("LINES", 40)),
    }
}

fn restore_terminal() {
    let mut out = std::io::stdout();
    let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

fn main() {
    let args = Args::parse();
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let stdout_tty = std::io::stdout().is_terminal();
    let ui_mode = stdout_tty && std::io::stdin().is_terminal() && !args.no_ui && !args.json;
    let out_pal = Pal {
        on: stdout_tty && !no_color,
    };
    // progress goes to stderr in --json mode so stdout stays pure JSON
    let log_pal = Pal {
        on: !no_color
            && if args.json {
                std::io::stderr().is_terminal()
            } else {
                stdout_tty
            },
    };
    let log = |s: String| {
        if args.json {
            eprintln!("{s}");
        } else {
            println!("{s}");
        }
    };
    let limit = args.time.or(if ui_mode { None } else { Some(DEFAULT_NONTTY_SEC) });

    let Some(ffmpeg) = decoder::find_tool("ffmpeg") else {
        eprintln!("streamprobe: ffmpeg not found. Install it first:\n  macOS:  brew install ffmpeg\n  Debian/Ubuntu:  sudo apt install ffmpeg\n  RHEL/Alma/Rocky:  enable EPEL and RPM Fusion (rpmfusion.org/Configuration), then sudo dnf install ffmpeg");
        exit(3);
    };
    let ffprobe = decoder::find_tool("ffprobe");

    let mut url_s = args.url.trim().to_string();
    if !url_s.contains("://") {
        url_s = format!("http://{url_s}");
    }
    let sh = Arc::new(Shared::new(&url_s, args.insecure, args.allow_internal));
    {
        let sh = sh.clone();
        let _ = ctrlc::set_handler(move || {
            // While connecting (DNS, slow servers, redirects) nothing polls the
            // stop flag, and a second Ctrl-C means "now": exit straight away.
            if !sh.streaming.load(std::sync::atomic::Ordering::Relaxed) || sh.stopped() {
                if sh.ui_active.load(std::sync::atomic::Ordering::Relaxed) {
                    restore_terminal();
                }
                sh.kill_ffprobe();
                eprintln!("\nstreamprobe: interrupted");
                exit(130);
            }
            sh.set_stop();
        });
    }

    log(format!(
        "{}streamprobe{} connecting to {url_s} …{}",
        log_pal.cyan(),
        log_pal.reset(),
        limit.map_or(String::new(), |l| format!(" (running {l:.0}s)"))
    ));
    let first = match Url::parse(&url_s) {
        Err(e) => Err(net::NetError {
            code: "bad_url".into(),
            msg: format!("invalid URL: {e}"),
        }),
        Ok(u) => net::preflight(&sh, &u),
    };
    let first = match first {
        Ok(f) => f,
        Err(e) => {
            let now = sh.now();
            {
                let mut st = sh.lock();
                let msg = format!("Cannot open stream: {}", e.msg);
                st.event(now, Level::Bad, &e.code, msg.clone());
                st.fatal = Some((e.code, msg));
            }
            finish(&sh, &args, "could not connect", out_pal);
        }
    };

    let final_url = sh.lock().final_url.clone();
    let direct = first.is_none();
    if let Some(fp) = ffprobe {
        let (sh2, u) = (sh.clone(), direct.then(|| final_url.clone()));
        std::thread::spawn(move || decoder::run_ffprobe(sh2, fp, u));
    } else {
        sh.lock().ffprobe_err = Some("ffprobe not found".into());
    }
    let mut ff = match decoder::spawn_ffmpeg(&ffmpeg, direct.then_some(final_url.as_str())) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("streamprobe: could not start ffmpeg: {e}");
            exit(3);
        }
    };
    if let Some(first) = first {
        let stdin = ff.stdin.take().expect("ffmpeg stdin");
        let sh2 = sh.clone();
        std::thread::spawn(move || net::net_reader(sh2, stdin, first));
    }
    {
        let now = sh.now();
        let mut st = sh.lock();
        let note = if direct { " (HLS, read by ffmpeg)" } else { "" };
        st.event(now, Level::Ok, "connected", format!("Connected to {final_url}{note}"));
        st.last_connect = now;
    }
    let stdout = ff.stdout.take().expect("ffmpeg stdout");
    let stderr = ff.stderr.take().expect("ffmpeg stderr");
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    {
        let sh2 = sh.clone();
        std::thread::spawn(move || analysis::analyzer(sh2, stdout));
        let (sh3, t3) = (sh.clone(), tail.clone());
        std::thread::spawn(move || decoder::stderr_reader(sh3, stderr, t3));
    }
    let last_ff_line = || {
        tail.lock()
            .ok()
            .and_then(|t| t.back().cloned())
            .unwrap_or_else(|| "unknown reason".into())
    };

    if ui_mode {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |i| {
            restore_terminal();
            prev(i)
        }));
        let _ = terminal::enable_raw_mode();
        sh.ui_active.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = execute!(std::io::stdout(), terminal::EnterAlternateScreen, cursor::Hide);
    }

    let mut stop_reason = "stopped by user".to_string();
    let (mut printed, mut last_status, mut last_save) = (0u64, 0.0, 0.0);
    let mut cache = ui::TimelineCache::new();
    let ui_pal = Pal { on: !no_color };
    sh.streaming.store(true, std::sync::atomic::Ordering::Relaxed);
    while !sh.stopped() {
        analysis::tick(&sh);
        let now = sh.now();
        if let Ok(Some(_)) = ff.try_wait() {
            decoder_exited(&sh, now, &last_ff_line());
            stop_reason = "decoder stopped / stream ended".into();
            break;
        }
        if sh.lock().first_audio.is_none() && now > 30.0 {
            let msg = format!("No audio decoded within 30s (ffmpeg: {})", last_ff_line());
            let mut st = sh.lock();
            st.no_audio = true;
            st.event(now, Level::Bad, "no_audio", msg);
            stop_reason = "no audio".into();
            break;
        }
        if limit.is_some_and(|l| now >= l) {
            stop_reason = format!("time limit {:.0}s reached", limit.unwrap_or(0.0));
            break;
        }
        if let Some(p) = &args.output {
            if now - last_save > 30.0 {
                save_report(&sh, p, "in progress");
                last_save = now;
            }
        }
        if ui_mode {
            let (w, h) = term_size();
            let frame = ui::render(&sh.lock(), now, w, h, ui_pal, &mut cache);
            let mut out = std::io::stdout();
            let _ = write!(out, "\x1b[H{frame}\x1b[J");
            let _ = out.flush();
            if event::poll(Duration::from_millis(250)).unwrap_or(false) {
                if let Ok(TermEvent::Key(k)) = event::read() {
                    let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
                    if k.kind == KeyEventKind::Press
                        && (ctrl_c || matches!(k.code, KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc))
                    {
                        break;
                    }
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(500));
            let lines: Vec<String> = {
                let st = sh.lock();
                // events_total keeps counting when old events are dropped (MAX_EVENTS)
                let unseen = (st.events_total - printed).min(st.events.len() as u64) as usize;
                let new: Vec<String> = st.events[st.events.len() - unseen..]
                    .iter()
                    .map(|e| format!("{} {} {}", st.hms(e.t), log_pal.icon(e.level), e.msg))
                    .collect();
                printed = st.events_total;
                if now - last_status >= 10.0 {
                    last_status = now;
                    let mut v = new;
                    v.push(format!(
                        "{}{}{}",
                        log_pal.dim(),
                        ui::status_line(&st, now),
                        log_pal.reset()
                    ));
                    v
                } else {
                    new
                }
            };
            for l in lines {
                log(l);
            }
        }
    }
    // the reader thread stops everything when ffmpeg stops reading: that's the
    // decoder dying (garbage input, unsupported codec), not the user quitting
    if sh.lock().decoder_gone && stop_reason == "stopped by user" {
        decoder_exited(&sh, sh.now(), &last_ff_line());
        stop_reason = "decoder stopped / stream ended".into();
    }
    sh.set_stop();
    if ui_mode {
        restore_terminal();
    }
    let _ = ff.kill();
    let _ = ff.wait();
    if !ui_mode {
        log(String::new());
    }
    finish(&sh, &args, &stop_reason, out_pal);
}
