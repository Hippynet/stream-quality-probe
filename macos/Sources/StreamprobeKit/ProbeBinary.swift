import Foundation

public enum ProbeBinary {
    /// `STREAMPROBE_BIN` (development), else the helper inside the app bundle.
    public static func locate() -> URL? {
        if let p = ProcessInfo.processInfo.environment["STREAMPROBE_BIN"], !p.isEmpty {
            return URL(fileURLWithPath: p)
        }
        let helper = Bundle.main.bundleURL.appending(path: "Contents/Helpers/streamprobe")
        return FileManager.default.isExecutableFile(atPath: helper.path) ? helper : nil
    }
}
