import Foundation

/// The call engine's WebSockets (`SocketFactory`, `Socket`) over `URLSessionWebSocketTask`,
/// which answers the server's pings itself and follows the system between Wi-Fi and cellular.
/// `connect` returns once the upgrade is answered, so a refused or unreachable server is the
/// engine's `TransportError` and never a socket that closes at once.
final class CallSockets: NSObject, SocketFactory, URLSessionWebSocketDelegate, @unchecked Sendable {
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.timeoutIntervalForRequest = 30
        return URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()

    private let lock = NSLock()
    /// Each task waiting for its upgrade, by the task, until the delegate hears it open or end.
    private var opening: [ObjectIdentifier: CheckedContinuation<Void, Error>] = [:]

    func connect(url: String, bearer: String?) async throws -> ConnectedSocket {
        guard let url = URL(string: url) else {
            throw TransportError.Failed(detail: "\(url) is not a URL")
        }
        var request = URLRequest(url: url)
        if let bearer {
            request.setValue("Bearer \(bearer)", forHTTPHeaderField: "Authorization")
        }
        let task = session.webSocketTask(with: request)
        do {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                lock.lock()
                opening[ObjectIdentifier(task)] = continuation
                lock.unlock()
                task.resume()
            }
        } catch {
            throw TransportError.Failed(detail: error.localizedDescription)
        }
        return ConnectedSocket(socket: CallSocket(task: task))
    }

    private func opened(_ task: URLSessionTask, result: Result<Void, Error>) {
        lock.lock()
        let continuation = opening.removeValue(forKey: ObjectIdentifier(task))
        lock.unlock()
        continuation?.resume(with: result)
    }

    func urlSession(
        _ session: URLSession,
        webSocketTask: URLSessionWebSocketTask,
        didOpenWithProtocol protocol: String?
    ) {
        opened(webSocketTask, result: .success(()))
    }

    /// A task that ends before it opened was refused (a non-101 answer, no route, a certificate
    /// not trusted); one that ends afterwards is read as `Closed` by whoever receives on it.
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        let failure = error ?? URLError(.cannotConnectToHost)
        opened(task, result: .failure(failure))
    }
}

/// One open WebSocket. Closing it, from either side, makes the next `receive` answer `Closed`.
final class CallSocket: Socket, @unchecked Sendable {
    private let task: URLSessionWebSocketTask

    init(task: URLSessionWebSocketTask) {
        self.task = task
    }

    func sendText(text: String) async throws {
        do {
            try await task.send(.string(text))
        } catch {
            throw TransportError.Failed(detail: error.localizedDescription)
        }
    }

    func receive() async -> Received {
        while true {
            let message: URLSessionWebSocketTask.Message
            do {
                message = try await task.receive()
            } catch {
                return .closed
            }
            switch message {
            case .string(let text):
                return .text(text)
            case .data:
                continue
            @unknown default:
                continue
            }
        }
    }

    func closeSocket() {
        task.cancel(with: .normalClosure, reason: nil)
    }
}
