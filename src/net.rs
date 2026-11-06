//! Minimal HTTP/ICY client (handles "ICY 200 OK", redirects, TLS, playlists)
//! and the network reader thread that feeds ffmpeg.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::process::ChildStdin;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme, StreamOwned};
use url::{Host, Position, Url};

use crate::state::*;

pub const USER_AGENT: &str = concat!(
    "streamprobe/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Hippynet/stream-quality-probe)"
);

pub enum Conn {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Conn {
    fn tcp(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => &s.sock,
        }
    }
    pub fn set_read_timeout(&self, d: Duration) {
        let _ = self.tcp().set_read_timeout(Some(d));
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

#[derive(Debug)]
pub struct NetError {
    pub code: String,
    pub msg: String,
}

fn err(code: &str, msg: impl Into<String>) -> NetError {
    NetError {
        code: code.to_string(),
        msg: msg.into(),
    }
}

pub struct Opened {
    pub conn: Conn,
    pub headers: Vec<(String, String)>,
    pub leftover: Vec<u8>,
    pub url: Url,
}

/// Side information gathered while opening (kept even when opening fails).
#[derive(Default)]
pub struct OpenLog {
    pub redirects: Vec<(u16, String)>,
    pub tls_error: Option<String>,
    pub info: Option<ConnInfo>,
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------
fn provider() -> Arc<CryptoProvider> {
    static P: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    P.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

fn tls_config(insecure: bool) -> Arc<ClientConfig> {
    static SECURE: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    static INSECURE: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let builder = || {
        ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("tls versions")
    };
    if insecure {
        INSECURE
            .get_or_init(|| {
                Arc::new(
                    builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(NoVerify))
                        .with_no_client_auth(),
                )
            })
            .clone()
    } else {
        SECURE
            .get_or_init(|| {
                let mut roots = RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                Arc::new(builder().with_root_certificates(roots).with_no_client_auth())
            })
            .clone()
    }
}

/// Only used with --insecure, after the certificate problem has been recorded.
#[derive(Debug)]
struct NoVerify;

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider().signature_verification_algorithms.supported_schemes()
    }
}

fn tls_handshake(mut sock: TcpStream, host: &str, insecure: bool) -> io::Result<(ClientConnection, TcpStream)> {
    let name = ServerName::try_from(host.to_string()).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut tls = ClientConnection::new(tls_config(insecure), name).map_err(io::Error::other)?;
    let deadline = Instant::now() + READ_DEADLINE;
    while tls.is_handshaking() {
        if Instant::now() > deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "TLS handshake too slow"));
        }
        tls.complete_io(&mut sock)?;
    }
    Ok((tls, sock))
}

/// Human description of a certificate verification failure, if that's what `e` is.
fn cert_error(e: &io::Error) -> Option<String> {
    let inner = e.get_ref()?.downcast_ref::<rustls::Error>()?;
    match inner {
        rustls::Error::InvalidCertificate(ce) => {
            let d = format!("{ce:?}");
            let msg = if d.starts_with("Expired") {
                "certificate has expired".to_string()
            } else if d.starts_with("NotValidYet") {
                "certificate is not valid yet (check server clock)".to_string()
            } else if d.starts_with("NotValidForName") {
                "certificate does not match the hostname".to_string()
            } else if d.starts_with("UnknownIssuer") {
                "certificate issuer not trusted (self-signed, or missing intermediate certificate)".to_string()
            } else if d.starts_with("Revoked") {
                "certificate has been revoked".to_string()
            } else {
                format!("invalid certificate ({d})")
            };
            Some(msg)
        }
        _ => None,
    }
}

/// (days left, expiry date, issuer) of the server's leaf certificate.
fn cert_info(der: &[u8]) -> Option<(f64, String, Option<String>)> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).ok()?;
    let not_after = cert.validity().not_after.timestamp();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs_f64();
    let expires = chrono::DateTime::from_timestamp(not_after, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let issuer = cert.issuer();
    let name = issuer
        .iter_common_name()
        .next()
        .or_else(|| issuer.iter_organization().next())
        .and_then(|a| a.as_str().ok())
        .map(|s| cap(s, 80));
    Some((round((not_after as f64 - now) / 86400.0, 1), expires, name))
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------
fn ms(t: Instant) -> f64 {
    round(t.elapsed().as_secs_f64() * 1000.0, 1)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn read_headers(conn: &mut Conn) -> Result<(Vec<u8>, Vec<u8>), NetError> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let deadline = Instant::now() + READ_DEADLINE; // a server dripping bytes can't hold us forever
    loop {
        if Instant::now() > deadline {
            return Err(err(
                "no_response",
                format!("server took over {}s to send response headers", READ_DEADLINE.as_secs()),
            ));
        }
        if let Some(p) = find(&buf, b"\r\n\r\n") {
            return Ok((buf[..p].to_vec(), buf[p + 4..].to_vec()));
        }
        if let Some(p) = find(&buf, b"\n\n") {
            return Ok((buf[..p].to_vec(), buf[p + 2..].to_vec()));
        }
        if buf.len() > 65536 {
            break;
        }
        match conn.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(err("no_response", format!("no response headers from server ({e})"))),
        }
    }
    Err(err(
        "no_response",
        "server closed the connection without a valid response (it may be blocking this client)",
    ))
}

