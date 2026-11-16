package com.aspen_chat.client.call

import android.Manifest
import android.content.Context
import android.graphics.Rect
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.MainActivity
import com.aspen_chat.client.push.PushStore
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.roundToInt
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The web view's bridge to the call engine, through the app's own page: `MainActivity` is
 * launched, which loads the web shell with `AspenCallPlugin` registered, and the test drives
 * `Capacitor.registerPlugin("AspenCall")` from the page's JavaScript, as the app's `NativeCall`
 * does. It reads a snapshot, joins a channel's call against a real deployment and watches the
 * engine's snapshots reach the page in revision order up to `connected`, moves the call to the
 * loudspeaker (and the earpiece, where the device has one) and watches the `routes` events say so
 * while Telecom's current endpoint changes, mutes and unmutes, drives the video: lists the
 * cameras, turns the camera on and sees its source in the snapshots, reports a hole for the
 * preview and hears its shape back while the surface view behind the page draws it in that hole,
 * moves to the other camera, watches a second participant's camera from a browser and hears its
 * shape, enters the picture-in-picture mode on it and hears `pip`, has malformed video commands
 * refused by name, turns the camera off, and leaves. `scripts/check_mobile_call.py --platform
 * android` runs it with `CallEngineTest` and gives it the same arguments; without them it is
 * skipped.
 */
@RunWith(AndroidJUnit4::class)
class AspenCallPluginTest {
    private lateinit var context: Context
    private lateinit var store: PushStore
    private var keptPushState: String? = null
    private var scenario: ActivityScenario<MainActivity>? = null
    private var runs = 0
    private var helper: String? = null

