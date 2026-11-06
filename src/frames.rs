//! Frame-header analysis of the audio bytes as received (before ffmpeg).
//!
//! Recognises MPEG audio (MPEG-1/2/2.5 Layer I/II/III) and AAC in ADTS, and
//! reports what the encoder actually sends: version and layer, CBR or VBR,
//! channel mode, CRC. It also counts *sync losses*: places where the bytes
//! stop being valid frames. That's corruption, data injected by a relay or
//! proxy, or ICY metadata left inside the audio because the server's
//! `icy-metaint` is wrong. ID3 tags (which file-based autoDJs send between
//! tracks) are recognised and skipped, not counted as junk.
//!
//! Everything here runs on untrusted bytes: it must never panic, and its
//! buffer is bounded.

/// Container formats we recognise but don't analyse frame by frame.
const OGG: &[u8] = b"OggS";
const FLAC: &[u8] = b"fLaC";
/// Give up looking for a first sync after this many bytes (not MPEG/ADTS).
const GIVE_UP_BYTES: u64 = 256 * 1024;
/// Frames that must chain together before we trust a sync.
const CONFIRM_FRAMES: usize = 3;
const MAX_BUF: usize = 64 * 1024;
/// Larger "ID3 tags" are treated as noise (real ones with cover art are < 1 MB).
const MAX_ID3: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Mpeg,
    Adts,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Hdr {
    kind: Kind,
    len: usize,
    samples: u32,
    rate: u32,
    /// MPEG: 10 = 1, 20 = 2, 25 = 2.5. ADTS: 2 = MPEG-2, 4 = MPEG-4.
    version: u8,
    /// MPEG layer 1-3; ADTS audio object type (1 Main, 2 LC, 3 SSR, 4 LTP).
    layer: u8,
    kbps: u16, // MPEG only (frame header bitrate)
    mode: u8,  // MPEG channel mode 0..3 / ADTS channel configuration
    mode_ext: u8,
    crc: bool,
    emphasis: u8,
    copyright: bool,
    original: bool,
    adts_vbr: bool,
}

impl Hdr {
    /// Parameters that must stay the same from frame to frame.
    fn family(&self) -> (Kind, u8, u8, u32) {
        (self.kind, self.version, self.layer, self.rate)
    }
}

const MPEG_KBPS: [[u16; 15]; 5] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448], // V1 L1
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],    // V1 L2
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],     // V1 L3
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],    // V2/2.5 L1
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],         // V2/2.5 L2, L3
];
const ADTS_RATES: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350,
];

fn parse_mpeg(b: &[u8]) -> Option<Hdr> {
    if b.len() < 4 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = match (b[1] >> 3) & 3 {
        0 => 25,
        2 => 20,
        3 => 10,
        _ => return None,
    };
    let layer = match (b[1] >> 1) & 3 {
        1 => 3,
        2 => 2,
        3 => 1,
        _ => return None, // 0 is reserved (and is how ADTS looks)
    };
    let br_idx = (b[2] >> 4) as usize;
    let sr_idx = ((b[2] >> 2) & 3) as usize;
    if br_idx == 0 || br_idx == 15 || sr_idx == 3 || b[3] & 3 == 2 {
        return None; // free format, bad bitrate, reserved rate or emphasis
    }
    let table = match (version, layer) {
        (10, l) => (l - 1) as usize,
        (_, 1) => 3,
        _ => 4,
    };
    let kbps = MPEG_KBPS[table][br_idx];
    let rate = [44100, 48000, 32000][sr_idx]
        / if version == 10 {
            1
        } else if version == 20 {
            2
        } else {
            4
        };
    let pad = ((b[2] >> 1) & 1) as usize;
    let br = kbps as usize * 1000;
    let r = rate as usize;
    let (len, samples) = match layer {
        1 => ((12 * br / r + pad) * 4, 384),
        2 => (144 * br / r + pad, 1152),
        _ if version == 10 => (144 * br / r + pad, 1152),
        _ => (72 * br / r + pad, 576),
    };
    if len < 4 {
        return None;
    }
    Some(Hdr {
        kind: Kind::Mpeg,
        len,
        samples,
        rate,
        version,
        layer,
        kbps,
        mode: b[3] >> 6,
        mode_ext: (b[3] >> 4) & 3,
        crc: b[1] & 1 == 0,
        emphasis: b[3] & 3,
        copyright: (b[3] >> 3) & 1 == 1,
        original: (b[3] >> 2) & 1 == 1,
        adts_vbr: false,
    })
}

