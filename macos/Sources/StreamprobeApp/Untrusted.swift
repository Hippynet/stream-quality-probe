import SwiftUI

/// Text that came from the stream's server or encoder (titles, headers, ICY
/// fields, issuer, ffprobe output, finding and event messages). Always verbatim:
/// never Markdown, never a link.
func untrusted(_ s: String?) -> Text {
    Text(verbatim: s ?? "—")
}
