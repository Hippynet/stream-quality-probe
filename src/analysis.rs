//! Audio analysis of the decoded PCM: levels, stereo state, clipping,
//! EBU R128 loudness, bandwidth; plus the simulated listener buffer.

use std::io::Read;
use std::process::ChildStdout;
use std::sync::Arc;
use std::time::Duration;

use ebur128::{EbuR128, Mode};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::state::*;

struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    buf: Vec<Complex<f32>>,
    spec_sum: Vec<f64>,
    nblock: u64,
    r128: Option<EbuR128>,
    r128_ch: u32,
    left: Vec<f32>,
}

pub fn analyzer(sh: Arc<Shared>, mut pcm: ChildStdout) {
    let mut raw = vec![0u8; BLOCK_FRAMES * 2 * 4];
    let n = FFT_N as f32 - 1.0;
    // 4-term Blackman-Harris: ~92 dB sidelobes, so loud lows don't mask the cutoff
    let win = (0..FFT_N)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / n;
            0.35875 - 0.48829 * (2.0 * x).cos() + 0.14128 * (4.0 * x).cos() - 0.01168 * (6.0 * x).cos()
        })
        .collect();
    let mut a = Analyzer {
        fft: FftPlanner::new().plan_fft_forward(FFT_N),
        win,
        buf: vec![Complex::new(0.0, 0.0); FFT_N],
        spec_sum: vec![0.0; FFT_N / 2],
        nblock: 0,
        r128: None,
        r128_ch: 2,
        left: Vec::with_capacity(BLOCK_FRAMES),
    };
    let mut inter = vec![0f32; BLOCK_FRAMES * 2];
    while pcm.read_exact(&mut raw).is_ok() {
        for (o, c) in inter.iter_mut().zip(raw.as_chunks::<4>().0) {
            *o = f32::from_le_bytes(*c);
        }
        a.block(&sh, &inter);
    }
    sh.lock().ffmpeg_alive = false;
}

/// Highest frequency with meaningful energy in the averaged spectrum (Hz).
pub fn estimate_bandwidth(spec: &[f64]) -> Option<f64> {
    let n = spec.len();
    let hz = SR as f64 / 2.0 / n as f64;
    let db: Vec<f64> = spec.iter().map(|&p| to_db(p)).collect();
    let r = &db[(200.0 / hz) as usize..(5000.0 / hz) as usize];
    let reference = r.iter().cloned().fold(f64::MIN, f64::max);
    if reference < -150.0 {
        return None;
    }
    let thresh = reference - 70.0;
    let lo = (2000.0 / hz) as usize;
    for i in (lo + 1..=n - 5).rev() {
        if (0..4).all(|k| db[i - k] > thresh) {
            return Some((i + 1) as f64 * hz);
        }
    }
    Some(2000.0)
}