fn parse_adts(b: &[u8]) -> Option<Hdr> {
    if b.len() < 7 || b[0] != 0xFF || b[1] & 0xF6 != 0xF0 {
        return None;
    }
    let sr_idx = ((b[2] >> 2) & 0xF) as usize;
    let rate = *ADTS_RATES.get(sr_idx)?;
    let crc = b[1] & 1 == 0;
    let len = (((b[3] & 3) as usize) << 11) | ((b[4] as usize) << 3) | ((b[5] >> 5) as usize);
    if len < if crc { 9 } else { 7 } {
        return None;
    }
    let fullness = (((b[5] & 0x1F) as u16) << 6) | ((b[6] >> 2) as u16);
    Some(Hdr {
        kind: Kind::Adts,
        len,
        samples: 1024 * ((b[6] & 3) as u32 + 1),
        rate,
        version: if b[1] & 0x08 != 0 { 2 } else { 4 },
        layer: (b[2] >> 6) + 1,
        kbps: 0,
        mode: ((b[2] & 1) << 2) | (b[3] >> 6),
        mode_ext: 0,
        crc,
        emphasis: 0,
        copyright: false,
        original: false,
        adts_vbr: fullness == 0x7FF,
    })
}

fn parse(b: &[u8]) -> Option<Hdr> {
    parse_mpeg(b).or_else(|| parse_adts(b))
}

/// Size of an ID3v2 tag (header + body + optional footer) starting at `b`.
fn id3v2_len(b: &[u8]) -> Option<usize> {
    if b.len() < 10 || &b[..3] != b"ID3" || b[3] == 0xFF || b[4] == 0xFF || b[6..10].iter().any(|x| x & 0x80 != 0) {
        return None;
    }
    let size = b[6..10].iter().fold(0usize, |a, x| (a << 7) | *x as usize);
    let n = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
    (n <= MAX_ID3).then_some(n)
}

/// What the scanner has learned so far (cheap to copy into reports).
#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    pub kind: Option<Kind>,
    /// Recognised container we don't analyse ("Ogg", "FLAC"), or "unrecognised".
    pub other: Option<&'static str>,
    pub version: u8,
    pub layer: u8,
    pub rate: u32,
    pub frames: u64,
    pub samples: u64,
    pub bytes: u64,
    pub kbps_min: u16,
    pub kbps_max: u16,
    /// MPEG: frames per channel mode (stereo, joint, dual, mono).
    pub modes: [u64; 4],
    pub joint_ms_frames: u64,
    pub joint_is_frames: u64,
    /// ADTS channel configuration (last seen).
    pub adts_channels: u8,
    pub adts_vbr: bool,
    pub crc: bool,
    pub emphasis: u8,
    pub copyright: bool,
    pub original: bool,
    /// Lost sync after having it: non-frame bytes inside the stream.
    pub sync_losses: u32,
    pub junk_bytes: u64,
    /// Bytes skipped before the first sync of each connection (normal: we
    /// join mid-frame).
    pub lead_in_bytes: u64,
    pub id3_tags: u32,
    /// Times the stream re-synced with different parameters (version,
    /// layer, sample rate): an encoder restarted with other settings.
    pub format_changes: u32,
}

impl FrameStats {
    pub fn avg_kbps(&self) -> Option<f64> {
        (self.samples > 0 && self.rate > 0)
            .then(|| self.bytes as f64 * 8.0 / (self.samples as f64 / self.rate as f64) / 1000.0)
    }

    pub fn cbr(&self) -> Option<bool> {
        match self.kind? {
            Kind::Mpeg => Some(self.kbps_min == self.kbps_max),
            Kind::Adts => Some(!self.adts_vbr),
        }
    }

