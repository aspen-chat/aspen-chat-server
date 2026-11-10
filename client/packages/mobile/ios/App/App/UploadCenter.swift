import UIKit
import UserNotifications
import os

/// Read with `log stream --predicate 'subsystem == "com.aspen-chat.client"'`.
private let log = Logger(subsystem: "com.aspen-chat.client", category: "uploads")

/// Sends attachments' bytes to storage and confirms them with their deployment, whether or not
/// the app is in front, or running at all (`AspenUploads` in the app's `src/api/uploadsBridge.ts`,
/// which `AspenUploadsPlugin` answers with this). A page's own requests stop when the app leaves
/// the front, and a message sent before its files had arrived is posted by its deployment the
/// moment the last is confirmed, so the confirming cannot wait for a page.
///
/// The bytes of each upload are spooled to a file, which a background `URLSession` puts to
/// storage: the system carries that transfer on while the app is suspended or gone, and starts
/// the app again to hear how it ended. The app then confirms the upload itself, as the user
/// whose session `send` gave it. Everything about an upload but its bytes is an `UploadRecord`
/// (`UploadStore`), kept from `send` until `discard`, so a page loaded later, or an app started
/// later, finds how each upload stands.
///
/// It is made at launch, before any page, because the system may start the app for the
/// session's events alone, with no web view to make a plugin. Everything of it but `shared` is
/// touched only on `queue`, which the sessions' delegate calls and answers arrive on too.
final class UploadCenter: NSObject, URLSessionTaskDelegate {
    static let shared = UploadCenter()

    /// Why a call was not carried out, in words for the page's log.
    struct Refusal: Error {
        let message: String
    }

    /// The background session's name, by which the system finds its transfers again after the
    /// app has been stopped; it must never change while an upload of an installed app is under
    /// way.
    static let sessionIdentifier = "com.aspen-chat.client.uploads"

    /// How long after `send` a record is kept, and its transfer let run, in seconds. A page
    /// reads how an upload ended long before, and a deployment has by then dropped an upload
    /// it never saw confirmed (`UNCONFIRMED_KEPT` in the server's `media_store.rs`).
    private static let lifetime: TimeInterval = 2 * 24 * 60 * 60
    /// How many other URLs one `send` may ask for when storage says its URL has expired.
    private static let renewalLimit = 2
    /// The least time between two `progress` events of one upload, in seconds.
    private static let progressInterval: TimeInterval = 0.15
    /// How long to wait before asking the deployment again, each time it does not answer or
    /// answers that it cannot now; after the last, the upload has failed as unreachable. Their
    /// sum, with the requests themselves, stays near the half minute the system gives an app
    /// it woke in the background.
    private static let retryDelays: [TimeInterval] = [1, 2, 4, 8]
    /// How long one request to the deployment may take, in seconds.
    private static let requestTimeout: TimeInterval = 15

    private static let locked = Refusal(message: "the phone has not been unlocked since it started")

    private let queue = DispatchQueue(label: "com.aspen-chat.client.uploads")

    /// The sessions' callbacks, on `queue`.
    private lazy var callbacks: OperationQueue = {
        let callbacks = OperationQueue()
        callbacks.underlyingQueue = queue
        callbacks.maxConcurrentOperationCount = 1
        return callbacks
    }()

    /// Carries the bytes to storage. Not discretionary, since someone is waiting for the
    /// message, and it starts the app for its events.
    private lazy var session: URLSession = {
        let configuration = URLSessionConfiguration.background(withIdentifier: Self.sessionIdentifier)
        configuration.isDiscretionary = false
        configuration.sessionSendsLaunchEvents = true
        configuration.timeoutIntervalForResource = Self.lifetime
        configuration.httpShouldSetCookies = false
        return URLSession(configuration: configuration, delegate: self, delegateQueue: callbacks)
    }()