fn strip_tags(s: &str) -> String {
    let t = regex!(r"<[^>]+>").replace_all(s, " ");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Connect and read the response headers, following redirects.
/// Anything that isn't a globally routable unicast address: loopback,
/// RFC 1918, link-local (incl. cloud metadata 169.254.169.254), CGNAT,
/// benchmarking, documentation, multicast, reserved, unique/site-local.
/// IPv6 forms that embed an IPv4 address (mapped, compatible, NAT64, 6to4)
/// are judged by that IPv4 address. A hand-rolled `is_global()`, which is
/// still unstable in std.
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            o[0] == 0                                     // 0.0.0.0/8 "this network"
                || o[0] == 10                             // 10/8
                || o[0] == 127                            // loopback
                || (o[0] == 100 && (o[1] & 0xc0) == 64)   // 100.64/10 CGNAT
                || (o[0] == 169 && o[1] == 254)           // link-local, cloud metadata
                || (o[0] == 172 && (o[1] & 0xf0) == 16)   // 172.16/12
                || (o[0] == 192 && o[1] == 168)           // 192.168/16
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0/24 protocol assignments
                || (o[0] == 192 && o[1] == 0 && o[2] == 2) // TEST-NET-1
                || (o[0] == 198 && (o[1] & 0xfe) == 18)   // 198.18/15 benchmarking
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // TEST-NET-3
                || o[0] >= 224 // multicast 224/4, reserved 240/4, broadcast
        }
        IpAddr::V6(a) => {
            if a.is_loopback() || a.is_unspecified() {
                return true;
            }
            if let Some(v4) = embedded_v4(a) {
                return is_internal(IpAddr::V4(v4));
            }
            let s = a.segments();
            (s[0] & 0xe000) != 0x2000           // outside 2000::/3 global unicast: ULA, link/site-local, multicast, 64:ff9b:1::/48...
                || (s[0] == 0x2001 && s[1] == 0)     // Teredo
                || (s[0] == 0x2001 && s[1] == 0xdb8) // documentation
                || (s[0] == 0x2001 && s[1] == 2 && s[2] == 0) // benchmarking
                || (s[0] == 0x2001 && (s[1] & 0xfff0) == 0x10) // ORCHID
        }
    }
}

/// The IPv4 address inside an IPv6 address that embeds one: mapped
/// (::ffff:a.b.c.d), compatible (::a.b.c.d), NAT64 (64:ff9b::a.b.c.d) and
/// 6to4 (2002:aabb:ccdd::). Such addresses are judged by their IPv4 part.
fn embedded_v4(a: std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    let s = a.segments();
    let v4 = |hi: u16, lo: u16| std::net::Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    let mapped_or_compatible = s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0);
    let nat64 = s[..6] == [0x64, 0xff9b, 0, 0, 0, 0];
    if mapped_or_compatible || nat64 {
        Some(v4(s[6], s[7]))
    } else if s[0] == 0x2002 {
        Some(v4(s[1], s[2]))
    } else {
        None
    }
}

/// Link-local addresses and cloud instance metadata endpoints
/// (169.254.169.254, Alibaba's 100.100.100.200, fe80::/10, AWS's
/// fd00:ec2::254), including IPv6 forms that embed them. Blocked even with internal addresses
/// allowed: a compromised station on a LAN mustn't be able to redirect the
/// probe to a cloud server's credentials endpoint.
pub fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        // 169.254/16 incl. 169.254.169.254 (AWS, GCP, Azure, Oracle, ...); 100.100.100.200 (Alibaba)
        IpAddr::V4(a) => a.octets()[..2] == [169, 254] || a.octets() == [100, 100, 100, 200],
        IpAddr::V6(a) => {
            if let Some(v4) = embedded_v4(a) {
                return is_link_local(IpAddr::V4(v4)); // e.g. NAT64 / 6to4 forms of 169.254.169.254
            }
            let s = a.segments();
            if s[..3] == [0x64, 0xff9b, 1] {
                // local-use NAT64 (RFC 8215): judge the IPv4 address it carries
                let v4 = std::net::Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
                return is_link_local(IpAddr::V4(v4));
            }
            (s[0] & 0xffc0) == 0xfe80 || s == [0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254]
        }
    }
}

/// The internal-address guard for one host's resolved addresses. `what`
/// says where the host came from, for the message (e.g. " (listed in the HLS
/// playlist)").
fn guard(host: &str, addrs: &[SocketAddr], allow: Allow, what: &str) -> Result<(), NetError> {
    let blocked = |ip: IpAddr| is_internal(ip) && (!allow.internal || (is_link_local(ip) && !allow.link_local));
    let Some(a) = addrs.iter().find(|a| blocked(a.ip())) else {
        return Ok(());
    };
    let why = if allow.internal {
        "it is a link-local / cloud-metadata address (only probed when you type that address yourself)"
    } else {
        "it is an internal (private/loopback/link-local) address. If you meant to probe an internal server, re-run with --allow-internal"
    };
    Err(err(
        "blocked_internal",
        format!("refused to connect to {host}{what} ({}): {why}", a.ip()),
    ))
}

/// Which non-public addresses this session may connect to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allow {
    pub internal: bool,
    pub link_local: bool,
}

/// What the URL the user typed allows. Internal addresses only when it is
/// literally an internal IP or localhost (or `--allow-internal`, added by the
/// caller); link-local/metadata only when it is literally such an address.
/// Never decided by DNS: the station controls its own DNS and could make a
/// hostname resolve to an internal address on purpose.
pub fn start_allows_internal(url: &Url) -> Allow {
    let ip = match url.host() {
        Some(Host::Ipv4(a)) => Some(IpAddr::V4(a)),
        Some(Host::Ipv6(a)) => Some(IpAddr::V6(a)),
        _ => None,
    };
    let localhost = matches!(url.host(), Some(Host::Domain(d)) if {
        let d = d.trim_end_matches('.').to_ascii_lowercase();
        d == "localhost" || d.ends_with(".localhost")
    });
    Allow {
        internal: localhost || ip.is_some_and(is_internal),
        link_local: ip.is_some_and(is_link_local),
    }
}

