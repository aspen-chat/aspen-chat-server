import UserNotifications
import os

/// The extension's log, read with `log stream --predicate 'subsystem == "org.aspenchat.client"'`.
private let log = Logger(subsystem: "org.aspenchat.client", category: "push")

/// Turns a push into the notification it stands for, within the thirty seconds iOS gives an
/// extension (`spec/push.md`, What reaches the phone): the push carries only a subscription id
/// and ciphertext, so the extension finds the account the app kept for that subscription
/// (`PushStateStore`), decrypts the pointer (`WebPush`), fetches the message or plugin's notice
/// it names with the account's session (getting a new one with its refresh token on `401`), and
/// shows who wrote it, where, and what it says (a notice: its plugin's name and what it says),
/// grouped by channel, carrying where it is for a tap to open it.
/// Whatever fails leaves the placeholder the push came with, which reads sensibly on its own.
/// A `read` or `deleted` pointer shows nothing new, and hiding what was shown needs Apple's
/// filtering entitlement, which this build does not hold; it leaves the placeholder.
final class NotificationService: UNNotificationServiceExtension {
    private var handler: ((UNNotificationContent) -> Void)?
    private var content: UNMutableNotificationContent?
    private let fetcher = MessageFetcher()

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        handler = contentHandler
        let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        self.content = content
        guard let aspen = request.content.userInfo["aspen"] as? [String: Any],
              let subscription = aspen["s"] as? String,
              let ciphertext = aspen["c"] as? String,
              let message = Data(base64url: ciphertext)
        else {
            log.error("a push without the relay's subscription and ciphertext")
            contentHandler(content)
            return
        }
        guard let state = PushStateStore.load() else {
            log.error("no push state is kept for the extension to read")
            contentHandler(content)
            return
        }
        guard let account = state.accounts.first(where: { $0.subscription == subscription }) else {
            log.error("no account is kept for subscription \(subscription, privacy: .public)")
            contentHandler(content)
            return
        }
        guard let privateKey = Data(base64url: account.keys.privateKey.d),
              let publicKey = Data(base64url: account.keys.publicKey),
              let auth = Data(base64url: account.keys.auth)
        else {
            log.error("the account's keys do not read as base64url")
            contentHandler(content)
            return
        }
        let plaintext: Data
        do {
            plaintext = try WebPush.decrypt(message: message, privateKey: privateKey, publicKey: publicKey, auth: auth)
        } catch {
            log.error("the push does not decrypt: \(String(describing: error), privacy: .public)")
            contentHandler(content)
            return
        }
        guard let pointer = try? JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
            log.error("the pointer is not a JSON object")
            contentHandler(content)
            return
        }
        if let badge = pointer["badge"] as? Int {
            content.badge = NSNumber(value: badge)
        }
        guard let channel = pointer["channel"] as? String else {
            contentHandler(content)
            return
        }
        let show: (ShownMessage?) -> Void = { [weak self] shown in
            guard let self, let handler = self.handler else {
                return
            }
            self.handler = nil
            if let shown {
                content.title = shown.title
                content.subtitle = shown.place ?? ""
                content.body = shown.body
                content.threadIdentifier = channel
                content.categoryIdentifier = "message"
                content.userInfo = [
                    "origin": account.origin,
                    "channel": shown.channel,
                    "message": shown.message ?? NSNull(),
                    "community": shown.community ?? NSNull(),
                    "parentChannel": shown.parentChannel ?? NSNull(),
                ]
            } else {
                log.error("what a push points to could not be fetched; the placeholder stays")
            }
            handler(content)
        }
        if !fetcher.fetch(account: account, pointer: pointer, done: show) {
            contentHandler(content)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        // Out of time: the placeholder, with the badge if the pointer gave one.
        if let handler, let content {
            self.handler = nil
            handler(content)
        }
    }
}

/// What a fetched message or notice shows as, and what a tap on it opens: the message in
/// `channel`, or the channel itself for a notice about no message.
struct ShownMessage {
    let title: String
    let place: String?
    let body: String
    let channel: String
    let message: String?
    let community: String?
    let parentChannel: String?
}

/// Reads what a pointer names, as the app would (`GET /messages/{id}` with the authors,
/// channels, and tagged people sideloaded, or `GET /users/@me/plugin-notices/{id}`), with the
/// account's session, renewed once on `401`.
final class MessageFetcher {
    private static let timeout: TimeInterval = 12

