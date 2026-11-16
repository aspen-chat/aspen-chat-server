import AVFoundation
import Capacitor
import PhotosUI
import UIKit
import UniformTypeIdentifiers

/// Photos and videos from the system's own screens for a message's attachments (`AspenMedia` in
/// the app's `src/api/mediaBridge.ts`, which Android's plugin of the same name answers too): the
/// photo picker, which runs apart from the app and hands it only what was chosen, so the app
/// holds no permission to read the photo library, and the camera. iOS lends no camera from
/// another process, so taking a photo or a video uses the app's own camera access, asked for
/// the first time it is needed.
///
/// The picker hands over what is chosen in its most compatible form (JPEG rather than HEIC, H.264
/// rather than HEVC), which every reader's device can show and the server can make previews of.
/// Each thing chosen or taken is copied into a directory of its own under the app's caches,
/// keeping its name, and answered as a `file:` URL the page reads it from and then `release`s.
@objc(AspenMediaPlugin)
public class AspenMediaPlugin: CAPPlugin, CAPBridgedPlugin, PHPickerViewControllerDelegate,
    UIImagePickerControllerDelegate, UINavigationControllerDelegate
{
    public let identifier = "AspenMediaPlugin"
    public let jsName = "AspenMedia"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "sources", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "choose", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "release", returnType: CAPPluginReturnPromise),
    ]

    /// The choice under way; touched only on the main thread.
    private var choosing: CAPPluginCall?

    private static var directory: URL {
        FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("media", isDirectory: true)
    }

    override public func load() {
        // Copies the page never released when the app last ran.
        try? FileManager.default.removeItem(at: Self.directory)
    }

    /// Which of the system's screens this device has: the photo picker, and the camera for each.
    @objc func sources(_ call: CAPPluginCall) {
        DispatchQueue.main.async {
            let camera =
                UIImagePickerController.isSourceTypeAvailable(.camera)
                ? UIImagePickerController.availableMediaTypes(for: .camera) ?? [] : []
            call.resolve([
                "library": true,
                "photo": camera.contains(UTType.image.identifier),
                "video": camera.contains(UTType.movie.identifier),
            ])
        }
    }

    /// Opens `source`'s screen: `library` (photos and videos, at most `limit`), `photo`, or
    /// `video`. Answers what was chosen or taken, none when the user backed out.
    @objc func choose(_ call: CAPPluginCall) {
        let source = call.getString("source") ?? ""
        let limit = max(1, call.getInt("limit") ?? 1)
        DispatchQueue.main.async {
            guard self.choosing == nil else {
                call.reject("another choice is under way")
                return
            }
            switch source {
            case "library":
                self.choosing = call
                self.pick(limit: limit)
            case "photo", "video":
                self.choosing = call
                self.withCamera { allowed in
                    if allowed {
                        self.capture(video: source == "video")
                    } else {
                        self.endChoice()?.reject("the camera is not allowed", "cameraDenied")
                    }
                }
            default:
                call.reject("unknown source: \(source)")
            }
        }
    }

    /// Removes `url`, a copy this plugin answered, and the directory it was made in.
    @objc func release(_ call: CAPPluginCall) {
        if let url = URL(string: call.getString("url") ?? "") {
            Self.release(url)
        }
        call.resolve()
    }

    private func pick(limit: Int) {
        var configuration = PHPickerConfiguration()
        configuration.selectionLimit = limit
        configuration.filter = .any(of: [.images, .videos])
        configuration.preferredAssetRepresentationMode = .compatible
        let picker = PHPickerViewController(configuration: configuration)
        picker.delegate = self
        present(picker)
    }

    public func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
        picker.dismiss(animated: true)
        guard let call = endChoice() else { return }
        var items = [[String: Any]?](repeating: nil, count: results.count)
        var failure: Error?
        let copied = DispatchGroup()
        for (index, result) in results.enumerated() {
            let provider = result.itemProvider
            guard
                let type = provider.registeredTypeIdentifiers.lazy.compactMap({ UTType($0) }).first(where: {
                    $0.conforms(to: .image) || $0.conforms(to: .movie)
                })
            else { continue }
            copied.enter()
            // The file is the provider's only until this returns, so it is copied here.
            provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, error in
                defer { copied.leave() }
                do {
                    guard let url else {
                        throw error ?? CocoaError(.fileReadUnknown)
                    }
                    let stem = provider.suggestedName ?? url.deletingPathExtension().lastPathComponent
                    let name = type.preferredFilenameExtension.map { "\(stem).\($0)" } ?? url.lastPathComponent
                    let copy = try Self.copy(url, named: name)
                    let item = try Self.item(copy, name: name, mimeType: type.preferredMIMEType ?? "")
                    DispatchQueue.main.async { items[index] = item }
                } catch {
                    DispatchQueue.main.async { failure = error }
                }
            }
        }
        copied.notify(queue: .main) {
            let answered = items.compactMap { $0 }
            if let failure {
                for item in answered {
                    if let url = URL(string: item["url"] as? String ?? "") {
                        Self.release(url)
                    }
                }
                call.reject("what was chosen could not be read: \(failure.localizedDescription)")
            } else {
                call.resolve(["items": answered])
            }
        }
    }

    /// Calls `then` on the main thread with whether the app may use the camera, asking first if
    /// it has never been asked.
    private func withCamera(_ then: @escaping (Bool) -> Void) {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            then(true)
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { allowed in
                DispatchQueue.main.async { then(allowed) }
            }
        case .denied, .restricted:
            then(false)
        @unknown default:
            then(false)
        }
    }

    private func capture(video: Bool) {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.mediaTypes = [video ? UTType.movie.identifier : UTType.image.identifier]
        if video {
            picker.cameraCaptureMode = .video
            picker.videoQuality = .typeHigh
        }
        picker.delegate = self
        present(picker)
    }

    public func imagePickerController(
        _ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]
    ) {
        picker.dismiss(animated: true)
        guard let call = endChoice() else { return }
        do {
            let item: [String: Any]
            if let movie = info[.mediaURL] as? URL {
                let type = UTType(filenameExtension: movie.pathExtension) ?? .quickTimeMovie
                let copy = try Self.copy(movie, named: movie.lastPathComponent)
                item = try Self.item(copy, name: "", mimeType: type.preferredMIMEType ?? "video/quicktime")
            } else if let image = info[.originalImage] as? UIImage, let jpeg = image.jpegData(compressionQuality: 0.9) {
                let copy = try Self.holding().appendingPathComponent("capture.jpg")
                try jpeg.write(to: copy)
                item = try Self.item(copy, name: "", mimeType: "image/jpeg")
            } else {
                call.reject("the camera answered nothing")
                return
            }
            // What the camera takes is named by the page, for when it was taken.
            call.resolve(["items": [item]])
        } catch {
            call.reject("what the camera took could not be kept: \(error.localizedDescription)")
        }
    }

    public func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
        picker.dismiss(animated: true)
        endChoice()?.resolve(["items": []])
    }

    private func present(_ controller: UIViewController) {
        guard let presenter = bridge?.viewController else {
            endChoice()?.reject("there is no screen to ask on")
            return
        }
        presenter.present(controller, animated: true)
    }

    private func endChoice() -> CAPPluginCall? {
        defer { choosing = nil }
        return choosing
    }

    /// One thing answered: where its copy is, its name (empty for what the camera took, which the
    /// page names for when it was taken), its type, and how many bytes the page is to read.
    private static func item(_ copy: URL, name: String, mimeType: String) throws -> [String: Any] {
        let size = try copy.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        return ["url": copy.absoluteString, "name": name, "mimeType": mimeType, "byteSize": size]
    }

    /// A new directory of its own for one copy, so the copy keeps its name whatever else is held.
    private static func holding() throws -> URL {
        let holding = directory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: holding, withIntermediateDirectories: true)
        return holding
    }

    private static func copy(_ url: URL, named name: String) throws -> URL {
        let copy = try holding().appendingPathComponent(
            (name as NSString).lastPathComponent.isEmpty ? "media" : (name as NSString).lastPathComponent)
        try FileManager.default.copyItem(at: url, to: copy)
        return copy
    }

    private static func release(_ url: URL) {
        let holding = url.deletingLastPathComponent().standardizedFileURL
        guard url.isFileURL, holding.deletingLastPathComponent().path == directory.standardizedFileURL.path
        else { return }
        try? FileManager.default.removeItem(at: holding)
    }
}