    private fun argument(name: String): String {
        val value = InstrumentationRegistry.getArguments().getString(name)
        assumeTrue("$name is not given", !value.isNullOrEmpty())
        return value!!
    }

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        store = PushStore(context)
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        automation.grantRuntimePermission(context.packageName, Manifest.permission.CAMERA)
        automation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
    }

    @After
    fun tearDown() {
        val kept = keptPushState
        if (kept != null) store.save(kept) else store.remove()
        helper?.let { http("POST", "$it/second-participant/leave") }
        scenario?.close()
    }

    @Test
    fun snapshotsAndRoutesCrossTheBridge() {
        val api = argument("ASPEN_CALL_API")
        val channel = argument("ASPEN_CALL_CHANNEL")
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")
        val helper = argument("ASPEN_CALL_HELPER")
        this.helper = helper

        // The account as the web view keeps it on sign-in, with a session token the deployment
        // refuses, so the first request goes through the renewal.
        keptPushState = store.load()
        val account = JSONObject()
            .put("subscription", "call-plugin-test")
            .put("origin", api)
            .put("userId", user)
            .put("refreshToken", refreshToken)
            .put("sessionToken", "stale-session-token")
            .put("keys", JSONObject().put("privateKey", JSONObject().put("d", "AA")).put("publicKey", "AA").put("auth", "AA"))
        store.save(JSONObject().put("version", 1).put("accounts", JSONArray().put(account)).toString())

        scenario = ActivityScenario.launch(MainActivity::class.java)
        awaitTrue("the page's Capacitor is up", 60) { evaluate("typeof window.Capacitor?.registerPlugin") == "\"function\"" }
        assertEquals("\"ready\"", evaluate(RECORDER))

        // A fresh snapshot, for a page that slept: the engine's idle state as its JSON, numbered,
        // of this origin.
        val published = run("state", JSONObject().put("origin", api))
        assertEquals(api, published.getString("origin"))
        val idle = JSONObject(published.getString("state"))
        assertEquals("idle", idle.getString("status"))
        assertEquals(0L, idle.getLong("revision"))
        assertTrue(idle.isNull("channel"))
        assertEquals(false, idle.getJSONObject("grants").getBoolean("speak"))
        assertEquals(0, idle.getJSONArray("participants").length())
        for (key in listOf("errorKind", "error", "retryAfterSeconds", "stateRefused", "endedReason", "session", "user")) {
            assertTrue("$key is null in $idle", idle.isNull(key))
        }

        // Outside a call Telecom lists no routes; the device says whether it has an earpiece.
        val before = run("routes", JSONObject())
        assertEquals(0, before.getJSONArray("routes").length())
        val hasReceiver = before.getBoolean("hasReceiver")
        assertTrue(before.isNull("chosen"))

        // A command the plugin cannot read is refused by name.
        assertTrue(refusal("setUserVolume", JSONObject().put("origin", api).put("user", "u").put("source", "video").put("gain", 1)).contains("source"))
        assertTrue(refusal("sessionEnded", JSONObject().put("origin", api).put("session", "s").put("channel", "c").put("reason", "bored")).contains("reason"))
        assertTrue(refusal("state", JSONObject().put("origin", "not a url")).contains("origin"))
        assertTrue(refusal("setRoute", JSONObject().put("kind", "car")).contains("kind"))

        // The join: the engine's snapshots reach the page as it connects, Telecom holding the call.
        val statesBefore = states().length()
        run("join", JSONObject().put("origin", api).put("channel", channel).put("name", "Bridge test"))
        val joining = untilState("the join's snapshot reaches the page", 15) { it.getString("status") == "joining" && it.optString("channel") == channel }
        assertEquals(api, joining.getString("origin"))
        val connected = untilState("the connected snapshot reaches the page", 40) { it.getString("status") == "connected" && it.optString("channel") == channel }
        assertEquals(user, connected.getString("user"))
        assertTrue(connected.getJSONObject("grants").getBoolean("speak"))
        assertTrue(connected.getLong("revision") > joining.getLong("revision"))

        // The routes are Telecom's for the call: moving to the loudspeaker shows in the `routes`
        // events and in Telecom's current endpoint; the earpiece again where there is one.
        val routes = untilRoutes("Telecom lists the call's routes", 15) { it.getJSONArray("routes").length() > 0 }
        val kinds = kindsOf(routes)
        assertTrue("the loudspeaker is a route: $kinds", kinds.contains("loudspeaker"))
        run("setRoute", JSONObject().put("kind", "loudspeaker"))
        untilRoutes("the call moves to the loudspeaker", 15) { current(it) == "loudspeaker" && it.optString("chosen") == "loudspeaker" }
        var moved = "the loudspeaker"
        if (hasReceiver && kinds.contains("receiver")) {
            run("setRoute", JSONObject().put("kind", "receiver"))
            untilRoutes("the call moves back to the earpiece", 15) { current(it) == "receiver" }
            moved += " and back to the receiver"
        }
        run("clearRoute", JSONObject())
        untilRoutes("the choice is let go", 5) { it.isNull("chosen") }

        // Muting shows at once; unmuting waits for the server's word.
        run("setMuted", JSONObject().put("origin", api).put("muted", true))
        untilState("the mute reaches the page", 10) { it.getBoolean("muted") }
        run("setMuted", JSONObject().put("origin", api).put("muted", false))
        untilState("the unmute reaches the page", 15) { it.getLong("revision") > connected.getLong("revision") && !it.getBoolean("muted") && it.getString("status") == "connected" }

        // Video through the bridge. The cameras, as Camera2 lists the emulator's, each facing a way.
        val listedCameras = run("cameras", JSONObject()).getJSONArray("cameras")
        val cameras = (0 until listedCameras.length()).map { listedCameras.getJSONObject(it) }
        assertTrue("the emulator has cameras: $cameras", cameras.isNotEmpty())
        val facings = cameras.map { it.optString("facing") }
        assertTrue("each faces a way: $facings", facings.all { it == "front" || it == "back" })

        // The camera on from the page: the plugin opens the front camera (or the first, where the
        // phone has none) and the engine sends it, which the snapshot shows as a source; the page
        // then reports a hole for its preview, mirrored, and hears the picture's shape back, while
        // the surface view behind the page draws the frames in that hole, in device pixels of the
        // page's CSS pixels.
        val opened = run("startCamera", JSONObject().put("origin", api).put("device", JSONObject.NULL))
        val expectedCamera = cameras.firstOrNull { it.optString("facing") == "front" } ?: cameras.first()
        assertEquals(expectedCamera.getString("id"), opened.getString("device"))
        assertEquals(expectedCamera.getString("facing"), opened.getString("facing"))
        val sendingCamera = untilState("the camera is a source", 20) { snapshot ->
            sourcesOf(snapshot).any { it.getString("source") == "camera" && it.getBoolean("live") }
        }
        val selfSurface = sourcesOf(sendingCamera).first { it.getString("source") == "camera" }.getString("track")
        val selfTile = tile("self", selfSurface, 8, 16, 120, 160, mirrored = true, cover = true, pictureInPicture = false)
        run("setTiles", JSONObject().put("tiles", JSONArray().put(selfTile)))
        val selfShape = untilRecorded("the preview's shape reaches the page", "shapes", 15) { it.getString("surface") == selfSurface }
        assertTrue("a picture: $selfShape", selfShape.getInt("width") > 0 && selfShape.getInt("height") > 0)
        val density = context.resources.displayMetrics.density
        val expectedBounds = Rect((8 * density).roundToInt(), (16 * density).roundToInt(), (128 * density).roundToInt(), (176 * density).roundToInt())
        awaitTrue("the preview is laid out in its hole", 10) { onActivity { CallEngine.video.bounds("self") } == expectedBounds }
        awaitTrue("the preview is drawn", 15) { onActivity { CallEngine.video.renderer("self")?.firstFrameRendered } == true }
        val other = cameras.firstOrNull { it.getString("id") != opened.getString("device") }
        val cameraMoved = if (other != null) {
            val switched = run("switchCamera", JSONObject().put("origin", api).put("device", other.getString("id")))
            assertEquals(other.getString("facing"), switched.getString("facing"))
            "moved to ${other.getString("id")} (${other.getString("facing")})"
        } else {
            "no other camera to move to"
        }

        // A second participant from a browser, showing video and sending Chromium's fake camera:
        // the page asks the engine to watch it, the snapshot shows the video's track, the page
        // reports a hole for it marked for the picture in picture, hears its shape, and the
        // activity enters the picture-in-picture mode on it, which the page hears as `pip`.
        val bob = JSONObject(http("POST", "$helper/second-participant")).getString("user")
        http("POST", "$helper/second-participant/show-video")
        http("POST", "$helper/second-participant/camera/start")
        run("setWatched", JSONObject().put("origin", api).put("video", JSONArray().put(JSONObject().put("user", bob).put("source", "camera"))))
        val watching = untilState("the browser's camera is received", 30) { snapshot ->
            videosOf(snapshot).any { it.getString("user") == bob && it.getString("source") == "camera" }
        }
        val remoteSurface = videosOf(watching).first { it.getString("user") == bob }.getString("track")
        val remoteTile = tile("remote", remoteSurface, 0, 200, 320, 240, mirrored = false, cover = false, pictureInPicture = true)
        run("setTiles", JSONObject().put("tiles", JSONArray().put(selfTile).put(remoteTile)))
        val remoteShape = untilRecorded("the browser's camera's shape reaches the page", "shapes", 20) { it.getString("surface") == remoteSurface }
        assertTrue("a picture: $remoteShape", remoteShape.getInt("width") > 0)
        assertEquals(0, remoteShape.getInt("rotation"))
        awaitTrue("the browser's camera is drawn", 15) { onActivity { CallEngine.video.renderer("remote")?.firstFrameRendered } == true }
        val entered = onActivity { activity -> (activity.bridge.getPlugin("AspenCall").instance as AspenCallPlugin).enterPictureInPicture() }
        assertEquals("the activity enters the picture-in-picture mode", true, entered)
        untilRecorded("the page hears the picture in picture start", "pips", 15) { it.getBoolean("active") }
        assertEquals(true, onActivity { it.isInPictureInPictureMode })
        // Bringing the activity to the front again leaves the mode.
        InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("am start -n ${context.packageName}/com.aspen_chat.client.MainActivity").close()
        untilRecorded("the page hears the picture in picture stop", "pips", 15) { !it.getBoolean("active") }
        awaitTrue("the browser decodes the camera", 20) { browserVideoFrames(helper) > 10 }

        // Malformed video commands are refused by name.
        assertTrue(refusal("setWatched", JSONObject().put("origin", api).put("video", JSONArray().put(JSONObject().put("user", bob).put("source", "microphone")))).contains("source"))
        assertTrue(refusal("setTiles", JSONObject().put("tiles", JSONArray().put(JSONObject().put("tile", "t").put("surface", "s")))).contains("rect"))

        // The camera off from the page: the source leaves the snapshot and the capture stops.
        run("stopCamera", JSONObject().put("origin", api))
        untilState("the camera leaves the sources", 15) { snapshot ->
            snapshot.getLong("revision") > sendingCamera.getLong("revision") && sourcesOf(snapshot).none { it.getString("source") == "camera" }
        }
        assertTrue(!CallEngine.camera(context).isOn)
        run("setTiles", JSONObject().put("tiles", JSONArray()))

        // Leaving: the page sees the call idle, in order, and a fresh read is the latest published.
        run("leave", JSONObject().put("origin", api))
        val left = untilState("the leave reaches the page", 15) { it.getLong("revision") > connected.getLong("revision") && it.getString("status") == "idle" }
        assertTrue(left.isNull("session"))
        val all = states()
        val revisions = (statesBefore until all.length()).map { all.getJSONObject(it).getLong("revision") }
        assertEquals("snapshots arrive in revision order: $revisions", revisions.sorted(), revisions)
        assertEquals("no snapshot twice: $revisions", revisions.toSet().size, revisions.size)
        val fresh = JSONObject(run("state", JSONObject().put("origin", api)).getString("state"))
        assertEquals("a fresh read is the latest published", revisions.last(), fresh.getLong("revision"))
        assertNotEquals("the renewed token is kept", "stale-session-token", store.accountAt(api)?.optString("sessionToken"))
        untilRoutes("the routes empty after the call", 15) { it.getJSONArray("routes").length() == 0 }
        println(
            "call-plugin-test: the page read the idle snapshot (receiver $hasReceiver), joined $channel through the plugin and heard " +
                "${revisions.size} snapshots in order (${revisions.first()}…${revisions.last()}): joining, connected as $user, muted, unmuted, idle; " +
                "routes $kinds, moved to $moved as the routes events said; listed $facings cameras, turned the camera on (${opened.getString("device")}, " +
                "facing ${opened.getString("facing")}), heard its preview's shape ${selfShape.getInt("width")}×${selfShape.getInt("height")} at ${selfShape.getInt("rotation")}° " +
                "for a hole laid out at $expectedBounds and drawn, $cameraMoved; watched $bob's camera (shape ${remoteShape.getInt("width")}×${remoteShape.getInt("height")}), " +
                "entered the picture in picture on it and left it, had bad video commands refused, turned the camera off; a fresh read matched the latest",
        )
    }

    /**
     * How far the native picture trails the page while a hole moves, for judging by eye: the page
     * hides its own content, draws a red-bordered box, and moves it across the screen and back,
     * three passes of 200 CSS pixels in 800 ms each (about four pixels a frame), reporting the
     * box's inside as a hole every animation frame as the app's `nativeVideo.ts` does, with the
     * emulator's camera drawn in it; `scripts/check_mobile_call.py --record-trailing` records the
     * emulator's screen around it, and the recording is frame-stepped (`android-builds.md`, Video).
     * Runs only with the argument `ASPEN_CALL_TRAILING`, since it is a recording, not a check.
     */
    @Test
    fun pictureFollowsAMovingHole() {
        assumeTrue("ASPEN_CALL_TRAILING is not given", InstrumentationRegistry.getArguments().getString("ASPEN_CALL_TRAILING") == "1")
        scenario = ActivityScenario.launch(MainActivity::class.java)
        awaitTrue("the page's Capacitor is up", 60) { evaluate("typeof window.Capacitor?.registerPlugin") == "\"function\"" }
        val camera = CallEngine.camera(context)
        var surface: String? = null
        val latch = CountDownLatch(1)
        scenario!!.onActivity {
            camera.start(null, "trailing") { result ->
                surface = result.getOrNull()?.first
                latch.countDown()
            }
        }
        assertTrue("the camera opens", latch.await(20, TimeUnit.SECONDS))
        val track = surface ?: throw AssertionError("no camera track")
        try {
            // `evaluateJavascript` answers the expression's value at once, so the page records when it is done.
            evaluate("window.__aspenTrailing = null; (" + TRAILING_SCRIPT.replace("SURFACE", JSONObject.quote(track)) + ").then((v) => { window.__aspenTrailing = v; }); 'started'")
            awaitTrue("the passes end", 30) { evaluate("window.__aspenTrailing") == "\"done\"" }
            println("call-plugin-test: recorded the picture following a moving hole")
        } finally {
            onActivity { camera.stop() }
        }
    }

    // --- Helpers --------------------------------------------------------------------------------

    /** Runs `script` in the page, on the main thread, and answers its result as the web view writes it (JSON). */
    private fun evaluate(script: String): String {
        val latch = CountDownLatch(1)
        var result = "null"
        scenario!!.onActivity { activity ->
            activity.bridge.webView.evaluateJavascript(script) { value ->
                result = value ?: "null"
                latch.countDown()
            }
        }
        assertTrue("the page answered", latch.await(20, TimeUnit.SECONDS))
        return result
    }

    /** What the plugin's `method` resolved with (an object, or an empty one for nothing). */
    private fun run(method: String, options: JSONObject): JSONObject {
        val outcome = outcome(method, options)
        if (outcome.has("error")) {
            fail("$method was refused: ${outcome.getString("error")}")
        }
        return outcome.optJSONObject("ok") ?: JSONObject()
    }

    /** The words the plugin refused `method` with. */
    private fun refusal(method: String, options: JSONObject): String {
        val outcome = outcome(method, options)
        if (!outcome.has("error")) {
            fail("$method was not refused: $outcome")
        }
        return outcome.getString("error")
    }

    private fun outcome(method: String, options: JSONObject): JSONObject {
        runs += 1
        val id = "run$runs"
        evaluate("window.__aspenCallRun(${JSONObject.quote(id)}, ${JSONObject.quote(method)}, $options)")
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20)
        while (true) {
            val answer = evaluate("window.__aspenCallTest.results[${JSONObject.quote(id)}] ?? null")
            if (answer != "null") {
                return JSONObject(answer)
            }
            if (System.nanoTime() > deadline) {
                fail("$method did not settle within 20 s")
            }
            Thread.sleep(50)
        }
    }

    private fun states(): JSONArray = JSONArray(evaluate("window.__aspenCallTest.states"))

    private fun sourcesOf(snapshot: JSONObject): List<JSONObject> {
        val sources = snapshot.optJSONArray("sources") ?: JSONArray()
        return (0 until sources.length()).map { sources.getJSONObject(it) }
    }

    private fun videosOf(snapshot: JSONObject): List<JSONObject> {
        val videos = snapshot.optJSONArray("videos") ?: JSONArray()
        return (0 until videos.length()).map { videos.getJSONObject(it) }
    }

    /** A hole as the page reports one (`TileReport`), in CSS pixels. */
    private fun tile(id: String, surface: String, x: Int, y: Int, width: Int, height: Int, mirrored: Boolean, cover: Boolean, pictureInPicture: Boolean): JSONObject =
        JSONObject()
            .put("tile", id)
            .put("surface", surface)
            .put("rect", JSONObject().put("x", x).put("y", y).put("width", width).put("height", height))
            .put("visible", true)
            .put("mirrored", mirrored)
            .put("cover", cover)
            .put("fullScreen", false)
            .put("pictureInPicture", pictureInPicture)

    /** `read` on the activity's main thread, waiting for its answer. */
    private fun <T> onActivity(read: (MainActivity) -> T): T? {
        var answer: T? = null
        val latch = CountDownLatch(1)
        scenario!!.onActivity { activity ->
            answer = read(activity)
            latch.countDown()
        }
        assertTrue("the activity answered", latch.await(10, TimeUnit.SECONDS))
        return answer
    }

    /** The first event the page recorded in `list` (`shapes`, `pips`) of which `holds`, waiting up to `seconds`. */
    private fun untilRecorded(what: String, list: String, seconds: Long, holds: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (true) {
            val all = JSONArray(evaluate("window.__aspenCallTest.$list"))
            for (i in 0 until all.length()) {
                val entry = all.getJSONObject(i)
                if (holds(entry)) {
                    return entry
                }
            }
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s: $all")
            }
            Thread.sleep(100)
        }
    }

    /** The body of `url`'s answer, which must be `200`. */
    private fun http(method: String, url: String): String {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 60_000
        connection.readTimeout = 60_000
        val status = connection.responseCode
        val body = (if (status < 400) connection.inputStream else connection.errorStream)?.bufferedReader()?.readText() ?: ""
        assertEquals("$method $url: $body", 200, status)
        return body
    }

    /** How many video frames the browser's connection has decoded, from the helper's report of it. */
    private fun browserVideoFrames(helper: String): Int {
        val streams = JSONObject(http("GET", "$helper/second-participant/report")).getJSONArray("streams")
        var frames = 0
        for (i in 0 until streams.length()) {
            val stream = streams.getJSONObject(i)
            if (stream.optString("direction") == "inbound" && stream.optString("kind") == "video") {
                frames += stream.optInt("framesDecoded")
            }
        }
        return frames
    }

    private fun latestRoutes(): JSONObject? {
        val answer = evaluate("window.__aspenCallTest.routes.at(-1) ?? null")
        return if (answer == "null") null else JSONObject(answer)
    }

    private fun kindsOf(routes: JSONObject): List<String> {
        val listed = routes.getJSONArray("routes")
        return (0 until listed.length()).map { listed.getJSONObject(it).getString("kind") }
    }

    /** The kind of the route in use, or `null` when none is. */
    private fun current(routes: JSONObject): String? {
        val listed = routes.getJSONArray("routes")
        return (0 until listed.length()).map { listed.getJSONObject(it) }.firstOrNull { it.getBoolean("inUse") }?.getString("kind")
    }

    /** The first snapshot the page recorded of which `holds`, waiting for it up to `seconds`. */
    private fun untilState(what: String, seconds: Long, holds: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (true) {
            val all = states()
            for (i in 0 until all.length()) {
                val snapshot = all.getJSONObject(i)
                if (holds(snapshot)) {
                    return snapshot
                }
            }
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s: $all")
            }
            Thread.sleep(100)
        }
    }

    /** The latest routes the page recorded once `holds` of them, waiting up to `seconds`. */
    private fun untilRoutes(what: String, seconds: Long, holds: (JSONObject) -> Boolean): JSONObject {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (true) {
            val latest = latestRoutes()
            if (latest != null && holds(latest)) {
                return latest
            }
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s: $latest")
            }
            Thread.sleep(100)
        }
    }

    private fun awaitTrue(what: String, seconds: Long, holds: () -> Boolean) {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (!holds()) {
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s")
            }
            Thread.sleep(200)
        }
    }

    private companion object {
        /** The page's side of `pictureFollowsAMovingHole`; `SURFACE` is replaced by the camera's track. */
        val TRAILING_SCRIPT = """

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
          box.style.left = `${'$'}{20 + 200 * (forward ? within : 1 - within)}px`;
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
        """.trimIndent()

        /**
         * What the page records under `window.__aspenCallTest`: the plugin's events, each `state`
         * one read as the engine's JSON with its origin beside, and each command's outcome by name.
         */
        val RECORDER = """
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
        """.trimIndent()
    }
}
