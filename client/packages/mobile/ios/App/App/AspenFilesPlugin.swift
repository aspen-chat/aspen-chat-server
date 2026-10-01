import Capacitor
import UIKit
import UniformTypeIdentifiers

/// Receiving a file into a place the user chose before it arrives (`AspenFiles` in the app's
/// `src/api/filesBridge.ts`, which Android's plugin of the same name answers too). iOS has no
/// picker that names a file yet to be written, so the user chooses a folder, and the file is made
/// there under the name it was sent with (numbered when the folder holds one already). The page
/// writes what arrives to it by id, in base64, until it closes or aborts it; an aborted file is
/// removed, since it holds only part of what was sent.
@objc(AspenFilesPlugin)
public class AspenFilesPlugin: CAPPlugin, CAPBridgedPlugin, UIDocumentPickerDelegate {
    public let identifier = "AspenFilesPlugin"
    public let jsName = "AspenFiles"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "create", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "write", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "close", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "abort", returnType: CAPPluginReturnPromise),
    ]

    /// A file being received: where it is, its handle, and the folder whose access it holds.
    private struct Sink {
        let folder: URL
        let file: URL
        let handle: FileHandle
        let accessing: Bool
    }

    /// Every file open, and the choice under way; touched only on `queue`.
    private let queue = DispatchQueue(label: "org.aspenchat.client.files")
    private var open: [String: Sink] = [:]
    private var choosing: (call: CAPPluginCall, name: String)?

    /// Asks where to save `name`; answers an id to write to, or `null` when the user backed out.
    @objc func create(_ call: CAPPluginCall) {
        guard let name = call.getString("name"), !name.isEmpty else {
            call.reject("name is required")
            return
        }
        queue.sync { choosing = (call, name) }
        DispatchQueue.main.async {
            let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
            picker.delegate = self
            self.bridge?.viewController?.present(picker, animated: true)
        }
    }

    public func documentPicker(
        _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
    ) {
        guard let (call, name) = takeChoice() else { return }
        guard let folder = urls.first else {
            call.resolve(["id": NSNull()])
            return
        }
        let accessing = folder.startAccessingSecurityScopedResource()
        do {
            let file = Self.freeName(name, in: folder)
            guard FileManager.default.createFile(atPath: file.path, contents: nil) else {
                throw CocoaError(.fileWriteNoPermission)
            }
            let handle = try FileHandle(forWritingTo: file)
            let id = UUID().uuidString
            let sink = Sink(folder: folder, file: file, handle: handle, accessing: accessing)
            queue.sync { open[id] = sink }
            call.resolve(["id": id])
        } catch {
            if accessing {
                folder.stopAccessingSecurityScopedResource()
            }
            call.reject("the file could not be made in the chosen folder: \(error.localizedDescription)")
        }
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        takeChoice()?.call.resolve(["id": NSNull()])
    }

    @objc func write(_ call: CAPPluginCall) {
        guard let sink = sinkOf(call) else { return }
        guard let base64 = call.getString("data"), let data = Data(base64Encoded: base64) else {
            call.reject("data must be base64")
            return
        }
        do {
            try sink.handle.write(contentsOf: data)
            call.resolve()
        } catch {
            call.reject("writing the file failed: \(error.localizedDescription)")
        }
    }

    @objc func close(_ call: CAPPluginCall) {
        guard let sink = sinkOf(call) else { return }
        let id = call.getString("id") ?? ""
        _ = queue.sync { open.removeValue(forKey: id) }
        defer { Self.release(sink) }
        do {
            try sink.handle.close()
            call.resolve()
        } catch {
            call.reject("saving the file failed: \(error.localizedDescription)")
        }
    }

    @objc func abort(_ call: CAPPluginCall) {
        let id = call.getString("id") ?? ""
        if let sink = queue.sync(execute: { open.removeValue(forKey: id) }) {
            try? sink.handle.close()
            try? FileManager.default.removeItem(at: sink.file)
            Self.release(sink)
        }
        call.resolve()
    }

    /// The choice under way, which this ends.
    private func takeChoice() -> (call: CAPPluginCall, name: String)? {
        queue.sync {
            defer { choosing = nil }
            return choosing
        }
    }

    private func sinkOf(_ call: CAPPluginCall) -> Sink? {
        let id = call.getString("id") ?? ""
        let sink = queue.sync { open[id] }
        if sink == nil {
            call.reject("no such file is open")
        }
        return sink
    }

    private static func release(_ sink: Sink) {
        if sink.accessing {
            sink.folder.stopAccessingSecurityScopedResource()
        }
    }

    /// `name` in `folder`, or `name (2)`, `name (3)`, and on, before its extension, when taken.
    private static func freeName(_ name: String, in folder: URL) -> URL {
        let wanted = folder.appendingPathComponent(name)
        guard FileManager.default.fileExists(atPath: wanted.path) else { return wanted }
        let stem = wanted.deletingPathExtension().lastPathComponent
        let ext = wanted.pathExtension
        var n = 2
        while true {
            let numbered = ext.isEmpty ? "\(stem) (\(n))" : "\(stem) (\(n)).\(ext)"
            let candidate = folder.appendingPathComponent(numbered)
            if !FileManager.default.fileExists(atPath: candidate.path) { return candidate }
            n += 1
        }
    }
}
