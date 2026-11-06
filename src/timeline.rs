//! Whole-session timeline: per-second buckets aggregated into columns.

use crate::state::*;

pub const ZOOMS: [usize; 13] = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

pub fn pick_zoom(elapsed: usize, cols: usize) -> usize {
    *ZOOMS
        .iter()
        .find(|z| elapsed.div_ceil(**z) <= cols)
        .unwrap_or(&ZOOMS[ZOOMS.len() - 1])
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Ok,
    Stall,
    Underrun,
    Disconnect,
    DecodeError,
}

impl Delivery {
    pub fn code(self) -> &'static str {
        match self {
            Delivery::Ok => "ok",
            Delivery::Stall => "stall",
            Delivery::Underrun => "underrun",
            Delivery::Disconnect => "disconnect",
            Delivery::DecodeError => "decode_error",
        }
    }
}

pub struct Column {
    pub blocks: u32,
    pub silent: bool,
    pub silent_s: f64,
    pub lufs: Option<f64>,
    pub rms: Option<f64>,
    pub stereo: Option<Class>,
    pub clip: u64,
    pub delivery: Option<Delivery>,
    pub decode_errors: u32,
    pub title_change: bool,
}

/// Aggregate wall-clock seconds [c0, c0+zoom) into one timeline point.
pub fn column(st: &State, c0: usize, zoom: usize) -> Column {
    let end = (c0 + zoom).min(st.buckets.len());
    let bs: &[Bucket] = if c0 < end { &st.buckets[c0..end] } else { &[] };
    let blocks: u32 = bs.iter().map(|b| b.blocks).sum();
    let nsil: u32 = bs.iter().map(|b| b.silent_blocks).sum();
    let (ls, ln) = bs.iter().fold((0.0, 0), |a, b| (a.0 + b.lufs_sum, a.1 + b.lufs_n));
    let (rs, rn) = bs.iter().fold((0.0, 0), |a, b| (a.0 + b.rms_sum, a.1 + b.rms_n));
    let mut cls = [0u32; 7];
    for b in bs {
        for (i, v) in b.classes.iter().enumerate() {
            cls[i] += v;
        }
    }
    let audible: u32 = CLASSES
        .iter()
        .filter(|c| **c != Class::Silence)
        .map(|c| cls[c.idx()])
        .sum();
    let stereo = if audible > 0 {
        let worst = [Class::LeftOnly, Class::RightOnly, Class::OutOfPhase]
            .into_iter()
            .find(|c| cls[c.idx()] >= (audible / 4).max(1));
        worst.or_else(|| {
            CLASSES
                .iter()
                .filter(|c| **c != Class::Silence)
                .max_by_key(|c| cls[c.idx()])
                .copied()
        })
    } else if cls[Class::Silence.idx()] > 0 {
        Some(Class::Silence)
    } else {
        None
    };
    let started = st.first_audio.is_some_and(|fa| (c0 + zoom) as f64 > fa);
    let delivery = if bs.iter().any(|b| b.disconnected) {
        Some(Delivery::Disconnect)
    } else if bs.iter().any(|b| b.underrun) {
        Some(Delivery::Underrun)
    } else if bs.iter().any(|b| b.stall) {
        Some(Delivery::Stall)
    } else if bs.iter().any(|b| b.decode_err > 0) {
        Some(Delivery::DecodeError)
    } else if blocks > 0 || bs.iter().any(|b| b.net_bytes > 0) || started {
        Some(Delivery::Ok)
    } else {
        None
    };
    let silent_s = nsil as f64 * BLOCK_SEC;
    Column {
        blocks,
        silent: blocks > 0 && silent_s >= (zoom as f64 * 0.5).min(DEAD_AIR_SEC),
        silent_s,
        lufs: (ln > 0).then(|| ls / ln as f64),
        rms: (rn > 0).then(|| rs / rn as f64),
        stereo,
        clip: bs.iter().map(|b| b.clip).sum(),
        delivery,
        decode_errors: bs.iter().map(|b| b.decode_err).sum(),
        title_change: bs.iter().any(|b| b.title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_fits_the_whole_session_in_the_columns() {
        assert_eq!(pick_zoom(1, 100), 1);
        assert_eq!(pick_zoom(100, 100), 1);
        assert_eq!(pick_zoom(101, 100), 2);
        assert_eq!(pick_zoom(3600, 120), 30);
        assert_eq!(pick_zoom(10_000_000, 100), 3600);
    }
}
