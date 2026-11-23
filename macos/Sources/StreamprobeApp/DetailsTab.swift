import StreamprobeKit
import SwiftUI

/// What the server and encoder actually send. Every value here is server- or
/// stream-controlled, so it's all shown verbatim.
struct DetailsTab: View {
    let report: ProbeReport?
    let startInfo: StartInfo?

    var body: some View {
        if let r = report {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(sections(r), id: \.title) { s in
                        Card(s.title) {
                            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 14, verticalSpacing: 5) {
                                ForEach(Array(s.rows.enumerated()), id: \.offset) { _, row in
                                    GridRow {
                                        Text(verbatim: row.0).foregroundStyle(.secondary)
                                            .frame(minWidth: 140, alignment: .trailing)
                                            .gridColumnAlignment(.trailing)
                                        untrusted(row.1).frame(maxWidth: .infinity, alignment: .leading)
                                    }
                                }
                            }
                            .font(.callout)
                            .textSelection(.enabled)
                        }
                    }
                }
                .padding(16)
            }
        } else {
            Text("Details appear once streamprobe has a report.")
                .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    struct Section {
        let title: String
        let rows: [(String, String)]
    }

    private func sections(_ r: ProbeReport) -> [Section] {
        let h = r.headers, c = r.connection, f = r.format, m = r.metadata
        let fr = f["frames"] ?? .null, fp = f["ffprobe_format"] ?? .null, fs = f["ffprobe_stream"] ?? .null
        func v(_ x: JSONValue?) -> String? {
            guard let s = x?.displayString, !s.isEmpty else { return nil }
            return s
        }
        func ms(_ x: JSONValue?) -> String? { x?.numberValue.map { String(format: "%.0f ms", $0) } }
        func khz(_ x: JSONValue?) -> String? {
            (x?.numberValue ?? x?.stringValue.flatMap(Double.init)).map { String(format: "%g kHz", $0 / 1000) }
        }
        func kbps(_ x: JSONValue?) -> String? {
            (x?.numberValue ?? x?.stringValue.flatMap(Double.init)).map { String(format: "%g kb/s", $0 >= 1000 ? $0 / 1000 : $0) }
        }

        var out: [Section] = []
        func add(_ title: String, _ rows: [(String, String?)]) {
            let kept = rows.compactMap { k, val in val.map { (k, $0) } }
            if !kept.isEmpty { out.append(Section(title: title, rows: kept)) }
        }

        add("Station", [
            ("Name", v(h["icy-name"])), ("Genre", v(h["icy-genre"])), ("Website", v(h["icy-url"])),
            ("Description", v(h["icy-description"])),
            ("Listed", v(h["icy-pub"]).map { $0 == "1" ? "yes (public directory)" : $0 == "0" ? "no" : $0 }),
        ])
        add("Server", [
            ("Server", v(h["server"])), ("Status", v(c["status"])), ("Content-Type", v(h["content-type"])),
            ("ICY metadata every", v(m["icy_metaint"]).map { $0 == "0" ? "off" : "\($0) bytes" }),
            ("CORS", v(h["access-control-allow-origin"])),
            ("Stream URL", startInfo?.url),
        ])
        add("Connection", [
            ("Host", v(c["host"]).map { h in v(c["port"]).map { "\(h):\($0)" } ?? h }),
            ("IP", v(c["ip"])), ("Addresses", v(c["addrs"])),
            ("DNS", ms(c["dns_ms"])), ("TCP connect", ms(c["connect_ms"])), ("TLS", ms(c["tls_ms"])),
            ("Headers", ms(c["headers_ms"])),
            ("Playlist", v(c["playlist"])),
            ("Redirects", {
                guard case .array(let a)? = c["redirects"], !a.isEmpty else { return nil }
                return a.map { "\($0["status"]?.displayString ?? "") → \($0["to"]?.displayString ?? "")" }
                    .joined(separator: "\n")
            }()),
        ])
        add("TLS", [
            ("Version", v(c["tls_version"])), ("Cipher", v(c["tls_cipher"])),
            ("Certificate expires", v(c["cert_expires"]).map { exp in
                c["cert_days_left"]?.numberValue.map { String(format: "%@ (%.0f days left)", exp, $0) } ?? exp
            }),
            ("Issuer", v(c["cert_issuer"])), ("Problem", v(c["tls_error"])),
        ])
        add("Format", [
            ("Codec", [v(f["codec"]), v(f["profile"])].compactMap { $0 }.joined(separator: " ").nilIfEmpty),
            ("Sample rate", khz(f["sample_rate"])),
            ("Channels", [v(f["channels"]), v(f["channel_layout"])].compactMap { $0 }.joined(separator: " · ").nilIfEmpty),
            ("Bitrate declared", v(f["declared_kbps"]).map { "\($0) kb/s" }),
            ("Bitrate measured", v(f["measured_kbps"]).map { "\($0) kb/s" }),
            ("Decoder input", v(f["ffmpeg_input"])),
        ])
        add("Frames", [
            ("Summary", v(fr["summary"]) ?? v(fr["other"])),
            ("Frames", v(fr["frames"])), ("Sync losses", v(fr["sync_losses"])), ("Junk bytes", v(fr["junk_bytes"])),
            ("ID3 tags", v(fr["id3_tags"])), ("Format changes", v(fr["format_changes"])),
        ])
        add("FFprobe", [
            ("Container", v(fp["format_long_name"]).map { n in v(fp["probe_score"]).map { "\(n) (score \($0))" } ?? n }),
            ("Codec", v(fs["codec_long_name"])), ("Sample format", v(fs["sample_fmt"])),
            ("Layout", v(fs["channel_layout"])), ("Bitrate", kbps(fs["bit_rate"])),
            ("Tags", v(f["ffprobe_tags"])), ("Error", v(f["ffprobe_error"])),
        ])
        add("Metadata", [
            ("Current title", v(m["current_title"])),
            ("Metadata packets", v(m["packets"])),
            ("Not UTF-8", m["non_utf8"] == .bool(true) ? "yes: titles may show garbled" : nil),
        ])
        add("Response headers", h.keys.sorted().map { ($0, v(h[$0]) ?? "") })
        return out
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
