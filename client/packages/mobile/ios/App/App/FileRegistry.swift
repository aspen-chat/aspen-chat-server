import Foundation

/// The files a call's transfers read and write, by the ids the page passes around and the native
/// side alone resolves: a file the system's picker chose to offer (`PickedFile`, held with its
/// security-scoped access until released), and a place the picker chose for a file being
/// received (`ReceivedFile`, written in place and kept or removed). `AspenFilesPlugin` puts them
/// here as the user chooses, and writes received files for the web's own calls; the call
/// engine's `CallFiles` reads and writes them for a native call. Everything is touched under one
/// lock, from any thread.
final class FileRegistry {
    static let shared = FileRegistry()

    /// A file the picker chose to offer, outside the app's sandbox, reached only while its
    /// security-scoped access is held; `release` gives that back once nothing offers or sends it.
    final class PickedFile {
        let url: URL
        private var accessing: Bool

        fileprivate init(url: URL, accessing: Bool) {
            self.url = url
            self.accessing = accessing
        }

        fileprivate func release() {
            if accessing {
                url.stopAccessingSecurityScopedResource()
                accessing = false
            }
        }
    }

    /// A file being received where the picker put it: written as the transfer delivers it,
    /// kept once finished, and removed when the transfer ends otherwise, since it holds only part
    /// of what was sent.
    final class ReceivedFile {
        let url: URL
        private let handle: FileHandle
        private var accessing: Bool
        private var ended = false
        private let lock = NSLock()

        fileprivate init(url: URL, handle: FileHandle, accessing: Bool) {
            self.url = url
            self.handle = handle
            self.accessing = accessing
        }

        func write(_ data: Data) throws {
            lock.lock()
            defer { lock.unlock() }
            guard !ended else {
                throw FileRegistryError.ended
            }
            try handle.write(contentsOf: data)
        }

        /// Every byte is in: keeps the file.
        func finish() throws {
            lock.lock()
            defer { lock.unlock() }
            guard !ended else { return }
            ended = true
            defer { release() }
            try handle.close()
        }

        /// The transfer did not finish: removes the file. Discarding a finished or discarded
        /// file does nothing.
        func discard() {
            lock.lock()
            defer { lock.unlock() }
            guard !ended else { return }
            ended = true
            try? handle.close()
            Self.remove(url)
            release()
        }

        private func release() {
            if accessing {
                url.stopAccessingSecurityScopedResource()
                accessing = false
            }
        }

        /// Removes a file the picker placed, coordinated so that the provider of where it lies
        /// (iCloud Drive, another app's storage) hears of it.
        private static func remove(_ file: URL) {
            var coordinating: NSError?
            NSFileCoordinator().coordinate(writingItemAt: file, options: .forDeleting, error: &coordinating) {
                try? FileManager.default.removeItem(at: $0)
            }
        }
    }

    private let lock = NSLock()
    private var picked: [String: PickedFile] = [:]
    private var received: [String: ReceivedFile] = [:]

    /// Holds `url`, which the picker chose to offer, taking its security-scoped access; the
    /// reference the page and the engine name it by.
    func addPicked(_ url: URL) -> String {
        let accessing = url.startAccessingSecurityScopedResource()
        let reference = UUID().uuidString
        lock.lock()
        picked[reference] = PickedFile(url: url, accessing: accessing)
        lock.unlock()
        return reference
    }

    func picked(_ reference: String) -> PickedFile? {
        lock.lock()
        defer { lock.unlock() }
        return picked[reference]
    }

    /// Lets the picked file `reference` names go, with its access.
    func releasePicked(_ reference: String) {
        lock.lock()
        let file = picked.removeValue(forKey: reference)
        lock.unlock()
        file?.release()
    }

    /// Opens `url`, where the picker put the empty file a transfer writes into, taking its
    /// security-scoped access; the id the page and the engine write to it by.
    func addReceived(_ url: URL) throws -> String {
        let accessing = url.startAccessingSecurityScopedResource()
        let handle: FileHandle
        do {
            handle = try FileHandle(forWritingTo: url)
        } catch {
            if accessing {
                url.stopAccessingSecurityScopedResource()
            }
            throw error
        }
        let id = UUID().uuidString
        lock.lock()
        received[id] = ReceivedFile(url: url, handle: handle, accessing: accessing)
        lock.unlock()
        return id
    }

    func received(_ id: String) -> ReceivedFile? {
        lock.lock()
        defer { lock.unlock() }
        return received[id]
    }

    /// Forgets the received file `id` names, once finished or discarded.
    func removeReceived(_ id: String) {
        lock.lock()
        received[id] = nil
        lock.unlock()
    }

    /// Discards every received file still open, as when the app is ending with transfers
    /// under way: each holds only part of what was sent.
    func discardAllReceived() {
        lock.lock()
        let open = Array(received.values)
        received.removeAll()
        lock.unlock()
        for file in open {
            file.discard()
        }
    }
}

enum FileRegistryError: LocalizedError {
    /// Written after it was finished or discarded.
    case ended

    var errorDescription: String? {
        switch self {
        case .ended: return "the file is no longer open"
        }
    }
}
