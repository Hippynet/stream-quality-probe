import Testing
@testable import StreamprobeKit

@Test func requestTrimsAndAddsLiveJSON() {
    let r = CheckRequest(url: "  http://x/live \n")
    #expect(r.isValid)
    #expect(r.arguments == ["http://x/live", "--live-json"])
}

@Test func requestPassesSchemelessURLThrough() {
    #expect(CheckRequest(url: "stream.example.com/live").arguments.first == "stream.example.com/live")
}

@Test func blankRequestIsInvalid() {
    #expect(!CheckRequest(url: "   ").isValid)
    #expect(!CheckRequest(url: "").isValid)
}

@Test func requestOptionsBecomeFlags() {
    let r = CheckRequest(url: "http://x/", durationS: 120, insecure: true, allowInternal: true)
    #expect(r.arguments == ["http://x/", "--live-json", "-t", "120", "--insecure", "--allow-internal"])
}