    pub fn codec(&self) -> Option<String> {
        Some(match self.kind? {
            Kind::Mpeg => format!(
                "MPEG-{} Layer {}",
                match self.version {
                    10 => "1",
                    20 => "2",
                    _ => "2.5",
                },
                ["?", "I", "II", "III"][self.layer.min(3) as usize]
            ),
            Kind::Adts => format!(
                "AAC {} (ADTS, MPEG-{})",
                ["?", "Main", "LC", "SSR", "LTP"][self.layer.min(4) as usize],
                self.version
            ),
        })
    }

    pub fn bitrate_text(&self) -> Option<String> {
        let avg = self.avg_kbps()?;
        Some(match (self.kind?, self.cbr()?) {
            (Kind::Mpeg, true) => format!("CBR {} kb/s", self.kbps_max),
            (Kind::Mpeg, false) => format!("VBR {}-{} kb/s (avg {avg:.0})", self.kbps_min, self.kbps_max),
            (Kind::Adts, true) => format!("~{avg:.0} kb/s"),
            (Kind::Adts, false) => format!("VBR ~{avg:.0} kb/s"),
        })
    }

    pub fn channel_text(&self) -> Option<String> {
        Some(match self.kind? {
            Kind::Mpeg => {
                let total: u64 = self.modes.iter().sum();
                let (i, n) = self.modes.iter().enumerate().max_by_key(|(_, n)| **n)?;
                let name = ["stereo", "joint stereo", "dual channel", "mono"][i];
                let mut s = name.to_string();
                if i == 1 && self.joint_ms_frames > 0 {
                    s += if self.joint_is_frames > 0 {
                        " (M/S + intensity)"
                    } else {
                        " (M/S)"
                    };
                }
                if *n < total {
                    s += &format!(", {:.0}% of frames", 100.0 * *n as f64 / total as f64);
                }
                s
            }
            Kind::Adts => match self.adts_channels {
                0 => "channels in stream config".into(),
                1 => "mono".into(),
                2 => "stereo".into(),
                n => format!("{n} channel config"),
            },
        })
    }

    /// One line for the dashboard / report, e.g.
    /// "MPEG-1 Layer III · CBR 128 kb/s · 44.1 kHz · joint stereo (M/S) · no CRC".
    pub fn summary(&self) -> Option<String> {
        if let Some(o) = self.other {
            return Some(match o {
                "unrecognised" => "not MPEG or ADTS frames (no frame analysis)".into(),
                c => format!("{c} container (no frame analysis)"),
            });
        }
        let mut v = vec![self.codec()?];
        v.extend(self.bitrate_text());
        v.push(format!("{} kHz", self.rate as f64 / 1000.0));
        v.extend(self.channel_text());
        v.push(if self.crc { "CRC".into() } else { "no CRC".into() });
        if self.kind == Some(Kind::Mpeg) && self.emphasis != 0 {
            v.push(if self.emphasis == 1 {
                "50/15 µs emphasis".into()
            } else {
                "CCITT emphasis".into()
            });
        }
        Some(v.join(" · "))
    }
}

/// Incremental scanner: feed it the demuxed audio bytes in any chunking.
#[derive(Default)]
pub struct FrameScan {
    buf: Vec<u8>,
    lock: Option<(Kind, u8, u8, u32)>,
    last_family: Option<(Kind, u8, u8, u32)>,
    /// This connection has produced frames: bytes skipped from now on are
    /// junk inside the stream, not the normal mid-frame lead-in.
    conn_had_frames: bool,
    /// Bytes searched without ever finding a sync (to give up on non-MPEG/ADTS).
    never_synced_bytes: u64,
    /// Rest of an ID3 tag still to skip (tags with cover art span many reads).
    skip_remaining: usize,
    pub stats: FrameStats,
}

impl FrameScan {
    /// A new connection starts mid-stream: drop the partial frame and expect
    /// lead-in bytes again, without counting a sync loss.
    pub fn new_connection(&mut self) {
        self.buf.clear();
        self.lock = None;
        self.conn_had_frames = false;
        self.skip_remaining = 0;
    }

