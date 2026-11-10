import Capacitor
import Foundation

/// Uploading attachments from the app's native side (`AspenUploads` in the app's
/// `src/api/uploadsBridge.ts`, which Android's plugin of the same name answers too). The page
/// hands over a file's bytes, in base64, and where they go; `UploadCenter` sends and confirms
/// them, and outlives this plugin and its web view to do so. The plugin only passes the page's
/// calls on and tells the page what the center tells it, so an upload that ends with no page
/// to tell is found by the page's next `list`.
@objc(AspenUploadsPlugin)
public class AspenUploadsPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenUploadsPlugin"
    public let jsName = "AspenUploads"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "begin", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "write", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "send", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "list", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "discard", returnType: CAPPluginReturnPromise),
    ]

    override public func load() {
        UploadCenter.shared.listen(
            progress: { [weak self] id, sent, total in
                self?.notifyListeners("progress", data: ["id": id, "sent": sent, "total": total])
            },
            // Kept for a page that has not begun listening yet, as one still loading has not.
            settled: { [weak self] upload in
                self?.notifyListeners("settled", data: upload, retainUntilConsumed: true)
            }
        )
    }

    @objc func begin(_ call: CAPPluginCall) {
        guard let id = call.getString("id") else {
            call.reject("id is required")
            return
        }
        UploadCenter.shared.begin(id: id) { Self.answer(call, $0) }
    }

    @objc func write(_ call: CAPPluginCall) {
        guard let id = call.getString("id") else {
            call.reject("id is required")
            return
        }
        guard let base64 = call.getString("data"), let data = Data(base64Encoded: base64) else {
            call.reject("data must be base64")
            return
        }
        UploadCenter.shared.write(id: id, data: data) { Self.answer(call, $0) }
    }

    @objc func send(_ call: CAPPluginCall) {
        var given: [String: String] = [:]
        for name in [
            "id", "origin", "userId", "sessionToken", "refreshToken", "uploadUrl", "contentType", "name",
            "mimeType",
        ] {
            guard let value = call.getString(name), !value.isEmpty else {
                call.reject("\(name) is required")
                return
            }
            given[name] = value
        }
        guard let byteSize = call.getInt("byteSize"), byteSize >= 0 else {
            call.reject("byteSize is required")
            return
        }
        for name in ["origin", "uploadUrl"] {
            let scheme = URL(string: given[name] ?? "")?.scheme?.lowercased()
            guard scheme == "https" || scheme == "http" else {
                call.reject("\(name) must be a web address")
                return
            }
        }
        let record = UploadRecord(
            id: given["id"] ?? "",
            origin: given["origin"] ?? "",
            userId: given["userId"] ?? "",
            sessionToken: given["sessionToken"] ?? "",
            refreshToken: given["refreshToken"] ?? "",
            uploadUrl: given["uploadUrl"] ?? "",
            contentType: given["contentType"] ?? "",
            name: given["name"] ?? "",
            mimeType: given["mimeType"] ?? "",
            byteSize: Int64(byteSize),
            acceptLanguage: call.getString("acceptLanguage"),
            failedTitle: call.getString("failedTitle") ?? "",
            failedBody: call.getString("failedBody") ?? "",
            sentAt: Date()
        )
        UploadCenter.shared.send(record) { Self.answer(call, $0) }
    }

    @objc func list(_ call: CAPPluginCall) {
        guard let origin = call.getString("origin"), let userId = call.getString("userId") else {
            call.reject("origin and userId are required")
            return
        }
        UploadCenter.shared.list(origin: origin, userId: userId) { result in
            switch result {
            case .success(let uploads):
                call.resolve(["uploads": uploads])
            case .failure(let refusal):
                call.reject(refusal.message)
            }
        }
    }

    @objc func discard(_ call: CAPPluginCall) {
        guard let id = call.getString("id") else {
            call.reject("id is required")
            return
        }
        UploadCenter.shared.discard(id: id) { Self.answer(call, $0) }
    }

    private static func answer(_ call: CAPPluginCall, _ result: Result<Void, UploadCenter.Refusal>) {
        switch result {
        case .success:
            call.resolve()
        case .failure(let refusal):
            call.reject(refusal.message)
        }
    }
}