/// `allow_internal = false` refuses any hop (the URL itself, a redirect, a
/// playlist entry) that resolves to an internal address, so a stream can't
/// steer the probe into the network it runs on (routers, cloud metadata,
/// admin pages) and copy what it finds into the report. We connect only to
/// the addresses we checked, so DNS rebinding can't slip past it. (HLS is the
/// exception: ffmpeg re-fetches the playlist and its segments itself; see
/// SECURITY.md.)
pub fn open_stream(start: &Url, insecure: bool, allow: Allow, log: &mut OpenLog) -> Result<Opened, NetError> {
    let mut url = start.clone();
    for _ in 0..8 {
        let scheme = url.scheme().to_string();
        if scheme != "http" && scheme != "https" {
            return Err(err("bad_url", format!("unsupported scheme '{scheme}'")));
        }
        let host = match url.host() {
            Some(Host::Domain(d)) => d.to_string(),
            Some(Host::Ipv4(a)) => a.to_string(),
            Some(Host::Ipv6(a)) => a.to_string(),
            None => return Err(err("bad_url", format!("no host in URL {url}"))),
        };
        let port = url.port_or_known_default().unwrap_or(80);
        let path = match &url[Position::BeforePath..Position::AfterQuery] {
            "" => "/".to_string(),
            p => p.to_string(),
        };
        let mut info = ConnInfo {
            host: host.clone(),
            port,
            scheme: scheme.clone(),
            ..Default::default()
        };

        let t0 = Instant::now();
        let addrs: Vec<SocketAddr> = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|e| err("dns_failed", format!("DNS lookup failed for {host}: {e}")))?
            .collect();
        info.dns_ms = ms(t0);
        let mut ips: Vec<String> = addrs.iter().map(|a| a.ip().to_string()).collect();
        ips.sort();
        ips.dedup();
        info.addrs = ips;
        if let Err(e) = guard(&host, &addrs, allow, "") {
            log.info = Some(info);
            return Err(e);
        }

        let t1 = Instant::now();
        let mut sock = None;
        let mut last_err = None;
        for a in &addrs {
            match TcpStream::connect_timeout(a, CONNECT_TIMEOUT) {
                Ok(s) => {
                    info.ip = a.ip().to_string();
                    sock = Some((s, *a));
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        let (sock, addr) = match sock {
            Some(s) => s,
            None => {
                log.info = Some(info);
                return Err(err(
                    "tcp_failed",
                    format!(
                        "TCP connect to {host}:{port} failed: {}",
                        last_err.map(|e| e.to_string()).unwrap_or_default()
                    ),
                ));
            }
        };
        info.connect_ms = ms(t1);
        let _ = sock.set_read_timeout(Some(CONNECT_TIMEOUT));
        let _ = sock.set_write_timeout(Some(CONNECT_TIMEOUT));
        let _ = sock.set_nodelay(true);

        let mut conn = if scheme == "https" {
            let t2 = Instant::now();
            let (tls, sock) = match tls_handshake(sock, &host, false) {
                Ok(x) => x,
                Err(e) => match cert_error(&e) {
                    Some(m) => {
                        log.tls_error = Some(m.clone());
                        if !insecure {
                            log.info = Some(info);
                            return Err(err(
                                "tls_cert",
                                format!(
                                    "TLS certificate verification failed: {m} (players will refuse this; re-run with --insecure to analyse the audio anyway)"
                                ),
                            ));
                        }
                        let s2 = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
                            .map_err(|e| err("tcp_failed", format!("TCP reconnect failed: {e}")))?;
                        let _ = s2.set_read_timeout(Some(CONNECT_TIMEOUT));
                        tls_handshake(s2, &host, true)
                            .map_err(|e| err("tls_failed", format!("TLS handshake failed: {e}")))?
                    }
                    None => {
                        log.info = Some(info);
                        return Err(err("tls_failed", format!("TLS handshake failed: {e}")));
                    }
                },
            };
            info.tls_ms = Some(ms(t2));
            info.tls_version = tls.protocol_version().map(|v| match v {
                rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2".to_string(),
                rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3".to_string(),
                o => format!("{o:?}"),
            });
            info.tls_cipher = tls
                .negotiated_cipher_suite()
                .map(|s| format!("{:?}", s.suite()).replace("TLS13_", "").replace("TLS_", ""));
            if let Some((days, expires, issuer)) = tls
                .peer_certificates()
                .and_then(|c| c.first())
                .and_then(|c| cert_info(c.as_ref()))
            {
                info.cert_days_left = Some(days);
                info.cert_expires = Some(expires);
                info.cert_issuer = issuer;
            }
            Conn::Tls(Box::new(StreamOwned::new(tls, sock)))
        } else {
            Conn::Plain(sock)
        };

        let hostport = match url.port() {
            Some(p) => format!("{}:{p}", url.host_str().unwrap_or(&host)),
            None => url.host_str().unwrap_or(&host).to_string(),
        };
        let req = format!(
            "GET {path} HTTP/1.0\r\nHost: {hostport}\r\nUser-Agent: {USER_AGENT}\r\nAccept: */*\r\nIcy-MetaData: 1\r\nConnection: close\r\n\r\n"
        );
        let t3 = Instant::now();
        if let Err(e) = conn.write_all(req.as_bytes()).and_then(|_| conn.flush()) {
            log.info = Some(info);
            return Err(err("no_response", format!("could not send request: {e}")));
        }
        let (head, leftover) = match read_headers(&mut conn) {
            Ok(x) => x,
            Err(e) => {
                log.info = Some(info);
                return Err(e);
            }
        };
        info.headers_ms = ms(t3);

        let head = String::from_utf8_lossy(&head).to_string(); // headers are latin-1-ish
        let mut lines = head.lines();
        let status_line = lines.next().unwrap_or("").trim().to_string();
        let caps = match regex!(r"^(HTTP/\d(?:\.\d)?|ICY)\s+(\d{3})\s*(.*)$").captures(&status_line) {
            Some(c) => c,
            None => {
                log.info = Some(info);
                return Err(err(
                    "bad_response",
                    format!(
                        "unrecognised response: {:?}",
                        status_line.chars().take(80).collect::<String>()
                    ),
                ));
            }
        };
        let status: u16 = caps[2].parse().unwrap_or(0);
        let reason = cap(&caps[3], 120);
        info.status = format!("{} {status} {reason}", &caps[1]).trim().to_string();
        let raw: Vec<(&str, &str)> = lines.filter_map(|l| l.split_once(':')).collect();
        // stored/displayed copies are cleaned and capped; a redirect is followed
        // with the full Location (signed CDN URLs are often longer than the cap)
        let headers: Vec<(String, String)> = raw
            .iter()
            .map(|(k, v)| (cap(&k.trim().to_lowercase(), 100), cap(v.trim(), 300)))
            .collect();
        let location = raw
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("location"))
            .map(|(_, v)| v.trim().chars().take(8192).collect::<String>());

        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            if let Some(loc) = location.as_deref() {
                if let Ok(next) = url.join(loc) {
                    log.redirects.push((status, next.to_string()));
                    url = next;
                    continue;
                }
            }
        }
        log.info = Some(info);
        if status != 200 {
            let mut body = leftover;
            conn.set_read_timeout(Duration::from_secs(2));
            let mut tmp = [0u8; 2048];
            let until = Instant::now() + Duration::from_secs(4);
            while body.len() < 2048 && Instant::now() < until {
                match conn.read(&mut tmp) {
                    Ok(n) if n > 0 => body.extend_from_slice(&tmp[..n]),
                    _ => break,
                }
            }
            let text: String = cap(&strip_tags(&String::from_utf8_lossy(&body)), 160);
            let hint = match status {
                401 => " (authentication required)",
                403 => " (forbidden - listener limit reached, IP/geo block or restricted mount?)",
                404 => " (mount point not found - is the station's encoder/source connected?)",
                503 => " (service unavailable - server full or source down?)",
                _ => "",
            };
            let said = if text.is_empty() {
                String::new()
            } else {
                format!(" - server said: {text}")
            };
            return Err(err(
                &format!("http_{status}"),
                format!("HTTP {status} {reason}{hint}{said}").replace("  ", " "),
            ));
        }
        return Ok(Opened {
            conn,
            headers,
            leftover,
            url,
        });
    }
    Err(err("redirect_loop", "too many redirects"))
}