    pub fn push(&mut self, data: &[u8]) {
        let mut data = data;
        if self.skip_remaining > 0 {
            let k = self.skip_remaining.min(data.len());
            self.skip_remaining -= k;
            data = &data[k..];
        }
        if self.stats.other.is_some() || data.is_empty() {
            return;
        }
        if self.last_family.is_none() && self.never_synced_bytes == 0 && self.buf.is_empty() {
            if data.starts_with(OGG) {
                self.stats.other = Some("Ogg");
                return;
            }
            if data.starts_with(FLAC) {
                self.stats.other = Some("FLAC");
                return;
            }
        }
        self.buf.extend_from_slice(data);
        let mut pos = 0;
        loop {
            let rest = &self.buf[pos..];
            if let Some(n) = id3v2_len(rest) {
                // ID3v2 (start of stream, or between tracks from an autoDJ)
                self.stats.id3_tags += 1;
                if rest.len() >= n {
                    pos += n;
                    continue;
                }
                self.skip_remaining = n - rest.len(); // the rest arrives in later reads
                pos = self.buf.len();
                break;
            }
            if let Some(fam) = self.lock {
                if rest.len() < 10 {
                    break; // a frame header needs 9 bytes, an ID3v2 header 10
                }
                match parse(rest) {
                    Some(h) if h.family() == fam => {
                        if rest.len() < h.len {
                            break; // wait for the whole frame
                        }
                        self.record(&h);
                        pos += h.len;
                    }
                    _ if &rest[..3] == b"TAG" => {
                        if rest.len() < 128 + 9 {
                            break; // wait for the whole tag and the next header
                        }
                        if parse(&rest[128..]).is_some_and(|h| h.family() == fam) {
                            self.stats.id3_tags += 1; // ID3v1 between tracks
                            pos += 128;
                        } else {
                            self.lose_sync();
                        }
                    }
                    _ => self.lose_sync(),
                }
                continue;
            }
            // Searching: need a header that chains into CONFIRM_FRAMES more.
            if rest.len() < 16 {
                break;
            }
            match confirm(rest) {
                Confirm::Yes(h) => {
                    if self.last_family.is_some_and(|f| f != h.family()) {
                        self.stats.format_changes += 1;
                    }
                    self.lock = Some(h.family());
                }
                Confirm::NeedMore => break,
                Confirm::No => {
                    if self.conn_had_frames {
                        self.stats.junk_bytes += 1;
                    } else {
                        self.stats.lead_in_bytes += 1;
                    }
                    pos += 1;
                    if self.last_family.is_none() {
                        self.never_synced_bytes += 1;
                        if self.never_synced_bytes > GIVE_UP_BYTES {
                            self.stats.other = Some("unrecognised");
                            self.buf.clear();
                            return;
                        }
                    }
                }
            }
        }
        self.buf.drain(..pos.min(self.buf.len()));
        if self.buf.len() > MAX_BUF {
            // can't happen with real frames (max 8 KB); bound memory anyway
            let cut = self.buf.len() - MAX_BUF / 2;
            self.stats.junk_bytes += cut as u64;
            self.buf.drain(..cut);
        }
    }

    fn lose_sync(&mut self) {
        self.stats.sync_losses += 1;
        self.lock = None;
    }

    fn record(&mut self, h: &Hdr) {
        self.conn_had_frames = true;
        self.last_family = Some(h.family());
        let s = &mut self.stats;
        if s.frames == 0 || s.kind != Some(h.kind) {
            s.kbps_min = h.kbps;
            s.kbps_max = h.kbps;
        }
        s.kind = Some(h.kind);
        s.version = h.version;
        s.layer = h.layer;
        s.rate = h.rate;
        s.frames += 1;
        s.samples += h.samples as u64;
        s.bytes += h.len as u64;
        s.crc = h.crc;
        match h.kind {
            Kind::Mpeg => {
                s.kbps_min = s.kbps_min.min(h.kbps);
                s.kbps_max = s.kbps_max.max(h.kbps);
                s.modes[h.mode as usize & 3] += 1;
                if h.mode == 1 && h.layer == 3 {
                    if h.mode_ext & 2 != 0 {
                        s.joint_ms_frames += 1;
                    }
                    if h.mode_ext & 1 != 0 {
                        s.joint_is_frames += 1;
                    }
                }
                s.emphasis = h.emphasis;
                s.copyright = h.copyright;
                s.original = h.original;
            }
            Kind::Adts => {
                s.adts_channels = h.mode;
                s.adts_vbr |= h.adts_vbr;
            }
        }
    }
}

