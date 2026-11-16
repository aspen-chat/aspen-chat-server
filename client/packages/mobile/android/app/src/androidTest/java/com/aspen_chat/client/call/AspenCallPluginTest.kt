package com.aspen_chat.client.call

import android.Manifest
import android.content.Context
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.MainActivity
import com.aspen_chat.client.push.PushStore
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
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
 * while Telecom's current endpoint changes, mutes and unmutes, and leaves.
 * `scripts/check_mobile_call.py --platform android` runs it with `CallEngineTest` and gives it the
 * same arguments; without them it is skipped.
 */
@RunWith(AndroidJUnit4::class)
class AspenCallPluginTest {
    private lateinit var context: Context
    private lateinit var store: PushStore
    private var keptPushState: String? = null
    private var scenario: ActivityScenario<MainActivity>? = null
    private var runs = 0

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
        automation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
    }

    @After
    fun tearDown() {
        val kept = keptPushState
        if (kept != null) store.save(kept) else store.remove()
        scenario?.close()
    }

    @Test
    fun snapshotsAndRoutesCrossTheBridge() {
        val api = argument("ASPEN_CALL_API")
        val channel = argument("ASPEN_CALL_CHANNEL")
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")

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
                "routes $kinds, moved to $moved as the routes events said; a fresh read matched the latest",
        )
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
        /**
         * What the page records under `window.__aspenCallTest`: the plugin's events, each `state`
         * one read as the engine's JSON with its origin beside, and each command's outcome by name.
         */
        val RECORDER = """
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
        """.trimIndent()
    }
}
