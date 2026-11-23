//! `--live-json`: newline-delimited JSON state on stdout, for a GUI front end.
//!
//! Each line is one compact JSON object with a `type`: `start` once, `live`
//! every 250 ms (current meters, in-progress findings, new events), `report`
//! about once a second (the `--json` report so far), and `final` at the end.
//! `error` replaces all of them when the run can't start (ffmpeg missing).

use std::io::Write;

use serde_json::{json, Value};

use crate::findings::{findings, overall};
use crate::report::json_report;
use crate::state::*;
use crate::ui::{self, Pal};

pub fn start_line(url: &str) -> Value {
    json!({"type": "start", "version": crate::VERSION, "url": url})
}

/// Current values, as the terminal dashboard shows them, plus the events
/// logged since `seen_events` (a previous `events_total`).
pub fn live_line(st: &State, now: f64, seen_events: u64) -> Value {
    let fs = findings(st, now, false); // in-progress rules, as the dashboard
    let alive = if st.mode_direct { st.ffmpeg_alive } else { st.connected };
    let fresh = st.last_block.is_some_and(|t| now - t < 2.0);
    let cur = st.cur.filter(|_| fresh);
    let unseen = (st.events_total.saturating_sub(seen_events)).min(st.events.len() as u64) as usize;
    let events: Vec<Value> = st.events[st.events.len() - unseen..]
        .iter()
        .map(|e| {
            json!({"t": st.hms(e.t), "offset_s": round(e.t, 1), "level": e.level.as_str(),
                   "code": e.code, "message": e.msg})
        })
        .collect();
    let previous = st.titles.len().checked_sub(2).map(|i| st.titles[i].1.clone());
    json!({
        "type": "live",
        "elapsed_s": round(now, 1),
        "phase": if alive { "live" } else { "down" },
        "stream_url": st.final_url,
        "levels": cur.map(|c| json!({
            "rms_l": jnum(Some(c.rms_l), 1), "rms_r": jnum(Some(c.rms_r), 1),
            "peak_l": jnum(Some(c.pk_l), 1), "peak_r": jnum(Some(c.pk_r), 1),
        })),
        "loudness": {
            "momentary_lufs": jnum(st.lufs_m, 1),
            "short_term_lufs": jnum(st.lufs_s, 1),
            "integrated_lufs": jnum(st.lufs_i, 1),
            "lra_lu": jnum(st.lra, 1),
            "true_peak_dbtp": jnum(st.tpk, 1),
        },
        "stereo": {
            "state": cur.map(|c| c.class.code()),
            "correlation": cur.map(|c| round(c.corr, 3)),
            "side_db": cur.map_or(Value::Null, |c| jnum(Some(c.side_db), 1)),
        },
        "quality": {
            "bandwidth_hz": st.bandwidth.map(|b| b.round() as i64),
            "samples_over_0dbfs": st.overs_total,
            "samples_at_full_scale": st.clip_total,
            "max_peak_dbfs": jnum(Some(st.peak_max), 1),
            "decode_errors": st.decode_errors,
        },
        "delivery": {
            "realtime_ratio": st.realtime_ratio().map(|r| round(r, 3)),
            "input_kbps": (!st.mode_direct).then(|| round(st.net_rate(now) * 8.0 / 1000.0, 1)),
            "listener_buffer_s": round(st.buffer_sec, 1),
            "underruns": st.underruns,
            "stalls": st.stalls,
            "disconnects": st.disconnects,
        },
        "now_playing": {
            "title": st.title,
            "since_s": st.title.as_ref().map(|_| round(st.title_since, 1)),
            "previous": previous,
        },
        "result": overall(&fs),
        "findings": fs.iter().map(|f| json!({"level": f.level.as_str(), "code": f.code, "message": f.msg})).collect::<Vec<_>>(),
        "events": events,
        "events_total": st.events_total,
    })
}

pub fn report_line(st: &State, now: f64) -> Value {
    json!({"type": "report", "report": json_report(st, now, "in progress")})
}

pub fn final_line(st: &State, now: f64, stop_reason: &str, exit_code: i32) -> Value {
    let report = json_report(st, now, stop_reason);
    json!({
        "type": "final",
        "json_text": serde_json::to_string_pretty(&report).unwrap_or_default(),
        "report": report,
        "text": ui::text_report(st, now, Pal { on: false }, stop_reason),
        "exit_code": exit_code,
    })
}