/// Does a frame header at the start of `b` chain into CONFIRM_FRAMES frames
/// with the same parameters? (A lone 0xFF 0xFx is common in random data.)
fn confirm(b: &[u8]) -> Confirm {
    let Some(first) = parse(b) else { return Confirm::No };
    let mut off = 0;
    for _ in 0..CONFIRM_FRAMES {
        match b.get(off..).and_then(parse) {
            Some(h) if h.family() == first.family() => off += h.len,
            Some(_) => return Confirm::No,
            None if b.len() < off + 9 => return Confirm::NeedMore,
            None => return Confirm::No,
        }
    }
    Confirm::Yes(first)
}

enum Confirm {
    Yes(Hdr),
    NeedMore,
    No,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MPEG-1 Layer III, 128 kb/s, 44.1 kHz, joint stereo (M/S), no CRC.
    fn mp3_frame(pad: bool) -> Vec<u8> {
        let mut f = vec![0xFF, 0xFB, 0x90 | if pad { 2 } else { 0 }, 0x64];
        let len = 144 * 128_000 / 44_100 + pad as usize;
        f.resize(len, 0x55);
        f
    }

    /// AAC LC ADTS, 44.1 kHz, stereo, no CRC, VBR fullness, `len` bytes.
    fn adts_frame(len: usize) -> Vec<u8> {
        let mut f = vec![0xFF, 0xF1, 0x50, 0x80, 0, 0, 0xFC];
        f[3] |= ((len >> 11) & 3) as u8;
        f[4] = ((len >> 3) & 0xFF) as u8;
        f[5] = (((len & 7) << 5) as u8) | 0x1F;
        f.resize(len, 0x21);
        f
    }

    fn feed(s: &mut FrameScan, bytes: &[u8], chunk: usize) {
        for c in bytes.chunks(chunk.max(1)) {
            s.push(c);
        }
    }

    #[test]
    fn analyses_cbr_mp3_in_any_chunking() {
        let mut stream = vec![0x12, 0x34, 0x56]; // joined mid-frame
        for i in 0..200 {
            stream.extend(mp3_frame(i % 3 == 0));
        }
        for chunk in [1, 7, 417, 4096] {
            let mut s = FrameScan::default();
            feed(&mut s, &stream, chunk);
            let st = &s.stats;
            assert_eq!(st.kind, Some(Kind::Mpeg));
            assert!(st.frames >= 195, "chunk {chunk}: {} frames", st.frames);
            assert_eq!(
                (st.sync_losses, st.junk_bytes, st.lead_in_bytes),
                (0, 0, 3),
                "chunk {chunk}"
            );
            assert_eq!(st.cbr(), Some(true));
            assert!((st.avg_kbps().unwrap() - 128.0).abs() < 1.0);
            assert_eq!(
                st.summary().unwrap(),
                "MPEG-1 Layer III · CBR 128 kb/s · 44.1 kHz · joint stereo (M/S) · no CRC"
            );
        }
    }

    #[test]
    fn counts_injected_junk_once_and_skips_id3() {
        let mut stream = Vec::new();
        for _ in 0..50 {
            stream.extend(mp3_frame(false));
        }
        let junk = b"StreamTitle='leaked metadata';\0\0\0"; // metadata left in: wrong metaint
        stream.extend(junk);
        for _ in 0..50 {
            stream.extend(mp3_frame(false));
        }
        let mut tag = b"ID3\x04\x00\x00\x00\x00\x00\x10".to_vec(); // 16-byte body
        tag.resize(26, 0);
        stream.extend(tag);
        for _ in 0..50 {
            stream.extend(mp3_frame(false));
        }
        let mut s = FrameScan::default();
        feed(&mut s, &stream, 1000);
        assert_eq!(s.stats.sync_losses, 1);
        assert_eq!(s.stats.junk_bytes, junk.len() as u64);
        assert_eq!(s.stats.id3_tags, 1);
        assert!(s.stats.frames >= 145);
    }

