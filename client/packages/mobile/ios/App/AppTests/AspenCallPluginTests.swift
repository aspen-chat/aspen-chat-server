import AVFoundation
import WebKit
import XCTest
@testable import App

/// The web view's bridge to the call engine, through the app's own page: the app under test is
/// the app itself, whose `MainViewController` has loaded the web shell with `AspenCallPlugin`
/// registered, so the test finds its web view and drives `Capacitor.registerPlugin("AspenCall")`
/// from the page's JavaScript, as the app's `NativeCall` does. It reads a snapshot, chooses the
/// loudspeaker and the receiver and reads the session's route, joins a channel's call against a
/// real deployment and watches the engine's snapshots reach the page (as far as the simulator's
/// CallKit takes a call: the start action joins, and its daemon ends the call, which the page
/// sees too), mutes outside a call, and reports whether the page's Web Audio runs while the
/// call's audio session is active. `scripts/check_mobile_call.py` runs it with `CallEngineTests`
/// and gives it the same environment; without it, the join is skipped.
final class AspenCallPluginTests: XCTestCase {
    private var keptPushState: String?
    private var webView: WKWebView!

    /// What the page recorded under `window.__aspenCallTest`: the plugin's events, each `state`
    /// one read as the engine's JSON with its origin beside, and each command's outcome by name.
    private static let recorder = """
    (() => {
      const plugin = window.Capacitor.registerPlugin("AspenCall");
      const record = { states: [], routes: [], results: {} };
      window.__aspenCallTest = record;
      plugin.addListener("state", (published) => record.states.push({ ...JSON.parse(published.state), origin: published.origin }));
      plugin.addListener("routes", (routes) => record.routes.push(routes));
      window.__aspenCallRun = (id, method, options) => {
        void plugin[method](options).then(
          (value) => { record.results[id] = { ok: value === undefined ? null : value }; },
          (error) => { record.results[id] = { error: String(error && error.message || error) }; },
        );
      };
      return "ready";
    })()
    """

    override func setUp() async throws {
        try await super.setUp()
        continueAfterFailure = false
        webView = try await MainActor.run {
            let windows = UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
            guard let root = windows.first(where: \.isKeyWindow)?.rootViewController ?? windows.first?.rootViewController,
                  let webView = Self.webView(under: root.view) else {
                throw XCTSkip("the app's web view is not up")
            }
            return webView
        }
        // The page's Capacitor is up once the shell's script has run.
        try await untilTrue("the page's Capacitor is up", seconds: 30) {
            (try? await self.evaluate("typeof window.Capacitor?.registerPlugin")) as? String == "function"
        }
        let ready = try await evaluate(Self.recorder)
        XCTAssertEqual(ready as? String, "ready")
    }

    override func tearDown() {
        if let keptPushState {
            _ = PushStateStore.save(json: keptPushState)
        } else {
            PushStateStore.remove()
        }
        super.tearDown()
    }

