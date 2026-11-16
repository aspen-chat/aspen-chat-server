// Proves the Kotlin bindings load the engine and talk to it: the platform traits are implemented
// by stand-ins, the engine is started, asked to join a call the stand-in API server refuses, read
// until it reports the refusal, and stopped. `kotlin.sh` builds and runs it.

import com.aspen_chat.client.voice.*
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList

class FakeCredentials : Credentials {
    override suspend fun sessionToken(): String? = "secret"
    override suspend fun renewedSessionToken(rejected: String): String? = null
}

class FakeHttp : Http {
    val requests = CopyOnWriteArrayList<HttpRequest>()

    override suspend fun request(request: HttpRequest): HttpResponse {
        requests.add(request)
        return HttpResponse(
            403.toUShort(),
            listOf(Header("Content-Type", "application/problem+json")),
            """{"code":"forbidden","title":"Forbidden","detail":"You may not join this call.","status":403}""",
        )
    }
}

class FakeSockets : SocketFactory {
    override suspend fun connect(url: String, bearer: String?): ConnectedSocket =
        throw TransportException.Failed("this check opens no sockets")
}

class FakePeerConnections : PeerConnectionFactory {
    override suspend fun create(configuration: Configuration, observer: PeerConnectionObserver): CreatedPeerConnection =
        throw PeerConnectionException.Failed("this check makes no peer connections")
}

class Snapshots : CallObserver {
    val seen = CopyOnWriteArrayList<CallState>()
    /** The JSON beside each snapshot, which the apps hand their web views. */
    val json = CopyOnWriteArrayList<String>()
    /** The call's sounds as the engine cues them; a join the API server refuses cues none. */
    val sounds = CopyOnWriteArrayList<SoundCue>()

    override fun stateChanged(state: CallState, json: String) {
        seen.add(state)
        this.json.add(json)
    }

    override fun playSound(cue: SoundCue) {
        sounds.add(cue)
    }
}

fun main() {
    val http = FakeHttp()
    val snapshots = Snapshots()
    val engine = Engine.start(
        Platform(http, FakeSockets(), FakePeerConnections()),
        Options("https://chat.example.org", FakeCredentials(), false),
        snapshots,
    )
    check(engine.state().status == CallStatus.IDLE) { "a started engine is idle" }
    engine.setMicrophone("microphone-1")
    val channel = UUID.randomUUID().toString()
    engine.join(channel)
    val started = System.nanoTime()
    while (engine.state().status != CallStatus.FAILED) {
        check(System.nanoTime() - started < 10_000_000_000L) { "the call never failed: ${engine.state()}" }
        Thread.sleep(5)
    }
    val state = engine.state()
    check(state.errorKind == ErrorKind.OFFER) { "failed at the offer: $state" }
    check(state.error == "You may not join this call.") { "with the Problem's detail: $state" }
    check(state.channel == channel) { "keeping the channel: $state" }
    // The offer failed before granting speech, so the engine dropped the microphone it was given.
    check(state.microphone == null) { "the microphone was dropped: $state" }
    val request = http.requests.single()
    check(request.url == "https://chat.example.org/api/v1/channels/$channel/voice/join") { request.url }
    check(request.headers.contains(Header("Authorization", "Bearer secret"))) { "${request.headers}" }
    check(snapshots.seen.last() == state) { "the observer saw the latest snapshot" }
    check(snapshots.seen.zipWithNext().all { (before, after) -> after.revision == before.revision + 1uL }) {
        "revisions count up by one: ${snapshots.seen.map { it.revision }}"
    }
    check(snapshots.sounds.isEmpty()) { "a refused join cues no sound: ${snapshots.sounds}" }
    check(SoundCue.values().size == 7) { "every cue crosses: ${SoundCue.values().toList()}" }
    engine.close()
    println("ok: ${snapshots.seen.size} snapshots, the last revision ${state.revision}, status ${state.status}")
}
