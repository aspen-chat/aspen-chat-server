// Proves the Swift bindings link the engine and talk to it: the platform protocols are
// implemented by stand-ins, the engine is started, asked to join a call the stand-in API server
// refuses, read until it reports the refusal, and released. `swift.sh` builds and runs it,
// compiling the generated `VoiceClient.swift` into this module (an app imports `VoiceClient`).

import Foundation

final class Locked<T>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: T

    init(_ value: T) {
        self.value = value
    }

    func with<R>(_ body: (inout T) -> R) -> R {
        lock.lock()
        defer { lock.unlock() }
        return body(&value)
    }
}

final class FakeCredentials: Credentials {
    func sessionToken() async -> String? { "secret" }
}

final class FakeHttp: Http {
    let requests = Locked<[HttpRequest]>([])

    func request(request: HttpRequest) async throws -> HttpResponse {
        requests.with { $0.append(request) }
        return HttpResponse(
            status: 403,
            headers: [Header(name: "Content-Type", value: "application/problem+json")],
            body: #"{"code":"forbidden","title":"Forbidden","detail":"You may not join this call.","status":403}"#
        )
    }
}

final class FakeSockets: SocketFactory {
    func connect(url: String, bearer: String?) async throws -> ConnectedSocket {
        throw TransportError.Failed(detail: "this check opens no sockets")
    }
}

final class FakePeerConnections: PeerConnectionFactory {
    func create(configuration: Configuration, observer: PeerConnectionObserver) async throws -> CreatedPeerConnection {
        throw PeerConnectionError.Failed(detail: "this check makes no peer connections")
    }
}

final class Snapshots: CallObserver {
    let seen = Locked<[CallState]>([])

    func stateChanged(state: CallState) {
        seen.with { $0.append(state) }
    }
}

func run() async throws -> String {
    let http = FakeHttp()
    let snapshots = Snapshots()
    let engine = Engine.start(
        platform: Platform(http: http, sockets: FakeSockets(), peerConnections: FakePeerConnections()),
        options: Options(apiUrl: "https://chat.example.org", credentials: FakeCredentials(), loopbackPlainText: false),
        observer: snapshots
    )
    precondition(engine.state().status == .idle, "a started engine is idle")
    engine.setMicrophone(track: "microphone-1")
    let channel = UUID().uuidString.lowercased()
    engine.join(channel: channel)
    let started = Date()
    while engine.state().status != .failed {
        precondition(Date().timeIntervalSince(started) < 10, "the call never failed: \(engine.state())")
        try await Task.sleep(nanoseconds: 5_000_000)
    }
    let state = engine.state()
    precondition(state.errorKind == .offer, "failed at the offer: \(state)")
    precondition(state.error == "You may not join this call.", "with the Problem's detail: \(state)")
    precondition(state.channel == channel, "keeping the channel: \(state)")
    // The offer failed before granting speech, so the engine dropped the microphone it was given.
    precondition(state.microphone == nil, "the microphone was dropped: \(state)")
    let requests = http.requests.with { $0 }
    precondition(requests.count == 1, "one request: \(requests)")
    precondition(requests[0].url == "https://chat.example.org/api/v1/channels/\(channel)/voice/join", requests[0].url)
    precondition(requests[0].headers.contains(Header(name: "Authorization", value: "Bearer secret")), "\(requests[0].headers)")
    let seen = snapshots.seen.with { $0 }
    precondition(seen.last == state, "the observer saw the latest snapshot")
    precondition(zip(seen, seen.dropFirst()).allSatisfy { $1.revision == $0.revision + 1 }, "revisions count up by one")
    // `engine` is released on return, which leaves the call and waits for the engine's thread.
    return "ok: \(seen.count) snapshots, the last revision \(state.revision), status \(state.status)"
}

@main
struct Check {
    static func main() async throws {
        print(try await run())
    }
}