// ---------------------------------------------------------------------------
// Playlists
// ---------------------------------------------------------------------------
fn looks_like_playlist(ctype: &str, data: &[u8]) -> bool {
    if ["mpegurl", "scpls", "pls", "xspf", "vnd.apple"]
        .iter()
        .any(|p| ctype.contains(p))
    {
        return true;
    }
    let head = String::from_utf8_lossy(&data[..data.len().min(512)])
        .trim_start()
        .to_lowercase();
    head.starts_with("#extm3u") || head.starts_with("[playlist]")
}

/// Some((is_hls, url)) for a playlist body.
/// URLs an HLS playlist points at: media/variant lines and `URI="…"` attributes.
fn hls_uris(text: &str, base: &Url) -> Vec<Url> {
    let mut v = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if line.starts_with('#') {
            for c in regex!(r#"URI="([^"]*)""#).captures_iter(line) {
                v.extend(base.join(&c[1]).ok());
            }
        } else {
            v.extend(base.join(line).ok());
        }
        if v.len() >= 5000 {
            break;
        }
    }
    v
}

/// For HLS, ffmpeg fetches the playlist's URLs itself, past our client. So
/// before handing it over, every host the playlist lists (and, for a master
/// playlist, the hosts its first variant playlists list) goes through the
/// internal-address guard. Variant playlists are fetched with our own,
/// guarded client. What remains unchecked: playlist refreshes and DNS answers
/// that change after this check (SECURITY.md).
fn check_hls(text: &str, base: &Url, insecure: bool, allow: Allow) -> Result<(), NetError> {
    let mut seen = std::collections::HashSet::new();
    let mut check = |urls: &[Url]| -> Result<(), NetError> {
        for u in urls {
            if !matches!(u.scheme(), "http" | "https") {
                return Err(err(
                    "blocked_internal",
                    format!(
                        "HLS playlist lists a non-HTTP URL ({}:), refusing to let ffmpeg open it",
                        u.scheme()
                    ),
                ));
            }
            let (Some(host), Some(port)) = (u.host_str(), u.port_or_known_default()) else {
                continue;
            };
            let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
            if seen.len() >= 64 || !seen.insert((host.clone(), port)) {
                continue;
            }
            // an unresolvable host is ffmpeg's problem to report, not a risk
            if let Ok(addrs) = (host.as_str(), port).to_socket_addrs() {
                guard(
                    &host,
                    &addrs.collect::<Vec<_>>(),
                    allow,
                    " (listed in the HLS playlist)",
                )?;
            }
        }
        Ok(())
    };
    let urls = hls_uris(text, base);
    check(&urls)?;
    let master = text.contains("#EXT-X-STREAM-INF") || text.contains("#EXT-X-MEDIA:");
    if master {
        let mut variants: Vec<&Url> = Vec::new();
        for u in &urls {
            if !variants.contains(&u) {
                variants.push(u);
            }
        }
        for v in variants.into_iter().take(4) {
            let mut o = match open_stream(v, insecure, allow, &mut OpenLog::default()) {
                Ok(o) => o,
                Err(e) if e.code == "blocked_internal" => return Err(e),
                Err(_) => continue, // ffmpeg will report a broken variant
            };
            let mut body = std::mem::take(&mut o.leftover);
            o.conn.set_read_timeout(Duration::from_secs(3));
            let mut tmp = [0u8; 16384];
            let until = Instant::now() + Duration::from_secs(8);
            while body.len() < 262144 && Instant::now() < until {
                match o.conn.read(&mut tmp) {
                    Ok(n) if n > 0 => body.extend_from_slice(&tmp[..n]),
                    _ => break,
                }
            }
            check(&hls_uris(&String::from_utf8_lossy(&body), &o.url))?;
        }
    }
    Ok(())
}

