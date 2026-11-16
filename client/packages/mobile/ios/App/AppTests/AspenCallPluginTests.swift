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
/// sees too), mutes outside a call, reports whether the page's Web Audio runs while the call's
/// audio session is active, and then, the engine the plugin made joined directly (the simulator's
/// CallKit holds no call), drives the video from the page: lists the cameras, turns the camera
/// on (in software, `SyntheticVideoCapturer`) and sees its source in the snapshots, reports a
/// hole for the preview and hears its shape back, moves to the other camera, watches a second
/// participant's camera from a browser and hears its shape, has the system's picture in picture
/// started on it and hears `pip`, has malformed video commands refused by name, and turns the
/// camera off. `scripts/check_mobile_call.py` runs it with `CallEngineTests` and gives it the
/// same environment; without it, the join is skipped.
final class AspenCallPluginTests: XCTestCase {
    private var keptPushState: String?
    private var webView: WKWebView!
    private var bridgeViewController: MainViewController!
    private var helper: URL?

    /// What the page recorded under `window.__aspenCallTest`: the plugin's events, each `state`
    /// one read as the engine's JSON with its origin beside, and each command's outcome by name.
    private static let recorder = """
    (() => {
      const plugin = window.Capacitor.registerPlugin("AspenCall");
      const record = { states: [], routes: [], shapes: [], pips: [], results: {} };
      window.__aspenCallTest = record;
      plugin.addListener("state", (published) => record.states.push({ ...JSON.parse(published.state), origin: published.origin }));
      plugin.addListener("routes", (routes) => record.routes.push(routes));
      plugin.addListener("shape", (shape) => record.shapes.push(shape));
      plugin.addListener("pip", (pip) => record.pips.push(pip));
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
        // The simulator's own audio unit aborts the process when initialized, so the engines the
        // plugin makes run on an audio device in software, the camera on a capturer in software,
        // the simulator having none, and the call's sounds on players in software, since the
        // simulator's audio server starts no output for a test process.
        CallEngine.peerConnections = CallPeerConnections(tracks: CallEngine.tracks, audioDevice: ToneAudioDevice())
        CallEngine.camera.makeCapturer = { SyntheticVideoCapturer() }
        CallSounds.makePlayer = { try SoftwareSoundPlayer(url: $0) }
        (webView, bridgeViewController) = try await MainActor.run {
            let windows = UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
            guard let root = windows.first(where: \.isKeyWindow)?.rootViewController as? RootViewController
                    ?? windows.first?.rootViewController as? RootViewController,
                  let webView = Self.webView(under: root.view) else {
                throw XCTSkip("the app's web view is not up")
            }
            return (webView, root.web)
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
        if let helper {
            var request = URLRequest(url: helper.appendingPathComponent("second-participant/leave"))
            request.httpMethod = "POST"
            let done = expectation(description: "the browser is told to leave")
            URLSession.shared.dataTask(with: request) { _, _, _ in done.fulfill() }.resume()
            wait(for: [done], timeout: 10)
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

        // Video through the bridge. The simulator's CallKit holds no call, so the engine the
        // plugin made for the page is joined directly, as the engine's own proof does, the test
        // standing in for `didActivate`; the page then drives the camera, the holes, and the
        // watching, and hears the shapes and the picture in picture back.
        guard let helperAddress = ProcessInfo.processInfo.environment["ASPEN_CALL_HELPER"], !helperAddress.isEmpty else {
            throw XCTSkip("ASPEN_CALL_HELPER is not set")
        }
        let helper = try XCTUnwrap(URL(string: helperAddress))
        self.helper = helper
        let pluginFound = await MainActor.run { bridgeViewController.bridge?.plugin(withName: "AspenCall") as? AspenCallPlugin }
        let plugin = try XCTUnwrap(pluginFound)
        let engine = await MainActor.run { plugin.engine(at: origin) }
        try engine.audio.configureForCall()
        engine.audio.activated(session)
        defer { engine.audio.deactivated(session) }
        engine.engine.setMicrophone(track: engine.audio.openMicrophone())
        let directFrom = try await states().count
        engine.engine.join(channel: channel)
        defer { engine.engine.leave() }
        let connected = try await untilState("the direct join connects", seconds: 40) { snapshot in
            snapshot["status"] as? String == "connected" && (snapshot["revision"] as? Int ?? 0) > (revisions.last ?? 0)
        }
        XCTAssertEqual((connected["grants"] as? [String: Any])?["camera"] as? Bool, true, "the channel lets the user use a camera")

        // The cameras, as the system (here the capturer in software) lists them, each facing a way.
        let listedCameras = try await run("cameras", [:])
        let cameras = try XCTUnwrap(listedCameras["cameras"] as? [[String: Any]])
        XCTAssertEqual(cameras.map { $0["id"] as? String }, [SyntheticVideoCapturer.front.id, SyntheticVideoCapturer.back.id])
        XCTAssertEqual(cameras.map { $0["facing"] as? String }, ["front", "back"])

        // The camera on from the page: the plugin opens the default (front) camera and the engine
        // sends it, which the snapshot shows as a source; the page then reports a hole for its
        // preview, mirrored, and hears the picture's shape back, upright: the capturer's 320 by 240
        // frames at 90 degrees, which the track's source turns itself once the encoder wants them
        // turned (no orientation extension being negotiated), so the preview is 240 by 320 either
        // way; the preview's layer lies in the hole.
        let opened = try await run("startCamera", ["origin": origin, "device": NSNull()])
        XCTAssertEqual(opened["device"] as? String, SyntheticVideoCapturer.front.id)
        XCTAssertEqual(opened["facing"] as? String, "front")
        let sendingCamera = try await untilState("the camera is a source", seconds: 20) { snapshot in
            ((snapshot["sources"] as? [[String: Any]]) ?? []).contains { $0["source"] as? String == "camera" && $0["live"] as? Bool == true }
        }
        let selfSurface = try XCTUnwrap(((sendingCamera["sources"] as? [[String: Any]]) ?? []).first { $0["source"] as? String == "camera" }?["track"] as? String)
        let selfBox: [String: Any] = ["x": 8, "y": 16, "width": 120, "height": 160]
        let selfTile: [String: Any] = [
            "tile": "self", "surface": selfSurface, "rect": selfBox, "visible": true, "mirrored": true, "cover": true,
            "fullScreen": false, "pictureInPicture": false,
        ]
        _ = try await run("setTiles", ["tiles": [selfTile]])
        let selfShape = try await untilShape("the preview's shape reaches the page", seconds: 15) { $0["surface"] as? String == selfSurface }
        XCTAssertEqual(selfShape["width"] as? Int, 240)
        XCTAssertEqual(selfShape["height"] as? Int, 320)
        let selfDrawnAt = await MainActor.run { CallEngine.video.frame(ofTile: "self") }
        XCTAssertEqual(selfDrawnAt, CGRect(x: 8, y: 16, width: 120, height: 160))
        let switched = try await run("switchCamera", ["origin": origin, "device": SyntheticVideoCapturer.back.id])
        XCTAssertEqual(switched["device"] as? String, SyntheticVideoCapturer.back.id)
        XCTAssertEqual(switched["facing"] as? String, "back")

        // A second participant from a browser, showing video and sending Chromium's fake camera:
        // the page asks the engine to watch it, the snapshot shows the video's track, the page
        // reports a hole for it marked for the picture in picture, hears its shape, and hears the
        // system start and stop the picture in picture on it.
        let second = try await post(helper.appendingPathComponent("second-participant"))
        let bob = try XCTUnwrap(second["user"] as? String)
        _ = try await post(helper.appendingPathComponent("second-participant/show-video"))
        _ = try await post(helper.appendingPathComponent("second-participant/camera/start"))
        _ = try await run("setWatched", ["origin": origin, "video": [["user": bob, "source": "camera"]]])
        let watching = try await untilState("the browser's camera is received", seconds: 30) { snapshot in
            ((snapshot["videos"] as? [[String: Any]]) ?? []).contains { $0["user"] as? String == bob && $0["source"] as? String == "camera" }
        }
        let remoteSurface = try XCTUnwrap(((watching["videos"] as? [[String: Any]]) ?? []).first { $0["user"] as? String == bob }?["track"] as? String)
        let remoteTile: [String: Any] = [
            "tile": "remote", "surface": remoteSurface, "rect": ["x": 0, "y": 200, "width": 320, "height": 240], "visible": true,
            "mirrored": false, "cover": false, "fullScreen": false, "pictureInPicture": true,
        ]
        _ = try await run("setTiles", ["tiles": [selfTile, remoteTile]])
        let remoteShape = try await untilShape("the browser's camera's shape reaches the page", seconds: 20) { $0["surface"] as? String == remoteSurface }
        XCTAssertGreaterThan(remoteShape["width"] as? Int ?? 0, 0)
        XCTAssertEqual(remoteShape["rotation"] as? Int, 0)
        // The picture in picture needs a device: the simulator's AVKit says it is not supported.
        let pip = CallEngine.video.pictureInPicture
        let pictureInPicture: String
        if CallPictureInPicture.isSupported {
            try await untilTrue("the picture in picture is possible", seconds: 10) { await MainActor.run { pip.isPossible } }
            await MainActor.run { pip.start() }
            _ = try await untilPip("the page hears the picture in picture start", seconds: 15) { $0["active"] as? Bool == true }
            await MainActor.run { pip.stop() }
            _ = try await untilPip("the page hears the picture in picture stop", seconds: 15) { $0["active"] as? Bool == false }
            pictureInPicture = "heard the picture in picture start and stop"
        } else {
            pictureInPicture = "the system here supports no picture in picture (the simulator), so that is the device's to show"
        }
        try await untilTrue("the browser decodes the camera", seconds: 20) {
            (try? await self.browserVideoFrames(helper)) ?? 0 > 10
        }

        // Malformed video commands are refused by name.
        let badWatch = try await run("setWatched", ["origin": origin, "video": [["user": bob, "source": "microphone"]]], expectingError: true)
        XCTAssertTrue(badWatch.contains("source"), badWatch)
        let badTile = try await run("setTiles", ["tiles": [["tile": "t", "surface": "s"]]], expectingError: true)
        XCTAssertTrue(badTile.contains("rect"), badTile)

        // The camera off from the page: the source leaves the snapshot and the capture stops.
        _ = try await run("stopCamera", ["origin": origin])
        _ = try await untilState("the camera leaves the sources", seconds: 15) { snapshot in
            (snapshot["revision"] as? Int ?? 0) > (sendingCamera["revision"] as? Int ?? 0)
                && !((snapshot["sources"] as? [[String: Any]]) ?? []).contains { $0["source"] as? String == "camera" }
        }
        XCTAssertFalse(CallEngine.camera.isOn)
        _ = try await run("setTiles", ["tiles": []])
        _ = try await run("leave", ["origin": origin])
        _ = try await untilState("the leave reaches the page", seconds: 15) { snapshot in
            (snapshot["revision"] as? Int ?? 0) > (watching["revision"] as? Int ?? 0) && snapshot["status"] as? String == "idle"
        }
        let direct = try await states().dropFirst(directFrom).compactMap { $0["revision"] as? Int }
        XCTAssertEqual(direct, direct.sorted(), "snapshots arrive in revision order: \(direct)")
        print("call-plugin-test: the page read the idle snapshot and the routes \(listed.map { "\($0["kind"] ?? "")\($0["inUse"] as? Bool == true ? " (in use)" : "")" }); chose the loudspeaker (route \(onLoudspeaker)) and the receiver (route \(onReceiver), the simulator having no receiver port); muted and unmuted outside the call; web audio with the call's session active: \(audioReport); joined \(channel) through the system, the page hearing \(revisions.count) snapshots in order (\(revisions.first ?? 0)…\(revisions.last ?? 0)), joining then \(after["status"] as? String ?? "?"); then, the engine joined directly, listed \(cameras.count) cameras, turned the camera on (\(opened["device"] as? String ?? "?"), facing \(opened["facing"] as? String ?? "?")), heard its preview's shape \(selfShape["width"] ?? 0)×\(selfShape["height"] ?? 0) for a hole at \(selfDrawnAt.map { "\($0)" } ?? "?"), moved to \(switched["device"] as? String ?? "?"), watched \(bob)'s camera (shape \(remoteShape["width"] ?? 0)×\(remoteShape["height"] ?? 0)), \(pictureInPicture), had bad video commands refused, turned the camera off, and left")
    }

    /// How far the native picture trails the page while a hole moves, for judging by eye: the page
    /// hides its own content, draws a red-bordered box, and moves it across the screen and back,
    /// three passes of 200 points in 800 ms each (about four points a frame), reporting the box's
    /// inside as a hole every animation frame as the app's `nativeVideo.ts` does, with the camera
    /// (in software) drawn in it; `scripts/check_mobile_call.py --record-trailing` records the
    /// simulator's screen around it, and the recording is frame-stepped (`ios-app.md`, Video).
    /// Runs only with `ASPEN_CALL_TRAILING` set, since it is a recording, not a check.
    func testPictureFollowsAMovingHole() async throws {
        guard ProcessInfo.processInfo.environment["ASPEN_CALL_TRAILING"] == "1" else {
            throw XCTSkip("ASPEN_CALL_TRAILING is not set")
        }
        let camera = CallEngine.camera
        let surface: TrackId = try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.main.async {
                camera.start(device: nil, origin: "trailing") { result in
                    switch result {
                    case .success(let opened): continuation.resume(returning: opened.track)
                    case .failure(let failure): continuation.resume(throwing: failure)
                    }
                }
            }
        }
        defer { DispatchQueue.main.async { camera.stop() } }
        let script = Self.trailingScript.replacingOccurrences(of: "SURFACE", with: quoted(surface))
        let done = try await evaluateAsync("return await " + script)
        XCTAssertEqual(done as? String, "done")
        await MainActor.run { CallEngine.video.apply([]) }
        print("call-plugin-test: recorded the picture following a moving hole")
    }

    private static let trailingScript = """

