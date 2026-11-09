import Capacitor
import UIKit

/// Receiving a file into a place the user chose before it arrives (`AspenFiles` in the app's
/// `src/api/filesBridge.ts`, which Android's plugin of the same name answers too). The only picker
/// iOS has that names a file and asks where it goes is the one that moves an existing file out of
/// the app, so an empty file is made under the name the file was sent with and handed to that
/// picker; where the user has it put is then written in place. The picker itself asks what to do
/// when the place holds a file of that name. The page writes what arrives by id, in base64, until
/// it closes or aborts the file; an aborted file is removed, since it holds only part of what was
/// sent.
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

    /// A file being received: where the picker put it, its handle, and whether access to it was
    /// taken and so must be given back.
    private struct Sink {
        let file: URL
        let handle: FileHandle
        let accessing: Bool
    }

    /// Every file open, and the choice under way with the directory its empty file was made in;
    /// touched only on `queue`.
    private let queue = DispatchQueue(label: "com.aspen-chat.client.files")
    private var open: [String: Sink] = [:]
    private var choosing: (call: CAPPluginCall, holding: URL)?

    /// Asks where to save `name`; answers an id to write to, or `null` when the user backed out.
    @objc func create(_ call: CAPPluginCall) {
        guard let name = call.getString("name"), !name.isEmpty else {
            call.reject("name is required")
            return
        }
        // A directory of its own, so the empty file can carry the name whatever else is held.
        let holding = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let empty = holding.appendingPathComponent(name, isDirectory: false)
        do {
            try FileManager.default.createDirectory(at: holding, withIntermediateDirectories: true)
            try Data().write(to: empty)
        } catch {
            try? FileManager.default.removeItem(at: holding)
            call.reject("the file could not be made: \(error.localizedDescription)")
            return
        }
        let free = queue.sync { () -> Bool in
            guard choosing == nil else { return false }
            choosing = (call, holding)
            return true
        }
        guard free else {
            try? FileManager.default.removeItem(at: holding)
            call.reject("another file's place is being chosen")
            return
        }
        DispatchQueue.main.async {
            guard let presenter = self.bridge?.viewController else {
                self.endChoice()?.reject("there is no screen to ask on")
                return
            }
            let picker = UIDocumentPickerViewController(forExporting: [empty], asCopy: false)
            picker.delegate = self
            presenter.present(picker, animated: true)
        }
    }

    public func documentPicker(
        _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
    ) {
        guard let call = endChoice() else { return }
        guard let file = urls.first else {
            call.resolve(["id": NSNull()])
            return
        }
        // The moved file is outside the app's sandbox, reached only while this access is held.
        let accessing = file.startAccessingSecurityScopedResource()
        do {
            let handle = try FileHandle(forWritingTo: file)
            let id = UUID().uuidString
            let sink = Sink(file: file, handle: handle, accessing: accessing)
            queue.sync { open[id] = sink }
            call.resolve(["id": id])
        } catch {
            Self.remove(file)
            if accessing {
                file.stopAccessingSecurityScopedResource()
            }
            call.reject("the file could not be opened where it was saved: \(error.localizedDescription)")
        }
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        endChoice()?.resolve(["id": NSNull()])
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
            Self.remove(sink.file)
            Self.release(sink)
        }
        call.resolve()
    }

    /// Ends the choice under way, giving its call to be answered. The directory its empty file
    /// was made in goes too, with the file when the picker did not move it out.
    private func endChoice() -> CAPPluginCall? {
        let choice = queue.sync { () -> (call: CAPPluginCall, holding: URL)? in
            defer { choosing = nil }
            return choosing
        }
        guard let choice else { return nil }
        try? FileManager.default.removeItem(at: choice.holding)
        return choice.call
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
            sink.file.stopAccessingSecurityScopedResource()
        }
    }

    /// Removes a file the picker placed, coordinated so that the provider of where it lies (iCloud
    /// Drive, another app's storage) hears of it.
    private static func remove(_ file: URL) {
        var coordinating: NSError?
        NSFileCoordinator().coordinate(writingItemAt: file, options: .forDeleting, error: &coordinating) {
            try? FileManager.default.removeItem(at: $0)
        }
    }
}