fn parse_playlist(text: &str, base: &Url) -> Option<(bool, Url)> {
    let t = text.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("#EXTM3U") && t.contains("#EXT-X-") {
        return Some((true, base.clone())); // HLS: the spec requires #EXTM3U first
    }
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(c) = regex!(r"(?i)^File\d+\s*=\s*(.+)$").captures(line) {
            return base.join(c[1].trim()).ok().map(|u| (false, u));
        }
        if line.starts_with('#') || line.starts_with('[') || (line.contains('=') && !line.contains("://")) {
            continue;
        }
        return base.join(line).ok().map(|u| (false, u));
    }
    regex!(r"<location>\s*([^<]+)</location>")
        .captures(text)
        .and_then(|c| base.join(c[1].trim()).ok())
        .map(|u| (false, u))
}

fn merge_log(sh: &Shared, log: OpenLog) {
    let mut st = sh.lock();
    if st.first_conn.is_none() {
        // the path to the stream; reconnects through the same CDN redirect again
        let room = 20usize.saturating_sub(st.redirects.len());
        st.redirects.extend(log.redirects.into_iter().take(room));
    }
    if log.tls_error.is_some() {
        st.tls_error = log.tls_error;
    }
    if let Some(i) = log.info {
        st.conn = Some(i);
    }
}

/// First connection: follow playlists and detect HLS. Returns the open
/// connection for the reader thread, or None for direct (ffmpeg) mode.
pub fn preflight(sh: &Shared, start: &Url) -> Result<Option<Opened>, NetError> {
    let typed = start_allows_internal(start);
    let allow = Allow {
        internal: sh.allow_internal || typed.internal,
        link_local: typed.link_local,
    };
    sh.lock().allow = allow;
    let mut url = start.clone();
    for _ in 0..4 {
        let mut log = OpenLog::default();
        let r = open_stream(&url, sh.insecure, allow, &mut log);
        merge_log(sh, log);
        let mut o = r?;
        let ctype = header(&o.headers, "content-type").unwrap_or("").to_lowercase();
        if looks_like_playlist(&ctype, &o.leftover) || ctype.starts_with("text/") {
            let mut body = std::mem::take(&mut o.leftover);
            o.conn.set_read_timeout(Duration::from_secs(3));
            let mut tmp = [0u8; 16384];
            let until = Instant::now() + Duration::from_secs(10);
            while body.len() < 262144 && Instant::now() < until {
                match o.conn.read(&mut tmp) {
                    Ok(n) if n > 0 => body.extend_from_slice(&tmp[..n]),
                    _ => break,
                }
            }
            let text = String::from_utf8_lossy(&body).to_string();
            let low: String = text.chars().take(4000).collect::<String>().to_lowercase();
            if low.contains("<html") {
                let title = regex!(r"(?is)<title>(.*?)</title>")
                    .captures(&text)
                    .map(|c| format!(" ('{}')", cap(c[1].trim(), 60)))
                    .unwrap_or_default();
                return Err(err(
                    "not_audio",
                    format!("URL returned a web page, not audio{title} - check the mount point / stream URL"),
                ));
            }
            match parse_playlist(&text, &o.url) {
                Some((true, _)) => {
                    check_hls(&text, &o.url, sh.insecure, allow)?;
                    let mut st = sh.lock();
                    st.mode_direct = true;
                    st.final_url = o.url.to_string();
                    st.headers = o.headers.clone();
                    st.first_conn = st.conn.clone();
                    return Ok(None);
                }
                Some((false, next)) => {
                    let t = sh.now();
                    let mut st = sh.lock();
                    st.playlist_from = Some(o.url.to_string());
                    st.event(t, Level::Info, "playlist", format!("Playlist -> {next}"));
                    url = next;
                    continue;
                }
                None => {
                    return Err(err(
                        "not_audio",
                        format!("got '{ctype}' that is neither audio nor a playlist"),
                    ));
                }
            }
        }
        let mut st = sh.lock();
        st.final_url = o.url.to_string();
        st.headers = o.headers.clone();
        st.first_conn = st.conn.clone();
        return Ok(Some(o));
    }
    Err(err("redirect_loop", "too many nested playlists"))
}

// ---------------------------------------------------------------------------
// ICY metadata demux + reader thread
// ---------------------------------------------------------------------------
struct IcyParser {
    metaint: usize,
    audio_left: usize,
    meta_len: Option<usize>,
    meta_buf: Vec<u8>,
}

impl IcyParser {
    fn new(metaint: usize) -> Self {
        IcyParser {
            metaint,
            audio_left: metaint,
            meta_len: None,
            meta_buf: Vec::new(),
        }
    }

    fn push(&mut self, mut data: &[u8], audio: &mut Vec<u8>, metas: &mut Vec<Vec<u8>>) {
        if self.metaint == 0 {
            audio.extend_from_slice(data);
            return;
        }
        while !data.is_empty() {
            if let Some(need) = self.meta_len {
                let take = (need - self.meta_buf.len()).min(data.len());
                self.meta_buf.extend_from_slice(&data[..take]);
                data = &data[take..];
                if self.meta_buf.len() == need {
                    metas.push(std::mem::take(&mut self.meta_buf));
                    self.meta_len = None;
                    self.audio_left = self.metaint;
                }
            } else if self.audio_left > 0 {
                let take = self.audio_left.min(data.len());
                audio.extend_from_slice(&data[..take]);
                data = &data[take..];
                self.audio_left -= take;
            } else {
                let len = data[0] as usize * 16;
                data = &data[1..];
                if len == 0 {
                    self.audio_left = self.metaint;
                } else {
                    self.meta_len = Some(len);
                    self.meta_buf.clear();
                }
            }
        }
    }
}