    /// Asks the deployment: confirming, renewing, refreshing the session.
    private lazy var api: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = Self.requestTimeout
        configuration.timeoutIntervalForResource = Self.requestTimeout
        return URLSession(configuration: configuration, delegate: nil, delegateQueue: callbacks)
    }()

    private var application: UIApplication?

    /// Every upload held, by `key`; `nil` until the keychain has been read (`ready`).
    private var records: [String: UploadRecord]?
    /// The session's tasks this process has made or found, by upload.
    private var tasks: [String: URLSessionTask] = [:]
    /// How many bytes of each upload under way have reached storage.
    private var sent: [String: Int64] = [:]
    /// When each upload's progress was last told, in seconds of the system's uptime.
    private var told: [String: TimeInterval] = [:]
    /// What is being asked of the deployment, by upload.
    private var asking: [String: Work] = [:]

    /// What the system gave to be called once the session's events have been handled, and
    /// whether the session has said they all have.
    private var eventsHandled: (() -> Void)?
    private var eventsDelivered = false

    private var onProgress: ((_ id: String, _ sent: Int64, _ total: Int64) -> Void)?
    private var onSettled: (([String: Any]) -> Void)?

    /// Asking the deployment about one upload, which the app stays awake for when it is not in
    /// front: a background task taken from the system, given back when the asking ends or the
    /// system says time is up, whichever is first.
    private final class Work {
        var task: UIBackgroundTaskIdentifier = .invalid
        var ended = false
    }

    /// What the deployment said to a request.
    private enum Answer {
        case ok(Data)
        /// It refused, with a status other than one `transient` names; `reason` is its words.
        case refused(status: Int, reason: String?)
        /// It never answered, or kept answering that it could not, through every retry.
        case unreachable
    }

    /// Begins at launch: joins the background session, so the system has somewhere to deliver
    /// what its transfers did while the app was away, and takes up the uploads held.
    func start(application: UIApplication) {
        queue.async { [self] in
            self.application = application
            _ = session
            _ = ready()
        }
    }

    /// Keeps what the system gave the app to call when it was started, or woken, for the
    /// background session's events. It is called once those events have been delivered and
    /// every confirmation they began has ended, or run out of time: the system may suspend the
    /// app the moment it is.
    func handleEvents(ofSession identifier: String, completionHandler: @escaping () -> Void) {
        guard identifier == Self.sessionIdentifier else {
            completionHandler()
            return
        }
        queue.async { [self] in
            eventsHandled = completionHandler
            _ = session
            finishEventsIfIdle()
        }
    }

    /// Has `progress` and `settled` called, on the center's queue, with what the page is told
    /// of each; one pair at a time, the plugin's.
    func listen(
        progress: @escaping (_ id: String, _ sent: Int64, _ total: Int64) -> Void,
        settled: @escaping ([String: Any]) -> Void
    ) {
        queue.async { [self] in
            onProgress = progress
            onSettled = settled
        }
    }

    /// Makes an empty spool file for upload `id`, forgetting any upload held under it.
    func begin(id: String, done: @escaping (Result<Void, Refusal>) -> Void) {
        queue.async { [self] in
            guard let key = Self.key(id) else {
                done(.failure(Refusal(message: "id must be a UUID")))
                return
            }
            guard ready() else {
                done(.failure(Self.locked))
                return
            }
            forget(key)
            do {
                let file = try Self.spoolDirectory().appendingPathComponent(key, isDirectory: false)
                // Readable while the phone is locked, as the system must read it to send it then.
                let made = FileManager.default.createFile(
                    atPath: file.path, contents: nil,
                    attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
                done(made ? .success(()) : .failure(Refusal(message: "the upload's file could not be made")))
            } catch {
                done(.failure(Refusal(message: "the upload's file could not be made: \(error.localizedDescription)")))
            }
        }
    }

    /// Adds `data` to the spool file of upload `id`. Calls are carried out in the order made.
    func write(id: String, data: Data, done: @escaping (Result<Void, Refusal>) -> Void) {
        queue.async { [self] in
            guard let key = Self.key(id) else {
                done(.failure(Refusal(message: "id must be a UUID")))
                return
            }
            guard ready() else {
                done(.failure(Self.locked))
                return
            }
            // The system reads the file of an upload being sent; it must not grow under it.
            guard records?[key] == nil else {
                done(.failure(Refusal(message: "the upload has been sent; begin it again to replace its bytes")))
                return
            }
            do {
                let handle = try FileHandle(forWritingTo: Self.spool(key))
                defer { try? handle.close() }
                try handle.seekToEnd()
                try handle.write(contentsOf: data)
                done(.success(()))
            } catch {
                done(.failure(Refusal(message: "writing the upload's bytes failed: \(error.localizedDescription)")))
            }
        }
    }

    /// Sends the spooled bytes of `record`'s upload to its URL, in place of any sending of them
    /// under way, and answers once the system has the transfer.
    func send(_ record: UploadRecord, done: @escaping (Result<Void, Refusal>) -> Void) {
        queue.async { [self] in
            guard let key = Self.key(record.id) else {
                done(.failure(Refusal(message: "id must be a UUID")))
                return
            }
            guard ready() else {
                done(.failure(Self.locked))
                return
            }
            // Storage takes exactly the bytes the URL was signed for, so any other number
            // would be refused only after it had all been sent.
            let size = (try? FileManager.default.attributesOfItem(atPath: Self.spool(key).path))?[.size] as? NSNumber
            guard let size else {
                done(.failure(Refusal(message: "no bytes are held for this upload")))
                return
            }
            guard size.int64Value == record.byteSize else {
                done(.failure(Refusal(message: "\(size.int64Value) bytes are held for this upload, not \(record.byteSize)")))
                return
            }
            stop(key)
            UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.notification(key)])
            var record = record
            record.state = .sending
            record.arrived = false
            record.renewals = 0
            record.attachment = nil
            record.cause = nil
            record.reason = nil
            records?[key] = record
            put(key)
            done(.success(()))
        }
    }

    /// Every upload held for `userId` at `origin`, each as the page reads one (`NativeUpload`).
    func list(origin: String, userId: String, done: @escaping (Result<[[String: Any]], Refusal>) -> Void) {
        queue.async { [self] in
            guard ready(), let records else {
                done(.failure(Self.locked))
                return
            }
            let uploads = records
                .filter { $0.value.origin == origin && $0.value.userId == userId }
                .sorted { $0.value.sentAt < $1.value.sentAt }
                .map { described($0.key, $0.value) }
            done(.success(uploads))
        }
    }

    /// Stops upload `id` if it is under way and forgets it and its bytes; nothing, if nothing
    /// is held under it.
    func discard(id: String, done: @escaping (Result<Void, Refusal>) -> Void) {
        queue.async { [self] in
            guard let key = Self.key(id) else {
                done(.failure(Refusal(message: "id must be a UUID")))
                return
            }
            guard ready() else {
                done(.failure(Self.locked))
                return
            }
            forget(key)
            done(.success(()))
        }
    }

    /// What upload `id` is kept under, in the records and as its file's name: its UUID in lower
    /// case, or `nil` when `id` is not a UUID written the usual way. A path is only ever made
    /// from this, never from what the page wrote.
    private static func key(_ id: String) -> String? {
        guard let uuid = UUID(uuidString: id) else { return nil }
        let key = uuid.uuidString.lowercased()
        return key == id.lowercased() ? key : nil
    }

    /// Reads the records, the first time it can, and answers whether they are read. Until the
    /// phone has been unlocked once since starting they cannot be, and nothing is done about
    /// any upload: the session's events of that time are dropped, and `takeUp`, the next time
    /// this is asked and can read, starts again whatever they left without a task.
    private func ready() -> Bool {
        if records != nil {
            return true
        }
        guard let kept = UploadStore.load() else {
            return false
        }
        let oldest = Date().addingTimeInterval(-Self.lifetime)
        var records: [String: UploadRecord] = [:]
        for record in kept where record.sentAt >= oldest {
            if let key = Self.key(record.id) {
                records[key] = record
            }
        }
        self.records = records
        if records.count != kept.count {
            save()
        }
        sweep()
        takeUp()
        return true
    }

    private func save() {
        guard let records else { return }
        if !UploadStore.save(Array(records.values)) {
            log.error("the uploads' records could not be kept")
        }
    }

    /// An upload as the page reads one (`NativeUpload` in `uploadsBridge.ts`).
    private func described(_ key: String, _ record: UploadRecord) -> [String: Any] {
        var upload: [String: Any] = [
            "id": record.id,
            "name": record.name,
            "mimeType": record.mimeType,
            "byteSize": record.byteSize,
            "state": record.state.rawValue,
            "sent": record.state == .done ? record.byteSize : sent[key] ?? 0,
        ]
        switch record.state {
        case .sending:
            break
        case .done:
            upload["attachment"] = record.attachment ?? NSNull()
        case .failed:
            upload["cause"] = (record.cause ?? .unreachable).rawValue
            upload["reason"] = record.reason ?? NSNull()
        }
        return upload
    }

    /// Ends whatever is being done about upload `key`: its transfer, and asking its deployment.
    /// A transfer cancelled here still reports its end, which `current` then tells from the
    /// end of the upload's own.
    private func stop(_ key: String) {
        tasks.removeValue(forKey: key)?.cancel()
        if let work = asking[key] {
            end(work, key)
        }
        sent[key] = nil
        told[key] = nil
    }

    private func forget(_ key: String) {
        stop(key)
        if records?.removeValue(forKey: key) != nil {
            save()
        }
        try? FileManager.default.removeItem(at: Self.spool(key))
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [Self.notification(key)])
    }

    /// The record of upload `key` while it is still being sent by the transfer `task`, which is
    /// what a step begun for that transfer checks before going on: a `send` or `discard` since
    /// has taken the upload elsewhere, and the step's result is then nobody's.
    private func current(_ key: String, _ task: Int?) -> UploadRecord? {
        guard let record = records?[key], record.state == .sending, record.task == task else {
            return nil
        }
        return record
    }

    private static func spool(_ key: String) -> URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("uploads", isDirectory: true)
            .appendingPathComponent(key, isDirectory: false)
    }

    /// The directory the spool files are in, made if need be, and kept out of backups and
    /// transfers to a new phone: what it holds is on its way elsewhere, and worthless without
    /// the records, which stay on this device.
    private static func spoolDirectory() throws -> URL {
        let manager = FileManager.default
        let support = try manager.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        var directory = support.appendingPathComponent("uploads", isDirectory: true)
        try manager.createDirectory(at: directory, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try directory.setResourceValues(values)
        return directory
    }

    /// Removes the spool files no upload still to be sent owns: those of records dropped for
    /// their age, and those a page began and never sent. Run as the records are first read,
    /// before any page can have begun one.
    private func sweep() {
        guard let records, let directory = try? Self.spoolDirectory(),
            let files = try? FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
        else { return }
        for file in files {
            let record = records[file.lastPathComponent]
            if record == nil || record?.state == .done {
                try? FileManager.default.removeItem(at: file)
            }
        }
    }

    /// Starts a transfer of upload `key`'s spool file to its record's URL.
    private func put(_ key: String) {
        guard var record = records?[key] else { return }
        // A background session raises an exception for an address that is not the web's, or a
        // file that is not there, rather than failing the task.
        guard let url = URL(string: record.uploadUrl), ["https", "http"].contains(url.scheme?.lowercased() ?? ""),
            FileManager.default.fileExists(atPath: Self.spool(key).path)
        else {
            fail(key, .unreachable, nil)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        request.setValue(record.contentType, forHTTPHeaderField: "Content-Type")
        let task = session.uploadTask(with: request, fromFile: Self.spool(key))
        task.taskDescription = key
        record.task = task.taskIdentifier
        record.arrived = false
        records?[key] = record
        save()
        tasks[key] = task
        sent[key] = 0
        told[key] = nil
        task.resume()
    }

    /// Takes up the uploads held once their records are read: finds each one's transfer among
    /// the session's, cancels the transfers no record owns, confirms what had arrived and was
    /// not yet confirmed, and starts again from its spool file whatever was being sent and has
    /// no transfer.
    private func takeUp() {
        // Only records as they stand now are judged against the tasks found: one that `send`
        // takes up while the session is being asked has a task the answer may not list.
        let held = (records ?? [:]).mapValues { $0.task }
        session.getAllTasks { [self] found in
            var live: [String: URLSessionTask] = [:]
            for task in found {
                if let key = task.taskDescription, let record = records?[key], record.task == task.taskIdentifier {
                    live[key] = task
                } else {
                    task.cancel()
                }
            }
            for (key, task) in held {
                guard let record = current(key, task) else { continue }
                if record.arrived {
                    confirm(key)
                } else if let task = live[key] {
                    // Under way, or ended with its end still to be told to the delegate.
                    tasks[key] = task
                    sent[key] = task.countOfBytesSent
                } else {
                    put(key)
                }
            }
        }
    }

    func urlSession(
        _ session: URLSession, task: URLSessionTask, didSendBodyData bytesSent: Int64,
        totalBytesSent: Int64, totalBytesExpectedToSend: Int64
    ) {
        guard let key = task.taskDescription, let record = current(key, task.taskIdentifier) else { return }
        sent[key] = totalBytesSent
        let now = ProcessInfo.processInfo.systemUptime
        if totalBytesSent < record.byteSize, let last = told[key], now - last < Self.progressInterval {
            return
        }
        told[key] = now
        onProgress?(record.id, totalBytesSent, record.byteSize)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        guard ready(), let key = task.taskDescription, let record = current(key, task.taskIdentifier),
            !record.arrived
        else { return }
        tasks[key] = nil
        if let error = error as NSError? {
            // The center changes a record before cancelling its transfer, so a cancellation
            // that is still the record's own is the system's. The user closing the app from
            // the switcher cancels every transfer, and is no reason for the message they sent
            // to fail: the bytes are sent again, now that the app has been opened again.
            let reason = error.userInfo[NSURLErrorBackgroundTaskCancelledReasonKey] as? Int
            if error.domain == NSURLErrorDomain, error.code == NSURLErrorCancelled,
                reason == NSURLErrorCancelledReasonUserForceQuitApplication
            {
                put(key)
                return
            }
            log.error("upload \(key, privacy: .public) could not be sent: \(error.localizedDescription, privacy: .public)")
            fail(key, .unreachable, nil)
            return
        }
        let status = (task.response as? HTTPURLResponse)?.statusCode ?? 0
        switch status {
        case 200..<300:
            records?[key]?.arrived = true
            sent[key] = record.byteSize
            save()
            confirm(key)
        case 403:
            // What storage says of a URL that has expired, as one may have by the time the
            // system gets to a transfer it put off.
            renew(key)
        default:
            log.error("storage answered \(status, privacy: .public) to upload \(key, privacy: .public)")
            fail(key, .unreachable, nil)
        }
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        eventsDelivered = true
        finishEventsIfIdle()
    }

    /// Tells the system its events are handled, once they have all been delivered and nothing
    /// they began is still being asked of a deployment.
    private func finishEventsIfIdle() {
        guard eventsDelivered, asking.isEmpty, let handled = eventsHandled else { return }
        eventsHandled = nil
        eventsDelivered = false
        DispatchQueue.main.async(execute: handled)
    }

    /// Takes a background task for asking upload `key`'s deployment something, or `nil` when
    /// something is already being asked about it.
    private func stayAwake(for key: String) -> Work? {
        guard asking[key] == nil else { return nil }
        let work = Work()
        asking[key] = work
        if let application {
            // The system calls this on the main thread, and suspends the app when it returns
            // with the task still held, so the task is given back before it does.
            work.task = application.beginBackgroundTask(withName: "Aspen upload") { [self] in
                queue.sync { end(work, key) }
            }
        }
        return work
    }

    /// Gives back `work`'s background task; the first call for it only.
    private func end(_ work: Work, _ key: String) {
        guard !work.ended else { return }
        work.ended = true
        if asking[key] === work {
            asking[key] = nil
        }
        if work.task != .invalid {
            application?.endBackgroundTask(work.task)
        }
        finishEventsIfIdle()
    }

    /// Confirms upload `key`, whose bytes storage has, with its deployment. A confirmation the
    /// deployment does not know of may be one it has already taken, whose answer never arrived
    /// (the app was stopped, the connection dropped); the attachment can be read only once
    /// confirmed, so reading it tells.
    private func confirm(_ key: String) {
        guard let record = records?[key], let work = stayAwake(for: key) else { return }
        let task = record.task
        let done: (Data) -> Void = { [self] data in
            if current(key, task) != nil {
                succeed(key, attachment: String(decoding: data, as: UTF8.self))
            }
            end(work, key)
        }
        ask(key, task, "POST", "/confirm") { [self] answer in
            switch answer {
            case .ok(let data):
                done(data)
            case .refused(status: 404, let reason):
                ask(key, task, "GET", "") { [self] read in
                    switch read {
                    case .ok(let data):
                        done(data)
                    case .refused:
                        fail(key, task, .refused, reason)
                        end(work, key)
                    case .unreachable:
                        fail(key, task, .unreachable, nil)
                        end(work, key)
                    }
                }
            case .refused(_, let reason):
                fail(key, task, .refused, reason)
                end(work, key)
            case .unreachable:
                fail(key, task, .unreachable, nil)
                end(work, key)
            }
        }
    }

    /// Asks the deployment for another URL for upload `key`, whose own storage refused, and
    /// sends the bytes to it.
    private func renew(_ key: String) {
        guard let record = records?[key] else { return }
        guard record.renewals < Self.renewalLimit else {
            log.error("storage refused upload \(key, privacy: .public) at every URL it was given")
            fail(key, .unreachable, nil)
            return
        }
        guard let work = stayAwake(for: key) else { return }
        records?[key]?.renewals += 1
        save()
        let task = record.task
        ask(key, task, "POST", "/renew", body: ["byteSize": record.byteSize]) { [self] answer in
            defer { end(work, key) }
            switch answer {
            case .ok(let data):
                guard current(key, task) != nil else { return }
                guard let handle = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                    let url = handle["uploadUrl"] as? String
                else {
                    fail(key, .unreachable, nil)
                    return
                }
                records?[key]?.uploadUrl = url
                if let type = handle["contentType"] as? String {
                    records?[key]?.contentType = type
                }
                put(key)
            case .refused(_, let reason):
                fail(key, task, .refused, reason)
            case .unreachable:
                fail(key, task, .unreachable, nil)
            }
        }
    }

    /// Asks upload `key`'s deployment `method` of its attachment (`path` after the attachment's
    /// own), as the record's user, for as long as the upload is still the transfer `task`'s.
    /// A session that has expired is refreshed, once, and the request made again; a deployment
    /// that does not answer is asked again after each of `retryDelays`.
    private func ask(
        _ key: String, _ task: Int?, _ method: String, _ path: String, body: [String: Any]? = nil,
        attempt: Int = 0, refreshed: Bool = false, done: @escaping (Answer) -> Void
    ) {
        guard let record = current(key, task),
            let url = URL(string: "\(record.origin)/api/v1/attachments/\(key)\(path)")
        else {
            done(.unreachable)
            return
        }
        let again = { [self] in
            guard attempt < Self.retryDelays.count else {
                done(.unreachable)
                return
            }
            queue.asyncAfter(deadline: .now() + Self.retryDelays[attempt]) { [self] in
                ask(key, task, method, path, body: body, attempt: attempt + 1, refreshed: refreshed, done: done)
            }
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("Bearer \(record.sessionToken)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        Self.speak(record, in: &request)
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        }
        api.dataTask(with: request) { [self] data, response, _ in
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            if (200..<300).contains(status) {
                done(.ok(data ?? Data()))
            } else if status == 401, !refreshed {
                refresh(key, task) { [self] answer in
                    switch answer {
                    case .ok:
                        ask(key, task, method, path, body: body, attempt: attempt, refreshed: true, done: done)
                    case .refused:
                        done(answer)
                    case .unreachable:
                        again()
                    }
                }
            } else if Self.transient(status, data) {
                again()
            } else {
                done(.refused(status: status, reason: Self.reason(data)))
            }
        }.resume()
    }

    /// Gets upload `key`'s record another session token with its refresh token.
    private func refresh(_ key: String, _ task: Int?, done: @escaping (Answer) -> Void) {
        guard let record = current(key, task),
            let url = URL(string: "\(record.origin)/api/v1/auth/token-refresh")
        else {
            done(.unreachable)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        Self.speak(record, in: &request)
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["refreshToken": record.refreshToken])
        api.dataTask(with: request) { [self] data, response, _ in
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            if status == 200, let data,
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let token = object["sessionToken"] as? String
            {
                if current(key, task) != nil {
                    records?[key]?.sessionToken = token
                    save()
                }
                done(.ok(data))
            } else if status == 200 || Self.transient(status, data) {
                done(.unreachable)
            } else {
                done(.refused(status: status, reason: Self.reason(data)))
            }
        }.resume()
    }

    /// Asks the deployment to answer `request` in the languages the page uses, so a refusal
    /// reads as the app does rather than as the phone is set.
    private static func speak(_ record: UploadRecord, in request: inout URLRequest) {
        if let languages = record.acceptLanguage, !languages.isEmpty {
            request.setValue(languages, forHTTPHeaderField: "Accept-Language")
        }
    }

    /// Whether an answer of `status` (0 for none) may be another the next time: the deployment
    /// was not reached, failed, or is holding the caller to its rate limits. A `429` of any
    /// other code is a refusal that asking again does not change (an upload quota).
    private static func transient(_ status: Int, _ body: Data?) -> Bool {
        if status == 429 {
            return problem(body)?["code"] as? String == "rateLimited"
        }
        return status == 0 || status == 408 || status >= 500
    }

    private static func problem(_ body: Data?) -> [String: Any]? {
        guard let body else { return nil }
        return try? JSONSerialization.jsonObject(with: body) as? [String: Any]
    }

    /// The deployment's own words for a refusal, from its `application/problem+json`: the
    /// `detail`, or the `title` when it gives none.
    private static func reason(_ body: Data?) -> String? {
        let problem = problem(body)
        if let detail = problem?["detail"] as? String, !detail.isEmpty {
            return detail
        }
        return problem?["title"] as? String
    }

    /// Empties what a record needs only while it is being sent. A record kept for the page to
    /// read holds no session, and no URL anyone could put bytes to.
    private static func spent(_ record: inout UploadRecord) {
        record.sessionToken = ""
        record.refreshToken = ""
        record.uploadUrl = ""
        record.task = nil
    }

    private func succeed(_ key: String, attachment: String) {
        guard var record = records?[key] else { return }
        record.state = .done
        record.attachment = attachment
        Self.spent(&record)
        records?[key] = record
        save()
        try? FileManager.default.removeItem(at: Self.spool(key))
        told[key] = nil
        onSettled?(described(key, record))
    }

    /// Fails upload `key`, if it is still the transfer `task`'s.
    private func fail(_ key: String, _ task: Int?, _ cause: UploadRecord.Cause, _ reason: String?) {
        if current(key, task) != nil {
            fail(key, cause, reason)
        }
    }

    /// Fails upload `key`, keeping its bytes for a `send` to try again, and says so with a
    /// notification when the app is not in front to say it itself.
    private func fail(_ key: String, _ cause: UploadRecord.Cause, _ reason: String?) {
        guard var record = records?[key] else { return }
        record.state = .failed
        record.cause = cause
        record.reason = reason
        Self.spent(&record)
        records?[key] = record
        save()
        told[key] = nil
        onSettled?(described(key, record))
        notify(key, record)
    }

    private static func notification(_ key: String) -> String {
        "upload-\(key)"
    }

    /// Shows `record`'s words for a failure, unless the app is in front or has not been allowed
    /// notifications. It never asks to be allowed: the page does, when the user turns
    /// notifications on.
    private func notify(_ key: String, _ record: UploadRecord) {
        guard !record.failedTitle.isEmpty || !record.failedBody.isEmpty else { return }
        DispatchQueue.main.async { [application] in
            guard let application, application.applicationState != .active else { return }
            let center = UNUserNotificationCenter.current()
            center.getNotificationSettings { settings in
                guard [.authorized, .provisional].contains(settings.authorizationStatus) else { return }
                let content = UNMutableNotificationContent()
                content.title = record.failedTitle
                content.body = record.failedBody
                content.sound = .default
                center.add(UNNotificationRequest(identifier: Self.notification(key), content: content, trigger: nil))
            }
        }
    }
}