    #[test]
    fn big_id3_tags_and_id3v1_are_skipped_in_any_chunking() {
        let mut stream = Vec::new();
        for _ in 0..30 {
            stream.extend(mp3_frame(false));
        }
        let body = 150_000usize; // cover art
        let mut tag = b"ID3\x03\x00\x00".to_vec();
        tag.extend((0..4).rev().map(|i| ((body >> (7 * i)) & 0x7F) as u8));
        tag.resize(10 + body, 0xFF); // 0xFF bytes look like sync words: must not matter
        stream.extend(tag);
        for _ in 0..30 {
            stream.extend(mp3_frame(false));
        }
        let mut v1 = b"TAG".to_vec();
        v1.resize(128, b' ');
        stream.extend(v1);
        for _ in 0..30 {
            stream.extend(mp3_frame(false));
        }
        for chunk in [1, 100, 4096, 65536] {
            let mut s = FrameScan::default();
            feed(&mut s, &stream, chunk);
            let st = &s.stats;
            assert_eq!((st.id3_tags, st.sync_losses, st.junk_bytes), (2, 0, 0), "chunk {chunk}");
            assert!(st.frames >= 85, "chunk {chunk}: {}", st.frames);
        }
    }

    #[test]
    fn analyses_adts_aac() {
        let mut stream = Vec::new();
        for i in 0..100 {
            stream.extend(adts_frame(300 + (i % 7) * 20));
        }
        let mut s = FrameScan::default();
        feed(&mut s, &stream, 333);
        let st = &s.stats;
        assert_eq!(st.kind, Some(Kind::Adts));
        assert_eq!((st.rate, st.adts_channels, st.sync_losses), (44100, 2, 0));
        assert!(st.adts_vbr);
        assert!(st.summary().unwrap().starts_with("AAC LC (ADTS, MPEG-4) · VBR ~"));
    }

    #[test]
    fn reconnect_lead_in_is_not_a_sync_loss() {
        let mut s = FrameScan::default();
        let mut a = vec![];
        for _ in 0..20 {
            a.extend(mp3_frame(false));
        }
        s.push(&a[..a.len() - 100]); // connection drops mid-frame
        s.new_connection();
        s.push(&[0xAA; 50]);
        s.push(&a);
        assert_eq!(s.stats.sync_losses, 0);
        assert_eq!(s.stats.junk_bytes, 0);
    }

    #[test]
    fn recognises_other_containers_and_gives_up_on_noise() {
        let mut s = FrameScan::default();
        s.push(b"OggS\0\x02rest-of-page");
        assert_eq!(s.stats.other, Some("Ogg"));
        let mut s = FrameScan::default();
        let mut x: u32 = 1;
        let noise: Vec<u8> = (0..400_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 3) as u8 & 0x7F // never 0xFF: no sync possible
            })
            .collect();
        feed(&mut s, &noise, 4096);
        assert_eq!(s.stats.other, Some("unrecognised"));
    }

    #[test]
    fn survives_random_bytes_without_panicking() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for round in 0..40 {
            let mut s = FrameScan::default();
            for _ in 0..200 {
                let n = (x % 3000) as usize;
                let chunk: Vec<u8> = (0..n)
                    .map(|_| {
                        x ^= x << 7;
                        x ^= x >> 9;
                        // bias towards 0xFF so headers actually get parsed
                        if x.is_multiple_of(5) {
                            0xFF
                        } else {
                            (x >> 11) as u8
                        }
                    })
                    .collect();
                s.push(&chunk);
                if round % 4 == 0 {
                    s.new_connection();
                }
            }
            assert!(s.buf.len() <= MAX_BUF);
            let _ = s.stats.summary();
        }
    }
}
