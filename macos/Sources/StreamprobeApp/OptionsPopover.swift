import SwiftUI

struct CheckOptions: Equatable {
    /// nil = until Stop; -1 = custom minutes.
    var duration: Int?
    var customMinutes = 15
    var insecure = false
    var allowInternal = false

    var durationS: Int? {
        guard let d = duration else { return nil }
        return d == -1 ? customMinutes * 60 : d
    }
}

struct OptionsPopover: View {
    @Binding var options: CheckOptions

    var body: some View {
        Form {
            Picker("Run for", selection: $options.duration) {
                Text("Until I press Stop").tag(Int?.none)
                Text("30 seconds").tag(Int?.some(30))
                Text("1 minute").tag(Int?.some(60))
                Text("2 minutes").tag(Int?.some(120))
                Text("5 minutes").tag(Int?.some(300))
                Text("10 minutes").tag(Int?.some(600))
                Text("Custom").tag(Int?.some(-1))
            }
            if options.duration == -1 {
                Stepper("\(options.customMinutes) minutes", value: $options.customMinutes, in: 1...720)
            }
            Toggle("Ignore bad TLS certificate", isOn: $options.insecure)
                .help("Carry on past an invalid certificate. The problem is still reported.")
            Toggle("Allow internal addresses", isOn: $options.allowInternal)
                .help("Allow private, loopback and link-local addresses, for a server on your own network.")
        }
        .formStyle(.grouped)
        .frame(width: 340)
    }
}