    func testSnapshotsAndRoutesCrossTheBridge() async throws {
        let api = ProcessInfo.processInfo.environment["ASPEN_CALL_API"]
            .flatMap { $0.isEmpty ? nil : $0 } ?? "http://127.0.0.1:1"
        let origin = api

        // A fresh snapshot, for a page that slept: the engine's idle state as its JSON, numbered,
        // of this origin.
        let published = try await run("state", ["origin": origin])
        XCTAssertEqual(published["origin"] as? String, origin)
        let idle = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(try XCTUnwrap(published["state"] as? String).utf8)) as? [String: Any])
        XCTAssertEqual(idle["status"] as? String, "idle")
        XCTAssertEqual(idle["revision"] as? Int, 0)
        XCTAssertTrue(idle["channel"] is NSNull, "\(idle)")
        XCTAssertEqual((idle["grants"] as? [String: Any])?["speak"] as? Bool, false)
        XCTAssertEqual((idle["participants"] as? [Any])?.count, 0)
        for key in ["errorKind", "error", "retryAfterSeconds", "stateRefused", "endedReason", "session", "user"] {
            XCTAssertTrue(idle[key] is NSNull, "\(key) is null in \(idle)")
        }

        // The routes the device has, typed: an iPhone lists its receiver and its loudspeaker,
        // which plays on the simulator.
        let routes = try await run("routes", [:])
        XCTAssertEqual(routes["hasReceiver"] as? Bool, true)
        let listed = try XCTUnwrap(routes["routes"] as? [[String: Any]])
        XCTAssertTrue(listed.contains { $0["kind"] as? String == "receiver" }, "\(listed)")
        XCTAssertTrue(listed.contains { $0["kind"] as? String == "loudspeaker" && $0["inUse"] as? Bool == true }, "\(listed)")
        XCTAssertTrue(routes["chosen"] is NSNull)

        // Choosing moves the session's route within the call's category: the loudspeaker is
        // the current route's output, and the receiver, which the simulator has no port for,
        // leaves the simulator on its loudspeaker with the choice standing; a device's receiver
        // is on the checklist. Each choice reaches the page as a `routes` event.
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP])
        _ = try await run("setRoute", ["kind": "loudspeaker"])
        try await untilTrue("the loudspeaker choice reaches the page", seconds: 5) {
            (try? await self.latestRoutes())?["chosen"] as? String == "loudspeaker"
        }
        let onLoudspeaker = session.currentRoute.outputs.map(\.portType.rawValue)
        XCTAssertEqual(onLoudspeaker, [AVAudioSession.Port.builtInSpeaker.rawValue])
        _ = try await run("setRoute", ["kind": "receiver"])
        try await untilTrue("the receiver choice reaches the page", seconds: 5) {
            (try? await self.latestRoutes())?["chosen"] as? String == "receiver"
        }
        let onReceiver = session.currentRoute.outputs.map(\.portType.rawValue)
        let refused = try await run("setRoute", ["kind": "car"], expectingError: true)
        XCTAssertTrue(refused.contains("car"), refused)
        _ = try await run("clearRoute", [:])
        try await untilTrue("the choice is let go", seconds: 5) {
            (try? await self.latestRoutes())?["chosen"] is NSNull
        }

        // Muted outside a call is what the user last asked for, shown in the next snapshot.
        _ = try await run("setMuted", ["origin": origin, "muted": true])
        let mutedSnapshot = try await untilState("the mute reaches the page", seconds: 5) { $0["muted"] as? Bool == true }
        XCTAssertEqual(mutedSnapshot["status"] as? String, "idle")
        XCTAssertGreaterThan(try XCTUnwrap(mutedSnapshot["revision"] as? Int), 0)
        _ = try await run("setMuted", ["origin": origin, "muted": false])
        _ = try await untilState("the unmute reaches the page", seconds: 5) { $0["muted"] as? Bool == false }

        // A command the plugin cannot read is refused by name.
        let badSource = try await run("setUserVolume", ["origin": origin, "user": "u", "source": "video", "gain": 1], expectingError: true)
        XCTAssertTrue(badSource.contains("source"), badSource)
        let badReason = try await run("sessionEnded", ["origin": origin, "session": "s", "channel": "c", "reason": "bored"], expectingError: true)
        XCTAssertTrue(badReason.contains("reason"), badReason)
        let badOrigin = try await run("state", ["origin": "not a url"], expectingError: true)
        XCTAssertTrue(badOrigin.contains("origin"), badOrigin)

        // Web Audio in the page while the call's session is configured and active, as CallKit
        // would have it in a call (the simulator's CallKit holds none to activate it for): the
        // call's sounds stay in the page in this stage, and this says whether they can sound.
        let activation: String
        do {
            try session.setActive(true)
            activation = "active"
        } catch {
            activation = "not activated (\(error.localizedDescription))"
        }
        let audio = try await evaluateAsync("""
        const context = new AudioContext();
        await context.resume();
        const start = context.currentTime;
        await new Promise((resolve) => setTimeout(resolve, 300));
        const advanced = context.currentTime - start;
        const state = context.state;
        await context.close();
        return JSON.stringify({ state, advanced });
        """)
        try? session.setActive(false)
        let audioReport = "\((audio as? String) ?? String(describing: audio)) with the session \(activation)"

        // The join, against the deployment the check runs, as far as the simulator's CallKit
        // takes it: the start action joins the engine, whose `joining` snapshot reaches the
        // page, and the daemon's end leaves it, whose `idle` reaches the page too.
        guard let channel = ProcessInfo.processInfo.environment["ASPEN_CALL_CHANNEL"], !channel.isEmpty,
              let user = ProcessInfo.processInfo.environment["ASPEN_CALL_USER_ID"], !user.isEmpty,
              let refreshToken = ProcessInfo.processInfo.environment["ASPEN_CALL_REFRESH_TOKEN"], !refreshToken.isEmpty else {
            print("call-plugin-test: snapshots and routes crossed the bridge (loudspeaker route \(onLoudspeaker), receiver asked \(onReceiver)); web audio \(audioReport); no deployment given, so the join was not tried")
            throw XCTSkip("ASPEN_CALL_CHANNEL is not set")
        }
        keptPushState = PushStateStore.loadJSON()
        let account: [String: Any] = [
            "subscription": "call-plugin-test",
            "origin": api,
            "userId": user,
            "refreshToken": refreshToken,
            "sessionToken": "stale-session-token",
            "keys": ["privateKey": ["d": "AA"], "publicKey": "AA", "auth": "AA"],
        ]
        let state = try JSONSerialization.data(withJSONObject: ["version": 1, "accounts": [account]])
        XCTAssertTrue(PushStateStore.save(json: String(decoding: state, as: UTF8.self)))
        let before = try await states().count
        _ = try await run("join", ["origin": origin, "channel": channel, "name": "Bridge test"])
        let joining = try await untilState("the join's snapshot reaches the page", seconds: 15) {
            $0["status"] as? String == "joining" && $0["channel"] as? String == channel
        }
        XCTAssertEqual(joining["origin"] as? String, origin)
        let after = try await untilState("the call's end reaches the page", seconds: 20) { snapshot in
            let status = snapshot["status"] as? String
            return (snapshot["revision"] as? Int ?? 0) > (joining["revision"] as? Int ?? 0)
                && (status == "idle" || status == "connected" || status == "failed")
        }
        if after["status"] as? String == "connected" {
            // A CallKit that kept the call (a device's): left from the page.
            _ = try await run("leave", ["origin": origin])
            _ = try await untilState("the leave reaches the page", seconds: 10) { $0["status"] as? String == "idle" }
        }
        XCTAssertNotEqual(after["status"] as? String, "failed", "the join failed: \(after["error"] as? String ?? "")")
        let revisions = try await states().dropFirst(before).compactMap { $0["revision"] as? Int }
        XCTAssertEqual(revisions, revisions.sorted(), "snapshots arrive in revision order: \(revisions)")
        XCTAssertEqual(Set(revisions).count, revisions.count, "no snapshot twice: \(revisions)")
        let fresh = try await run("state", ["origin": origin])
        let freshState = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(try XCTUnwrap(fresh["state"] as? String).utf8)) as? [String: Any])
        XCTAssertEqual(freshState["revision"] as? Int, revisions.last, "a fresh read is the latest published")
        print("call-plugin-test: the page read the idle snapshot and the routes \(listed.map { "\($0["kind"] ?? "")\($0["inUse"] as? Bool == true ? " (in use)" : "")" }); chose the loudspeaker (route \(onLoudspeaker)) and the receiver (route \(onReceiver), the simulator having no receiver port); muted and unmuted outside the call; web audio with the call's session active: \(audioReport); joined \(channel) through the system, the page hearing \(revisions.count) snapshots in order (\(revisions.first ?? 0)…\(revisions.last ?? 0)), joining then \(after["status"] as? String ?? "?")")
    }

    // MARK: Helpers

    /// The first `WKWebView` under `view`.
    private static func webView(under view: UIView) -> WKWebView? {
        if let webView = view as? WKWebView {
            return webView
        }
        for subview in view.subviews {
            if let found = webView(under: subview) {
                return found
            }
        }
        return nil
    }

    /// The page could not run `script`, or answered with a value the web view cannot hand back.
    struct EvaluationFailed: Error, CustomStringConvertible {
        let script: String
        let error: Error

        var description: String {
            "the page could not run \(script.prefix(200)): \(error)"
        }
    }

    /// Runs `script` in the page, on the main queue, and answers its value.
    private func evaluate(_ script: String) async throws -> Any? {
        let webView = self.webView!
        return try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Any?, Error>) in
            DispatchQueue.main.async {
                webView.evaluateJavaScript(script) { value, error in
                    if let error {
                        continuation.resume(throwing: EvaluationFailed(script: script, error: error))
                    } else {
                        continuation.resume(returning: value)
                    }
                }
            }
        }
    }

    /// Runs `body`, an async function's body, in the page and answers what it returns once it
    /// settles; `evaluateJavaScript` cannot hand a Promise back.
    private func evaluateAsync(_ body: String) async throws -> Any? {
        let webView = self.webView!
        return try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Any?, Error>) in
            DispatchQueue.main.async {
                webView.callAsyncJavaScript(body, arguments: [:], in: nil, in: .page) { result in
                    switch result {
                    case .success(let value):
                        continuation.resume(returning: value)
                    case .failure(let error):
                        continuation.resume(throwing: EvaluationFailed(script: body, error: error))
                    }
                }
            }
        }
    }

    private var runs = 0

    /// Runs the plugin's `method` with `options` from the page and answers what it resolved
    /// with (an object, or an empty one for nothing), or, `expectingError`, the words it
    /// rejected with.
    @discardableResult
    private func run(_ method: String, _ options: [String: Any]) async throws -> [String: Any] {
        let outcome = try await outcome(of: method, options)
        if let error = outcome["error"] as? String {
            XCTFail("\(method) was refused: \(error)")
            throw Stopped()
        }
        return (outcome["ok"] as? [String: Any]) ?? [:]
    }

    private func run(_ method: String, _ options: [String: Any], expectingError: Bool) async throws -> String {
        let outcome = try await outcome(of: method, options)
        guard let error = outcome["error"] as? String else {
            XCTFail("\(method) was not refused: \(outcome)")
            throw Stopped()
        }
        return error
    }

    private func outcome(of method: String, _ options: [String: Any]) async throws -> [String: Any] {
        runs += 1
        let id = "run\(runs)"
        let json = String(decoding: try JSONSerialization.data(withJSONObject: options), as: UTF8.self)
        _ = try await evaluate("window.__aspenCallRun(\(quoted(id)), \(quoted(method)), \(json))")
        let deadline = Date().addingTimeInterval(20)
        while true {
            if let outcome = try await evaluate("window.__aspenCallTest.results[\(quoted(id))] ?? null") as? [String: Any] {
                return outcome
            }
            if Date() > deadline {
                XCTFail("\(method) did not settle within 20 s")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 50_000_000)
        }
    }

    private func states() async throws -> [[String: Any]] {
        let recorded = try await evaluate("window.__aspenCallTest.states")
        return try XCTUnwrap(recorded as? [[String: Any]])
    }

    private func latestRoutes() async throws -> [String: Any]? {
        try await evaluate("window.__aspenCallTest.routes.at(-1) ?? null") as? [String: Any]
    }

    /// The first snapshot the page recorded of which `holds`, waiting for it up to `seconds`.
    private func untilState(_ what: String, seconds: Double, holds: @escaping ([String: Any]) -> Bool) async throws -> [String: Any] {
        let deadline = Date().addingTimeInterval(seconds)
        while true {
            if let found = try await states().first(where: holds) {
                return found
            }
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s: \(try await states())")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    private func untilTrue(_ what: String, seconds: Double, holds: @escaping () async -> Bool) async throws {
        let deadline = Date().addingTimeInterval(seconds)
        while await !holds() {
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    struct Stopped: Error {}
}

/// `text` as a JavaScript string literal: the one element of a JSON array, without its brackets.
private func quoted(_ text: String) -> String {
    guard let data = try? JSONSerialization.data(withJSONObject: [text]) else {
        return "\"\""
    }
    return String(String(decoding: data, as: UTF8.self).dropFirst().dropLast())
}