    (async () => {
      const plugin = window.Capacitor.registerPlugin("AspenCall");
      const root = document.getElementById("root");
      if (root) root.style.display = "none";
      for (const node of [document.documentElement, document.body]) node.style.background = "transparent";
      const box = document.createElement("div");
      box.id = "aspen-trailing";
      Object.assign(box.style, { position: "fixed", top: "300px", left: "20px", width: "160px", height: "120px", boxSizing: "border-box", border: "2px solid #ff0000", zIndex: "99999", pointerEvents: "none" });
      document.body.append(box);
      const report = () => {
        const r = box.getBoundingClientRect();
        void plugin.setTiles({ tiles: [{ tile: "trailing", surface: SURFACE, rect: { x: r.left + 2, y: r.top + 2, width: r.width - 4, height: r.height - 4 }, visible: true, mirrored: false, cover: true, fullScreen: false, pictureInPicture: false }] });
      };
      report();
      await new Promise((resolve) => setTimeout(resolve, 1500));
      // Three passes across 200 px, each 800 ms, about four pixels a frame at sixty frames a second.
      const start = performance.now();
      const passes = 3, pass = 800;
      await new Promise((resolve) => {
        const step = (now) => {
          const t = Math.min(passes, (now - start) / pass);
          const within = t - Math.floor(t);
          const forward = Math.floor(t) % 2 === 0;
          box.style.left = `${20 + 200 * (forward ? within : 1 - within)}px`;
          report();
          if (t < passes) requestAnimationFrame(step); else resolve();
        };
        requestAnimationFrame(step);
      });
      await new Promise((resolve) => setTimeout(resolve, 500));
      box.remove();
      if (root) root.style.display = "";
      for (const node of [document.documentElement, document.body]) node.style.background = "";
      await plugin.setTiles({ tiles: [] });
      return "done";
    })()
    """

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

    /// The first `shape` event the page recorded of which `holds`, waiting for it up to `seconds`.
    private func untilShape(_ what: String, seconds: Double, holds: @escaping ([String: Any]) -> Bool) async throws -> [String: Any] {
        try await untilRecorded(what, in: "shapes", seconds: seconds, holds: holds)
    }

    /// The first `pip` event the page recorded of which `holds`, waiting for it up to `seconds`.
    private func untilPip(_ what: String, seconds: Double, holds: @escaping ([String: Any]) -> Bool) async throws -> [String: Any] {
        try await untilRecorded(what, in: "pips", seconds: seconds, holds: holds)
    }

    private func untilRecorded(_ what: String, in list: String, seconds: Double, holds: @escaping ([String: Any]) -> Bool) async throws -> [String: Any] {
        let deadline = Date().addingTimeInterval(seconds)
        while true {
            let evaluated = try await evaluate("window.__aspenCallTest.\(list)")
            let recorded = try XCTUnwrap(evaluated as? [[String: Any]])
            if let found = recorded.first(where: holds) {
                return found
            }
            if Date() > deadline {
                XCTFail("\(what) did not happen within \(seconds) s: \(recorded)")
                throw Stopped()
            }
            try await Task.sleep(nanoseconds: 100_000_000)
        }
    }

    private func post(_ url: URL) async throws -> [String: Any] {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 60
        let (data, response) = try await URLSession.shared.data(for: request)
        XCTAssertEqual((response as? HTTPURLResponse)?.statusCode, 200, String(decoding: data, as: UTF8.self))
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    /// How many video frames the browser's connection has decoded, from the helper's report of it.
    private func browserVideoFrames(_ helper: URL) async throws -> Int {
        let (data, _) = try await URLSession.shared.data(from: helper.appendingPathComponent("second-participant/report"))
        let report = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let streams = try XCTUnwrap(report["streams"] as? [[String: Any]])
        return streams
            .filter { $0["direction"] as? String == "inbound" && $0["kind"] as? String == "video" }
            .reduce(0) { $0 + (($1["framesDecoded"] as? NSNumber)?.intValue ?? 0) }
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
