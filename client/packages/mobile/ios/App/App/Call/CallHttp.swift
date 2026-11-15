import Foundation

/// The call engine's HTTP requests (`Http`) over URLSession, so TLS trust and proxies are the
/// system's. Each request is sent as the engine wrote it: no cookies, no redirects followed, no
/// header added, and the answer's status, headers, and body given back whatever the status.
final class CallHttp: NSObject, Http, URLSessionTaskDelegate, @unchecked Sendable {
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.timeoutIntervalForRequest = 30
        return URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
    }()

    func request(request: HttpRequest) async throws -> HttpResponse {
        guard let url = URL(string: request.url) else {
            throw TransportError.Failed(detail: "\(request.url) is not a URL")
        }
        var sent = URLRequest(url: url)
        sent.httpMethod = request.method
        for header in request.headers {
            sent.setValue(header.value, forHTTPHeaderField: header.name)
        }
        if let body = request.body {
            sent.httpBody = Data(body.utf8)
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: sent)
        } catch {
            throw TransportError.Failed(detail: error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else {
            throw TransportError.Failed(detail: "\(request.url) answered with no HTTP response")
        }
        let headers = http.allHeaderFields.compactMap { name, value -> Header? in
            guard let name = name as? String, let value = value as? String else {
                return nil
            }
            return Header(name: name, value: value)
        }
        return HttpResponse(
            status: UInt16(clamping: http.statusCode),
            headers: headers,
            body: String(decoding: data, as: UTF8.self)
        )
    }

    /// A redirect is answered as the status it came with: the engine sends every request to a
    /// URL it made itself, and a redirected answer is one it did not ask for.
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}