fn on_meta(sh: &Shared, raw: &[u8]) {
    let end = raw.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    let raw = &raw[..end];
    let (text, non_utf8) = match std::str::from_utf8(raw) {
        Ok(s) => (s.to_string(), false),
        Err(_) => (raw.iter().map(|&b| b as char).collect::<String>(), true), // latin-1
    };
    let now = sh.now();
    let mut st = sh.lock();
    st.meta_packets += 1;
    st.meta_non_utf8 |= non_utf8;
    let Some(c) = regex!(r"(?s)StreamTitle='(.*?)';").captures(&text) else {
        return;
    };
    let title = cap(c[1].trim(), 200);
    if st.title.as_deref() == Some(title.as_str()) {
        return;
    }
    let first = st.title.is_none();
    st.title = Some(title.clone());
    st.title_since = now;
    st.push_title(now, title.clone());
    st.bucket(now).title = true;
    let shown = if title.is_empty() {
        "(empty title)".to_string()
    } else {
        title
    };
    st.event(
        now,
        Level::Info,
        "title",
        format!("{}{shown}", if first { "Title: " } else { "Now playing: " }),
    );
}

/// Pump one connection until it ends; returns the reason.
fn pump(sh: &Shared, sink: &mut ChildStdin, o: Opened) -> String {
    let Opened {
        mut conn,
        headers,
        leftover,
        ..
    } = o;
    let metaint: usize = header(&headers, "icy-metaint")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    {
        let now = sh.now();
        let mut st = sh.lock();
        st.metaint = metaint;
        st.connected = true;
        st.last_connect = now;
        st.frames.new_connection();
    }
    conn.set_read_timeout(Duration::from_secs(1));
    let mut parser = IcyParser::new(metaint);
    let (mut audio, mut metas) = (Vec::with_capacity(32768), Vec::new());
    let mut buf = vec![0u8; 16384];
    let mut data = leftover;
    let mut last_data = sh.now();
    loop {
        if !data.is_empty() {
            audio.clear();
            parser.push(&data, &mut audio, &mut metas);
            data.clear();
            if !audio.is_empty() {
                {
                    let mut st = sh.lock();
                    st.audio_bytes_total += audio.len() as u64;
                    st.frames.push(&audio);
                    let room = PROBE_SAMPLE_BYTES.saturating_sub(st.probe_sample.len());
                    if room > 0 {
                        st.probe_sample.extend_from_slice(&audio[..room.min(audio.len())]);
                    }
                }
                if sink.write_all(&audio).is_err() {
                    sh.lock().decoder_gone = true;
                    sh.set_stop();
                    return "decoder stopped".into();
                }
            }
            for m in metas.drain(..) {
                on_meta(sh, &m);
            }
        }
        if sh.stopped() {
            return "stopped".into();
        }
        match conn.read(&mut buf) {
            Ok(0) => return "server closed the connection".into(),
            Ok(n) => {
                let now = sh.now();
                let mut st = sh.lock();
                if st.stall_active {
                    st.stall_active = false;
                    st.event(
                        now,
                        Level::Ok,
                        "stall_end",
                        format!("Data resumed after {:.1}s stall", now - last_data),
                    );
                }
                st.net_bytes_total += n as u64;
                st.net_window.push_back((now, n as u64));
                while st.net_window.front().is_some_and(|(t, _)| now - t > 10.0) {
                    st.net_window.pop_front();
                }
                st.bucket(now).net_bytes += n as u64;
                drop(st);
                last_data = now;
                data.extend_from_slice(&buf[..n]);
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                let now = sh.now();
                let gap = now - last_data;
                let mut st = sh.lock();
                if gap >= STALL_SEC && !st.stall_active {
                    st.stall_active = true;
                    st.stalls += 1;
                    st.event(last_data, Level::Warn, "stall", "Stall: server stopped sending data");
                }
                if st.stall_active {
                    st.bucket(now).stall = true;
                }
                drop(st);
                if gap >= DEAD_CONN_SEC {
                    return format!("no data for {gap:.0}s");
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return "server closed the connection".into(),
            Err(e) => return format!("socket error: {e}"),
        }
    }
}

/// Reader thread: pumps the stream into ffmpeg, reconnecting on failure.
pub fn net_reader(sh: Arc<Shared>, mut sink: ChildStdin, first: Opened) {
    let mut next = Some(first);
    while !sh.stopped() {
        let o = match next.take() {
            Some(o) => o,
            None => {
                let (url, allow) = {
                    let st = sh.lock();
                    (Url::parse(&st.final_url).expect("final url"), st.allow)
                };
                let mut log = OpenLog::default();
                let r = open_stream(&url, sh.insecure, allow, &mut log);
                merge_log(&sh, log);
                let now = sh.now();
                match r {
                    Ok(o) => {
                        let mut st = sh.lock();
                        let ms = st.conn.as_ref().map_or(0.0, |c| c.connect_ms);
                        st.event(now, Level::Ok, "reconnected", format!("Reconnected (tcp {ms:.0} ms)"));
                        o
                    }
                    Err(e) => {
                        let mut st = sh.lock();
                        st.reconnect_fail += 1;
                        st.bucket(now).disconnected = true;
                        st.event(
                            now,
                            Level::Bad,
                            "reconnect_failed",
                            format!("Reconnect failed: {}", e.msg),
                        );
                        drop(st);
                        sh.sleep(RECONNECT_DELAY);
                        continue;
                    }
                }
            }
        };
        let reason = pump(&sh, &mut sink, o);
        if sh.stopped() {
            break;
        }
        let now = sh.now();
        {
            let mut st = sh.lock();
            st.connected = false;
            st.disconnects += 1;
            st.bucket(now).disconnected = true;
            st.event(now, Level::Bad, "disconnect", format!("Disconnected: {reason}"));
        }
        sh.sleep(RECONNECT_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icy_parser_splits_audio_and_metadata_across_any_boundary() {
        let meta = b"StreamTitle='A - B';";
        let mut block = vec![0u8; 32];
        block[..meta.len()].copy_from_slice(meta);
        let mut stream = b"abcd".to_vec();
        stream.push(2); // 2 * 16 bytes of metadata
        stream.extend(&block);
        stream.extend(b"efgh");
        stream.push(0); // empty metadata block
        stream.extend(b"ij");
        let mut p = IcyParser::new(4);
        let (mut audio, mut metas) = (Vec::new(), Vec::new());
        for b in &stream {
            p.push(std::slice::from_ref(b), &mut audio, &mut metas); // byte by byte
        }
        assert_eq!(audio, b"abcdefghij");
        assert_eq!(metas.len(), 1);
        assert!(metas[0].starts_with(meta));
    }

    #[test]
    fn icy_parser_passes_through_without_metaint() {
        let mut p = IcyParser::new(0);
        let (mut audio, mut metas) = (Vec::new(), Vec::new());
        p.push(b"\x02raw", &mut audio, &mut metas);
        assert_eq!(audio, b"\x02raw");
        assert!(metas.is_empty());
    }

    #[test]
    fn parses_pls_m3u_relative_and_hls() {
        let base = Url::parse("http://example.com/radio/listen.pls").unwrap();
        let u = |s: &str| Url::parse(s).unwrap();
        let pls = "[playlist]\nNumberOfEntries=1\nFile1=http://s.example.com:8000/live\nTitle1=x\n";
        assert_eq!(
            parse_playlist(pls, &base),
            Some((false, u("http://s.example.com:8000/live")))
        );
        let m3u = "#EXTM3U\n#EXTINF:-1,Station\nhttps://s.example.com/live.mp3\n";
        assert_eq!(
            parse_playlist(m3u, &base),
            Some((false, u("https://s.example.com/live.mp3")))
        );
        assert_eq!(
            parse_playlist("live.aac\n", &base),
            Some((false, u("http://example.com/radio/live.aac")))
        );
        let hls = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-STREAM-INF:BANDWIDTH=1\nchunk.m3u8\n";
        assert_eq!(parse_playlist(hls, &base), Some((true, base.clone())));
        assert_eq!(parse_playlist("", &base), None);
    }

    #[test]
    fn internal_addresses_are_recognised() {
        let internal = [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fd00::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "::ffff:192.168.0.1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b:1::1",
            "2002:c0a8:101::1",
            "2001:db8::1",
            "2001::1",
        ];
        for ip in internal {
            assert!(is_internal(ip.parse().unwrap()), "{ip} should be internal");
        }
        let public = [
            "8.8.8.8",
            "93.190.141.46",
            "100.128.0.1",
            "172.32.0.1",
            "198.20.0.1",
            "2a00:1450::1",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
        ];
        for ip in public {
            assert!(!is_internal(ip.parse().unwrap()), "{ip} should be public");
        }
    }

    #[test]
    fn only_the_users_own_literal_internal_url_allows_internal_hops() {
        let u = |s: &str| start_allows_internal(&Url::parse(s).unwrap());
        assert!(u("http://127.0.0.1:8000/live").internal);
        assert!(u("http://[fd00::1]:8000/live").internal);
        assert!(u("http://localhost:8000/live").internal);
        assert!(!u("http://8.8.8.8/live").internal);
        // a hostname never enables it, whatever it resolves to (localtest.me -> 127.0.0.1)
        assert!(!u("http://localtest.me:8000/live").internal);
        // metadata/link-local only when typed literally
        assert!(!u("http://10.0.0.5/live").link_local);
        assert!(u("http://169.254.10.1/live").link_local);
    }

    #[test]
    fn refuses_internal_hops_unless_allowed() {
        let code =
            |url: &str, allow| match open_stream(&Url::parse(url).unwrap(), false, allow, &mut OpenLog::default()) {
                Ok(_) => "connected".to_string(),
                Err(e) => e.code,
            };
        let none = Allow::default();
        let lan = Allow {
            internal: true,
            link_local: false,
        };
        assert_eq!(code("http://127.0.0.1:9/live", none), "blocked_internal"); // refused before connecting
        assert_ne!(code("http://127.0.0.1:9/live", lan), "blocked_internal"); // allowed: fails on the closed port
                                                                              // cloud metadata stays blocked even when internal addresses are allowed
        assert_eq!(
            code("http://169.254.169.254/latest/meta-data/", lan),
            "blocked_internal"
        );
        assert_eq!(
            code("http://[fd00:ec2::254]/latest/meta-data/", lan),
            "blocked_internal"
        );
        // ...including IPv6 forms that embed 169.254.169.254 (mapped, NAT64, 6to4)
        for v6 in ["::ffff:a9fe:a9fe", "64:ff9b::a9fe:a9fe", "2002:a9fe:a9fe::1"] {
            assert!(is_link_local(v6.parse().unwrap()), "{v6}");
            assert_eq!(code(&format!("http://[{v6}]/"), lan), "blocked_internal", "{v6}");
        }
    }

    #[test]
    fn hls_playlists_listing_internal_hosts_are_refused() {
        let base = Url::parse("https://cdn.example.com/live/index.m3u8").unwrap();
        let none = Allow::default();
        let lan = Allow {
            internal: true,
            link_local: false,
        };
        let media = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nhttp://127.0.0.1/a.ts\n";
        assert_eq!(
            check_hls(media, &base, false, none).unwrap_err().code,
            "blocked_internal"
        );
        assert!(check_hls(media, &base, false, lan).is_ok());
        let key = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://169.254.169.254/latest/meta-data/\"\n#EXTINF:4,\nhttp://127.0.0.1/a.ts\n";
        assert_eq!(check_hls(key, &base, false, lan).unwrap_err().code, "blocked_internal");
        let local = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nfile:///etc/passwd\n";
        assert_eq!(
            check_hls(local, &base, false, lan).unwrap_err().code,
            "blocked_internal"
        );
        // HLS needs #EXTM3U first; a stray #EXT-X- elsewhere isn't HLS
        assert_eq!(
            parse_playlist("hello\n#EXT-X-VERSION:3\nhttp://x/y\n", &base).map(|p| p.0),
            Some(false)
        );
    }

    #[test]
    fn metadata_endpoints_stay_blocked_in_every_form() {
        for ip in [
            "169.254.169.254",
            "100.100.100.200",
            "fd00:ec2::254",
            "64:ff9b:1::a9fe:a9fe",
            "64:ff9b::6464:64c8",
        ] {
            assert!(is_link_local(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_link_local("100.100.100.201".parse().unwrap()));
    }

    #[test]
    fn long_redirect_locations_are_followed_in_full() {
        let target = format!("/live?token={}", "a".repeat(1000));
        let mut first = true;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t2 = target.clone();
        std::thread::spawn(move || {
            for s in l.incoming().take(2).flatten() {
                let mut s = s;
                let mut req = [0u8; 4096];
                let n = s.read(&mut req).unwrap_or(0);
                let line = String::from_utf8_lossy(&req[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let resp = if first {
                    format!("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{port}{t2}\r\n\r\n")
                } else if line.contains(&t2) {
                    "ICY 200 OK\r\ncontent-type: audio/mpeg\r\n\r\n".to_string()
                } else {
                    "HTTP/1.1 404 Not Found\r\n\r\n".to_string()
                };
                first = false;
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let url = Url::parse(&format!("http://127.0.0.1:{port}/start")).unwrap();
        let allow = Allow {
            internal: true,
            link_local: false,
        };
        let mut log = OpenLog::default();
        let r = open_stream(&url, false, allow, &mut log);
        assert!(r.is_ok(), "{:?}", r.err().map(|e| e.msg));
    }

    /// A local server that answers every connection with `resp` (up to 12).
    fn serve(resp: Vec<u8>) -> Url {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().take(12).flatten() {
                let mut s = s;
                let mut req = [0u8; 2048];
                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = s.read(&mut req);
                let _ = s.write_all(&resp);
            }
        });
        Url::parse(&format!("http://127.0.0.1:{port}/live")).unwrap()
    }

    fn open(resp: &[u8]) -> Result<Opened, NetError> {
        let allow = Allow {
            internal: true,
            link_local: false,
        };
        open_stream(&serve(resp.to_vec()), false, allow, &mut OpenLog::default())
    }

    fn noise(seed: u64, n: usize) -> Vec<u8> {
        let mut x = seed | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn hostile_responses_are_errors_not_panics() {
        let code = |r: &[u8]| match open(r) {
            Ok(_) => "ok".to_string(),
            Err(e) => e.code,
        };
        // garbage without a header terminator, then garbage with one
        assert_eq!(code(&noise(7, 3000)), "no_response");
        let mut garbled = noise(9, 500);
        garbled.extend(b"\r\n\r\n");
        assert_eq!(code(&garbled), "bad_response");
        assert_eq!(code(b""), "no_response");
        let mut endless = b"HTTP/1.1 200 OK\r\nX-Pad: ".to_vec();
        endless.extend(vec![b'a'; 70_000]); // headers that never end
        assert_eq!(code(&endless), "no_response");
        assert_eq!(code(b"HTTP/1.1 302 Found\r\nLocation: /live\r\n\r\n"), "redirect_loop");
        assert_eq!(
            code(b"HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\n\r\n"),
            "bad_url"
        );
        assert_ne!(code(b"HTTP/1.1 302 Found\r\nLocation: http://[::1\r\n\r\n"), "ok");
        let mut html = b"HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\n".to_vec();
        html.extend(noise(3, 20_000));
        match open(&html) {
            Err(e) => assert!(e.code == "http_404" && e.msg.len() < 600, "{}", e.msg),
            Ok(_) => panic!("404 accepted"),
        }
    }

    #[test]
    fn odd_but_valid_responses_open() {
        let r = b"ICY 200 OK\r\nicy-metaint: 99999999999999999999999\r\nicy-name: \xff\xfe\x00\x01\r\n\r\nAUDIO";
        let o = open(r).unwrap_or_else(|e| panic!("{}: {}", e.code, e.msg));
        assert!(header(&o.headers, "icy-name").is_some());
        let mut long = b"HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\nicy-description: ".to_vec();
        long.extend(vec![b'x'; 5000]);
        long.extend(b"\r\n\r\n");
        let o = open(&long).unwrap_or_else(|e| panic!("{}: {}", e.code, e.msg));
        assert!(header(&o.headers, "icy-description").unwrap().chars().count() <= 301);
    }

    #[test]
    fn parsers_survive_random_input() {
        for seed in 1..60u64 {
            let bytes = noise(seed, 4000);
            // ICY demux with random metaint and random chunking
            let mut p = IcyParser::new((seed % 50) as usize);
            let (mut audio, mut metas) = (Vec::new(), Vec::new());
            for c in bytes.chunks(1 + (seed as usize % 97)) {
                p.push(c, &mut audio, &mut metas);
            }
            assert!(audio.len() <= bytes.len());
            // playlists: random text, and random text after real playlist markers
            let base = Url::parse("http://example.com/a/b.pls").unwrap();
            let text = String::from_utf8_lossy(&bytes).to_string();
            for t in [
                text.clone(),
                format!("[playlist]\nFile1={text}"),
                format!("#EXTM3U\n{text}\n"),
            ] {
                let _ = parse_playlist(&t, &base);
                let _ = looks_like_playlist("text/plain", t.as_bytes());
            }
        }
    }

    #[test]
    fn detects_playlists_by_type_or_body() {
        assert!(looks_like_playlist("audio/x-scpls", b""));
        assert!(looks_like_playlist("application/vnd.apple.mpegurl", b""));
        assert!(looks_like_playlist("text/plain", b"  #EXTM3U\n"));
        assert!(!looks_like_playlist("audio/mpeg", b"\xff\xfb\x90\x64"));
    }
}
