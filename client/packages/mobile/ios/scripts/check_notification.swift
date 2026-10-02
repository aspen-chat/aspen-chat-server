// The notification service extension's work, run as a Mac program with the extension's own
// sources (`check_notification.sh`): a `PushState` as the app keeps it (JSON), the APNs payload
// a relay sends (`aspen.s` and `aspen.c`), and the deployment the account names, reached over
// the network. Prints what the notification would show, or why it would not. The simulator
// never launches a service extension for `xcrun simctl push` (the simulator bridge adds the
// notification as a request directly, never through the path that runs extensions), so this is
// how the extension's path is checked short of a device.

import Foundation

let arguments = CommandLine.arguments
guard arguments.count == 3 else {
    print("usage: check_notification <state.json> <payload.json>")
    exit(2)
}
let state = try JSONDecoder().decode(PushState.self, from: Data(contentsOf: URL(fileURLWithPath: arguments[1])))
guard let payload = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: arguments[2]))) as? [String: Any],
      let aspen = payload["aspen"] as? [String: Any],
      let subscription = aspen["s"] as? String,
      let ciphertext = aspen["c"] as? String,
      let message = Data(base64url: ciphertext)
else {
    print("FAIL: the payload carries no aspen.s and aspen.c")
    exit(1)
}
guard let account = state.accounts.first(where: { $0.subscription == subscription }) else {
    print("FAIL: no account for subscription \(subscription); kept: \(state.accounts.map(\.subscription))")
    exit(1)
}
print("account \(account.userId) at \(account.origin), subscription \(subscription)")
guard let privateKey = Data(base64url: account.keys.privateKey.d),
      let publicKey = Data(base64url: account.keys.publicKey),
      let auth = Data(base64url: account.keys.auth)
else {
    print("FAIL: the account's keys do not read as base64url")
    exit(1)
}
let plaintext = try WebPush.decrypt(message: message, privateKey: privateKey, publicKey: publicKey, auth: auth)
guard let pointer = try JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
    print("FAIL: the pointer is not a JSON object")
    exit(1)
}
print("pointer \(pointer)")
guard pointer["kind"] as? String == "message", let messageId = pointer["message"] as? String else {
    print("a \(pointer["kind"] ?? "?") pointer shows nothing new")
    exit(0)
}
let done = DispatchSemaphore(value: 0)
var outcome: ShownMessage?
MessageFetcher().fetch(account: account, messageId: messageId) { shown in
    outcome = shown
    done.signal()
}
_ = done.wait(timeout: .now() + 20)
guard let shown = outcome else {
    print("FAIL: the message could not be fetched; the placeholder would stay")
    exit(1)
}
print("title: \(shown.title)")
print("subtitle: \(shown.place ?? "")")
print("body: \(shown.body)")
print("opens: community \(shown.community ?? "none"), parent channel \(shown.parentChannel ?? "none")")