impl Analyzer {
    fn block(&mut self, sh: &Shared, inter: &[f32]) {
        let n = inter.len() / 2;
        let (mut el, mut er, mut elr, mut pkl, mut pkr, mut dcl, mut dcr) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        let (mut overs, mut clip) = (0u64, 0u64);
        for [l, r] in inter.as_chunks::<2>().0 {
            let (l, r) = (*l as f64, *r as f64);
            el += l * l;
            er += r * r;
            elr += l * r;
            pkl = pkl.max(l.abs());
            pkr = pkr.max(r.abs());
            dcl += l;
            dcr += r;
            for v in [l.abs(), r.abs()] {
                if v > 1.0 {
                    overs += 1;
                }
                if v >= 0.9995 {
                    clip += 1;
                }
            }
        }
        let nf = n as f64;
        let (el, er, elr) = (el / nf, er / nf, elr / nf);
        let (rms_l, rms_r) = (to_db(el), to_db(er));
        let rms = to_db((el + er) / 2.0);
        let mid = (el + er + 2.0 * elr) / 4.0;
        let side = (el + er - 2.0 * elr) / 4.0;
        let corr = if el > 1e-12 && er > 1e-12 {
            elr / (el * er).sqrt()
        } else {
            0.0
        };
        let side_db = if mid > 1e-14 { to_db(side) - to_db(mid) } else { 0.0 };
        let silent = rms_l < SILENCE_DB && rms_r < SILENCE_DB;

        // loudness meter: created once we know whether the source is mono
        if self.r128.is_none() {
            for _ in 0..25 {
                if sh.lock().in_line.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            self.r128_ch = if sh.lock().in_channels == Some(1) { 1 } else { 2 };
            self.r128 = EbuR128::new(
                self.r128_ch,
                SR,
                Mode::I | Mode::S | Mode::M | Mode::LRA | Mode::TRUE_PEAK | Mode::HISTOGRAM,
            )
            .ok();
        }
        let mono_stream = self.r128_ch == 1;
        let class = if silent {
            Class::Silence
        } else if mono_stream {
            Class::MonoStream
        } else if rms_l - rms_r > ONE_SIDED_DB {
            Class::LeftOnly
        } else if rms_r - rms_l > ONE_SIDED_DB {
            Class::RightOnly
        } else if corr < PHASE_CORR {
            Class::OutOfPhase
        } else if corr > MONO_CORR {
            Class::MonoContent
        } else {
            Class::Stereo
        };

        let (mut lm, mut ls, mut li, mut lra, mut tp, mut tpmax) = (None, None, None, None, None, None);
        if let Some(r) = self.r128.as_mut() {
            let ok = if mono_stream {
                self.left.clear();
                self.left.extend(inter.iter().step_by(2));
                r.add_frames_f32(&self.left)
            } else {
                r.add_frames_f32(inter)
            };
            if ok.is_ok() {
                lm = r.loudness_momentary().ok().filter(|v| *v > -100.0);
                ls = r.loudness_shortterm().ok().filter(|v| *v > -100.0);
                li = r.loudness_global().ok().filter(|v| *v > -70.0);
                lra = r.loudness_range().ok().filter(|v| v.is_finite());
                let peak = |f: &dyn Fn(u32) -> Option<f64>| {
                    (0..self.r128_ch)
                        .filter_map(f)
                        .fold(None, |a: Option<f64>, v| Some(a.map_or(v, |a| a.max(v))))
                };
                tp = peak(&|c| r.prev_true_peak(c).ok()).map(amp_db);
                tpmax = peak(&|c| r.true_peak(c).ok()).map(amp_db);
            }
        }

        // spectrum about once a second, on the mid signal
        self.nblock += 1;
        let mut new_bw = None;
        let mut spectra = None;
        if !silent && self.nblock.is_multiple_of(4) && n >= FFT_N {
            let off = (n - FFT_N) / 2;
            for i in 0..FFT_N {
                let m = (inter[2 * (off + i)] + inter[2 * (off + i) + 1]) * 0.5;
                self.buf[i] = Complex::new(m * self.win[i], 0.0);
            }
            self.fft.process(&mut self.buf);
            for (s, c) in self.spec_sum.iter_mut().zip(self.buf.iter()) {
                *s += (c.re as f64).powi(2) + (c.im as f64).powi(2);
            }
            let cnt = sh.lock().spec_n + 1;
            if cnt < 10 || cnt.is_multiple_of(5) {
                new_bw = estimate_bandwidth(&self.spec_sum);
            }
            spectra = Some(cnt);
        }

        let now = sh.now();
        let mut st = sh.lock();
        if st.first_audio.is_none() {
            st.first_audio = Some(now);
            st.event(
                now,
                Level::Ok,
                "audio_start",
                format!("Audio decoding started ({now:.1}s after launch)"),
            );
        }
        st.audio_sec += BLOCK_SEC;
        st.ffmpeg_alive = true;
        if now - st.first_audio.unwrap_or(now) > 8.0 {
            // ignore the server's connect burst
            let a = st.audio_sec;
            st.rt_window.push_back((now, a));
            while st.rt_window.front().is_some_and(|(t, _)| now - t > 15.0) {
                st.rt_window.pop_front();
            }
        }
        st.last_block = Some(now);
        st.cur = Some(Current {
            rms_l,
            rms_r,
            pk_l: amp_db(pkl),
            pk_r: amp_db(pkr),
            corr,
            side_db,
            class,
        });
        st.lufs_m = lm;
        st.lufs_s = ls;
        st.lufs_i = li;
        st.lra = lra;
        st.tpk = tp;
        if let Some(t) = tpmax {
            st.tpk_max = st.tpk_max.max(t);
        }
        {
            let b = st.bucket(now);
            b.blocks += 1;
            b.rms_sum += rms;
            b.rms_n += 1;
            b.classes[class.idx()] += 1;
            b.clip += clip;
            if silent {
                b.silent_blocks += 1;
            } else if let Some(s) = ls {
                b.lufs_sum += s;
                b.lufs_n += 1;
            }
        }
        st.class_sec[class.idx()] += BLOCK_SEC;
        if !silent {
            st.rms_sum += (el + er) / 2.0;
            st.rms_n += 1;
        }
        st.peak_max = st.peak_max.max(amp_db(pkl.max(pkr)));
        st.clip_total += clip;
        st.overs_total += overs;
        st.dc[0] += dcl / nf;
        st.dc[1] += dcr / nf;
        st.dc_n += 1;
        if let Some(c) = spectra {
            st.spec_n = c;
        }
        if new_bw.is_some() {
            st.bandwidth = new_bw;
        }
        track_silence(&mut st, silent, now);
        track_class(&mut st, class, now);
    }
}

fn track_silence(st: &mut State, silent: bool, now: f64) {
    if silent {
        if st.silence_start.is_none() {
            st.silence_start = Some(now - BLOCK_SEC);
        }
        st.silence_run += BLOCK_SEC;
        st.silence_total += BLOCK_SEC;
        if st.silence_run >= DEAD_AIR_SEC && !st.silence_reported {
            st.silence_reported = true;
            st.silence_events += 1;
            let t = st.silence_start.unwrap_or(now);
            st.event(
                t,
                Level::Bad,
                "dead_air",
                format!("Dead air: silence (< {SILENCE_DB:.0} dBFS)"),
            );
        }
    } else {
        if st.silence_reported {
            let run = st.silence_run;
            st.event(
                now,
                Level::Ok,
                "dead_air_end",
                format!("Audio back after {run:.1}s of silence"),
            );
            st.silence_longest = st.silence_longest.max(run);
        }
        st.silence_run = 0.0;
        st.silence_start = None;
        st.silence_reported = false;
    }
}

/// Log channel conditions (mono / phase / dead channel) lasting PERSIST_SEC.
fn track_class(st: &mut State, class: Class, now: f64) {
    if class == Class::Silence {
        return;
    }
    if st.class_run != Some(class) {
        if let (true, Some(prev)) = (st.class_reported, st.class_run) {
            let name = prev.code().replace('_', " ");
            let mut name_c = name.clone();
            name_c[..1].make_ascii_uppercase();
            let len = st.class_run_len;
            st.event(
                now,
                Level::Ok,
                &format!("{}_end", prev.code()),
                format!(
                    "{name_c} ended after {len:.0}s (now {})",
                    class.code().replace('_', " ")
                ),
            );
        }
        st.class_run = Some(class);
        st.class_run_start = now;
        st.class_run_len = 0.0;
        st.class_reported = false;
    }
    st.class_run_len += BLOCK_SEC;
    if let Some((lvl, msg)) = class.alert() {
        if st.class_run_len >= PERSIST_SEC && !st.class_reported {
            st.class_reported = true;
            let t = st.class_run_start;
            st.event(t, lvl, class.code(), msg);
        }
    }
}

/// Periodic housekeeping: simulated listener buffer + disconnect marking.
/// The virtual player prebuffers PLAYER_PREBUFFER seconds, then plays in
/// realtime; if it runs dry, that's an audible rebuffer for a real listener.
pub fn tick(sh: &Shared) {
    let now = sh.now();
    let mut st = sh.lock();
    if !st.mode_direct && !st.connected && st.first_audio.is_some() {
        st.bucket(now).disconnected = true;
    }
    if st.first_audio.is_none() {
        return;
    }
    if !st.playing {
        st.buffer_sec = st.audio_sec - st.play_base;
        if st.buffer_sec >= PLAYER_PREBUFFER {
            st.playing = true;
            st.play_t0 = now;
        }
        return;
    }
    let buf = st.audio_sec - (st.play_base + (now - st.play_t0));
    st.buffer_sec = buf;
    st.buffer_min = Some(st.buffer_min.map_or(buf, |m| m.min(buf)));
    if buf < 0.0 {
        st.underruns += 1;
        st.bucket(now).underrun = true;
        st.event(
            now,
            Level::Bad,
            "underrun",
            "Listener buffer ran dry - a player would stop and rebuffer here",
        );
        st.playing = false;
        st.play_base = st.audio_sec;
        st.buffer_sec = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A falling spectrum with a brick-wall low-pass, like a lossy encoder's.
    fn spectrum_with_cutoff(cut_hz: f64) -> Vec<f64> {
        let n = FFT_N / 2;
        let hz = SR as f64 / 2.0 / n as f64;
        (0..n)
            .map(|i| {
                if i as f64 * hz < cut_hz {
                    1.0 / (1.0 + i as f64 / 50.0)
                } else {
                    1e-14
                }
            })
            .collect()
    }

    #[test]
    fn finds_the_lowpass_cutoff() {
        for cut in [4000.0, 11000.0, 16000.0, 19500.0] {
            let bw = estimate_bandwidth(&spectrum_with_cutoff(cut)).unwrap();
            assert!((bw - cut).abs() < 50.0, "cutoff {cut} estimated as {bw}");
        }
    }

    #[test]
    fn silence_has_no_bandwidth() {
        assert_eq!(estimate_bandwidth(&vec![0.0; FFT_N / 2]), None);
    }
}
