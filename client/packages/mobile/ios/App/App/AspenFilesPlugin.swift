import Capacitor
import UIKit
import UniformTypeIdentifiers

/// The files a call sends and receives, as the page reaches them (`AspenFiles` in the app's
/// `src/api/filesBridge.ts`, which Android's plugin of the same name answers too). Receiving: a
/// place is chosen before the file arrives. The only picker iOS has that names a file and asks
/// where it goes is the one that moves an existing file out of the app, so an empty file is made
/// under the name the file was sent with and handed to that picker; where the user has it put is
/// then written in place, by the page (`write`, in base64, until it closes or aborts the file) for
/// a call in the web view, or by the call engine for a native call, which is given the same id
/// (`CallFiles`). The picker itself asks what to do when the place holds a file of that name.
/// Offering: the system's picker chooses a file (`pick`), which is held by reference in
/// `FileRegistry` with its security-scoped access for the engine to read, until `release`; its
/// bytes never cross the bridge. An aborted received file is removed, since it holds only part
/// of what was sent.
@objc(AspenFilesPlugin)
public class AspenFilesPlugin: CAPPlugin, CAPBridgedPlugin, UIDocumentPickerDelegate {
    public let identifier = "AspenFilesPlugin"
    public let jsName = "AspenFiles"
    public let pluginMethods: [CAPPluginMethod] = [
        CAPPluginMethod(name: "create", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "write", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "close", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "abort", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "pick", returnType: CAPPluginReturnPromise),
        CAPPluginMethod(name: "release", returnType: CAPPluginReturnPromise),
    ]

    /// Which picker is up: the one placing a file to receive, with the directory its empty file
    /// was made in, or the one choosing a file to offer.
    private enum Choice {
        case placing(holding: URL)
        case picking
    }

    /// The choice under way; touched only on `queue`. One picker is up at a time.
    private let queue = DispatchQueue(label: "com.aspen-chat.client.files")
    private var choosing: (call: CAPPluginCall, choice: Choice)?
    private let registry = FileRegistry.shared

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
        guard begin(call, .placing(holding: holding)) else {
            try? FileManager.default.removeItem(at: holding)
            return
        }
        present(UIDocumentPickerViewController(forExporting: [empty], asCopy: false))
    }

    /// Asks for a file to offer; answers its reference, name, and size, or a `null` reference
    /// when the user backed out.
    @objc func pick(_ call: CAPPluginCall) {
        guard begin(call, .picking) else {
            return
        }
        present(UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: false))
    }

    /// Takes the choice slot for `call`, or rejects it when another choice is under way.
    private func begin(_ call: CAPPluginCall, _ choice: Choice) -> Bool {
        let free = queue.sync { () -> Bool in
            guard choosing == nil else { return false }
            choosing = (call, choice)
            return true
        }
        if !free {
            call.reject("another file is being chosen")
        }
        return free
    }

    private func present(_ picker: UIDocumentPickerViewController) {
        DispatchQueue.main.async {
            guard let presenter = self.bridge?.viewController else {
                self.endChoice()?.call.reject("there is no screen to ask on")
                return
            }
            picker.delegate = self
            presenter.present(picker, animated: true)
        }
    }

    public func documentPicker(
        _ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]
    ) {
        guard let (call, choice) = endChoice() else { return }
        guard let file = urls.first else {
            call.resolve(backedOut(choice))
            return
        }
        switch choice {
        case .placing:
            do {
                call.resolve(["id": try registry.addReceived(file)])
            } catch {
                removePlaced(file)
                call.reject("the file could not be opened where it was saved: \(error.localizedDescription)")
            }
        case .picking:
            let reference = registry.addPicked(file)
            let size: Int
            do {
                guard let fileSize = try file.resourceValues(forKeys: [.fileSizeKey]).fileSize else {
                    throw CocoaError(.fileReadUnknown)
                }
                size = fileSize
            } catch {
                registry.releasePicked(reference)
                call.reject("the file's size could not be read: \(error.localizedDescription)")
                return
            }
            call.resolve(["reference": reference, "name": file.lastPathComponent, "size": size])
        }
    }

    public func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        if let (call, choice) = endChoice() {
            call.resolve(backedOut(choice))
        }
    }

    /// What a choice the user backed out of answers: no id to write to, or no reference.
    private func backedOut(_ choice: Choice) -> JSObject {
        switch choice {
        case .placing: return ["id": NSNull()]
        case .picking: return ["reference": NSNull()]
        }
    }

    @objc func write(_ call: CAPPluginCall) {
        guard let file = receivedOf(call) else { return }
        guard let base64 = call.getString("data"), let data = Data(base64Encoded: base64) else {
            call.reject("data must be base64")
            return
        }
        do {
            try file.write(data)
            call.resolve()
        } catch {
            call.reject("writing the file failed: \(error.localizedDescription)")
        }
    }

    @objc func close(_ call: CAPPluginCall) {
        guard let file = receivedOf(call) else { return }
        registry.removeReceived(call.getString("id") ?? "")
        do {
            try file.finish()
            call.resolve()
        } catch {
            call.reject("saving the file failed: \(error.localizedDescription)")
        }
    }

    @objc func abort(_ call: CAPPluginCall) {
        let id = call.getString("id") ?? ""
        if let file = registry.received(id) {
            registry.removeReceived(id)
            file.discard()
        }
        call.resolve()
    }

    /// Lets go of a picked file the page did not offer.
    @objc func release(_ call: CAPPluginCall) {
        guard let reference = call.getString("reference"), !reference.isEmpty else {
            call.reject("reference is required")
            return
        }
        registry.releasePicked(reference)
        call.resolve()
    }

    /// Ends the choice under way, giving its call and choice to be answered. The directory a
    /// placing's empty file was made in goes too, with the file when the picker did not move it
    /// out.
    private func endChoice() -> (call: CAPPluginCall, choice: Choice)? {
        let choice = queue.sync { () -> (call: CAPPluginCall, choice: Choice)? in
            defer { choosing = nil }
            return choosing
        }
        guard let choice else { return nil }
        if case .placing(let holding) = choice.choice {
            try? FileManager.default.removeItem(at: holding)
        }
        return choice
    }

    private func receivedOf(_ call: CAPPluginCall) -> FileRegistry.ReceivedFile? {
        let file = registry.received(call.getString("id") ?? "")
        if file == nil {
            call.reject("no such file is open")
        }
        return file
    }

    /// Removes a file the picker placed that could not be opened, coordinated so that the
    /// provider of where it lies hears of it.
    private func removePlaced(_ file: URL) {
        var coordinating: NSError?
        NSFileCoordinator().coordinate(writingItemAt: file, options: .forDeleting, error: &coordinating) {
            try? FileManager.default.removeItem(at: $0)
        }
    }
}