pub fn error_line(code: &str, message: &str) -> Value {
    json!({"type": "error", "code": code, "message": message})
}

/// One compact line on stdout, flushed so the reader sees it at once.
pub fn emit(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", serde_json::to_string(v).unwrap_or_default());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> State {
        State {
            tpk_max: -200.0,
            peak_max: -200.0,
            final_url: "https://example.com/live".into(),
            url: "https://example.com/live".into(),
            ..Default::default()
        }
    }

    fn codes(v: &serde_json::Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|f| f["code"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn live_line_has_the_documented_keys() {
        let v = live_line(&base(), 5.0, 0);
        assert_eq!(v["type"], "live");
        for k in [
            "elapsed_s",
            "phase",
            "stream_url",
            "levels",
            "loudness",
            "stereo",
            "quality",
            "delivery",
            "now_playing",
            "result",
            "findings",
            "events",
            "events_total",
        ] {
            assert!(v.get(k).is_some(), "missing {k}");
        }
        for k in [
            "momentary_lufs",
            "short_term_lufs",
            "integrated_lufs",
            "lra_lu",
            "true_peak_dbtp",
        ] {
            assert!(v["loudness"].get(k).is_some(), "missing loudness.{k}");
        }
        for k in [
            "realtime_ratio",
            "input_kbps",
            "listener_buffer_s",
            "underruns",
            "stalls",
            "disconnects",
        ] {
            assert!(v["delivery"].get(k).is_some(), "missing delivery.{k}");
        }
    }

    #[test]
    fn live_findings_are_in_progress_findings() {
        let mut st = base();
        st.metaint = 16000;
        st.headers = vec![("icy-metaint".into(), "16000".into())];
        let live = live_line(&st, 5.0, 0);
        assert!(!codes(&live["findings"]).contains(&"no_title".to_string()));
        let rep = crate::report::json_report(&st, 5.0, "x");
        assert!(codes(&rep["findings"]).contains(&"no_title".to_string()));
    }

    #[test]
    fn live_events_are_incremental() {
        let mut st = base();
        for i in 0..3 {
            st.event(i as f64, Level::Info, "title", format!("e{i}"));
        }
        let v = live_line(&st, 1.0, 1);
        let evs = v["events"].as_array().unwrap();
        assert_eq!(evs.len(), 2);
        assert_eq!(evs[0]["message"], "e1");
        assert_eq!(evs[1]["message"], "e2");
        assert_eq!(evs[1]["offset_s"], 2.0);
        assert_eq!(v["events_total"], 3);
    }

    #[test]
    fn levels_null_when_audio_is_stale() {
        let mut st = base();
        st.cur = Some(Current {
            rms_l: -15.0,
            rms_r: -15.0,
            pk_l: -7.0,
            pk_r: -7.0,
            corr: 0.1,
            side_db: -1.0,
            class: Class::Stereo,
        });
        st.last_block = Some(0.0);
        assert!(live_line(&st, 5.0, 0)["levels"].is_null());
        let fresh = live_line(&st, 0.5, 0);
        assert_eq!(fresh["levels"]["rms_l"], -15.0);
        assert_eq!(fresh["stereo"]["state"], "stereo");
    }

    #[test]
    fn lines_are_single_line_json() {
        let mut st = base();
        st.title = Some("A\nB".into());
        st.event(0.0, Level::Info, "title", "line\nbreak");
        for v in [
            start_line("http://x/\n"),
            live_line(&st, 1.0, 0),
            report_line(&st, 1.0),
            final_line(&st, 1.0, "stopped by user", 0),
            error_line("ffmpeg_missing", "no\nffmpeg"),
        ] {
            let s = serde_json::to_string(&v).unwrap();
            assert!(!s.contains('\n'), "{s}");
            let back: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn final_line_carries_report_text_and_exit_code() {
        let v = final_line(&base(), 3.0, "stopped by user", 2);
        assert_eq!(v["type"], "final");
        assert_eq!(v["exit_code"], 2);
        assert!(!v["text"].as_str().unwrap().is_empty());
        let parsed: serde_json::Value = serde_json::from_str(v["json_text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed, v["report"]);
    }
}