    private let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = timeout
        configuration.timeoutIntervalForResource = timeout
        return URLSession(configuration: configuration)
    }()

    /// Fetches what `pointer` names, a message or a plugin's notice, and answers whether it names
    /// one; `done` is called only when it does.
    func fetch(account: PushState.Account, pointer: [String: Any], done: @escaping (ShownMessage?) -> Void) -> Bool {
        switch pointer["kind"] as? String {
        case "message":
            guard let messageId = pointer["message"] as? String else {
                return false
            }
            fetch(
                account: account,
                url: "\(account.origin)/api/v1/messages/\(messageId)?include=authors,channels,mentions",
                parse: { Self.shown(from: $0, messageId: messageId) },
                done: done
            )
            return true
        case "notice":
            guard let noticeId = pointer["notice"] as? String else {
                return false
            }
            fetch(
                account: account,
                url: "\(account.origin)/api/v1/users/@me/plugin-notices/\(noticeId)",
                parse: Self.notice(from:),
                done: done
            )
            return true
        default:
            return false
        }
    }

    private func fetch(
        account: PushState.Account,
        url: String,
        parse: @escaping (Data) -> ShownMessage?,
        done: @escaping (ShownMessage?) -> Void
    ) {
        get(url: url, token: account.sessionToken) { [self] status, data in
            if status == 200, let data {
                done(parse(data))
                return
            }
            guard status == 401 else {
                log.error("fetching what a push points to answered \(status, privacy: .public)")
                done(nil)
                return
            }
            refresh(account: account) { [self] token in
                guard let token else {
                    done(nil)
                    return
                }
                PushStateStore.setSessionToken(subscription: account.subscription, token: token)
                get(url: url, token: token) { status, data in
                    done(status == 200 && data != nil ? parse(data ?? Data()) : nil)
                }
            }
        }
    }

    private func get(url: String, token: String, done: @escaping (Int, Data?) -> Void) {
        guard let url = URL(string: url) else {
            done(0, nil)
            return
        }
        var request = URLRequest(url: url)
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        session.dataTask(with: request) { data, response, _ in
            done((response as? HTTPURLResponse)?.statusCode ?? 0, data)
        }.resume()
    }

    private func refresh(account: PushState.Account, done: @escaping (String?) -> Void) {
        guard let url = URL(string: "\(account.origin)/api/v1/auth/token-refresh") else {
            done(nil)
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["refreshToken": account.refreshToken])
        session.dataTask(with: request) { data, response, _ in
            guard (response as? HTTPURLResponse)?.statusCode == 200,
                  let data,
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let token = object["sessionToken"] as? String
            else {
                done(nil)
                return
            }
            done(token)
        }.resume()
    }

    /// The notification a message read makes: its author's name as the title, the channel as
    /// the place (`#name` in a community; none in a DM, whose people the title already names),
    /// and its text with tags as names, or what it holds instead of text.
    static func shown(from data: Data, messageId: String) -> ShownMessage? {
        guard let read = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let message = read["data"] as? [String: Any],
              let authorId = message["author"] as? String,
              let channelId = message["channelId"] as? String
        else {
            return nil
        }
        let included = read["included"] as? [String: Any] ?? [:]
        let author = find(included, "users", authorId)
        let channel = find(included, "channels", channelId)
        let community = channel?["community"] as? String
        let parentChannel = channel?["parentChannel"] as? String
        let place: String?
        if let name = channel?["name"] as? String, community != nil {
            place = "#\(name)"
        } else {
            place = nil
        }
        return ShownMessage(
            title: author.map(nameOf) ?? NSLocalizedString("notification.someone", comment: "Someone"),
            place: place,
            body: body(of: message, included: included),
            channel: channelId,
            message: messageId,
            community: community,
            parentChannel: parentChannel
        )
    }

    /// The notification a plugin's notice makes: the plugin's name as the title, and what it
    /// says, both in the language the session asked for.
    static func notice(from data: Data) -> ShownMessage? {
        guard let notice = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let title = notice["title"] as? String,
              let body = notice["body"] as? String,
              let channel = notice["channel"] as? String
        else {
            return nil
        }
        return ShownMessage(
            title: title,
            place: nil,
            body: body,
            channel: channel,
            message: notice["message"] as? String,
            community: notice["community"] as? String,
            parentChannel: notice["parentChannel"] as? String
        )
    }

    private static func body(of message: [String: Any], included: [String: Any]) -> String {
        let content = (message["content"] as? String ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !content.isEmpty {
            return readable(content, included: included)
        }
        switch message["kind"] as? String {
        case "poll":
            return NSLocalizedString("notification.poll", comment: "posted a poll")
        default:
            let attachments = message["attachments"] as? [Any] ?? []
            return attachments.isEmpty
                ? NSLocalizedString("notification.message", comment: "sent a message")
                : NSLocalizedString("notification.attachment", comment: "sent an attachment")
        }
    }

    /// The text as a notification shows it: people's tags as `@name`, roles' as `@…`.
    private static func readable(_ content: String, included: [String: Any]) -> String {
        var text = content
        if let users = try? NSRegularExpression(pattern: "<@([0-9a-fA-F-]{36})>") {
            let matches = users.matches(in: text, range: NSRange(text.startIndex..., in: text)).reversed()
            for match in matches {
                guard let whole = Range(match.range, in: text), let idRange = Range(match.range(at: 1), in: text) else {
                    continue
                }
                let name = find(included, "users", String(text[idRange])).map(nameOf) ?? "…"
                text.replaceSubrange(whole, with: "@\(name)")
            }
        }
        if let roles = try? NSRegularExpression(pattern: "<@&[0-9a-fA-F-]{36}>") {
            text = roles.stringByReplacingMatches(in: text, range: NSRange(text.startIndex..., in: text), withTemplate: "@…")
        }
        return text
    }

    private static func nameOf(_ user: [String: Any]) -> String {
        if let display = user["displayName"] as? String, !display.isEmpty {
            return display
        }
        return user["name"] as? String ?? "…"
    }

    private static func find(_ included: [String: Any], _ type: String, _ id: String) -> [String: Any]? {
        (included[type] as? [[String: Any]])?.first { $0["id"] as? String == id }
    }
}
