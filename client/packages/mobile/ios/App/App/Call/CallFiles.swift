import Foundation

/// The call engine's files (`Files`, `FileSource`, `FileSink`) over `FileRegistry`, where the
/// `AspenFiles` plugin puts what the user picks: a picked file is opened anew for each transfer
/// of it and read a chunk at a time from its start, and a received file is written a chunk at a
/// time into the place the picker chose, kept when every byte is in and removed otherwise. The
/// bytes never cross to the page. Reads and writes run off the engine's thread, on a task of
/// their own, since the disk may be slow.
final class CallFiles: Files, @unchecked Sendable {
    private let registry: FileRegistry

    init(registry: FileRegistry = .shared) {
        self.registry = registry
    }

    func openFile(reference: String) async throws -> OpenedFile {
        guard let picked = registry.picked(reference) else {
            throw FileError.Failed(detail: "the file offered is no longer held")
        }
        return try await Task.detached {
            do {
                let handle = try FileHandle(forReadingFrom: picked.url)
                let attributes = try FileManager.default.attributesOfItem(atPath: picked.url.path)
                let size = (attributes[.size] as? NSNumber)?.uint64Value ?? 0
                return OpenedFile(source: CallFileSource(handle: handle, size: size))
            } catch {
                throw FileError.Failed(detail: "the file could not be opened: \(error.localizedDescription)")
            }
        }.value
    }

    func release(reference: String) {
        registry.releasePicked(reference)
    }

    func createSink(destination: String) async throws -> CreatedSink {
        guard let received = registry.received(destination) else {
            throw FileError.Failed(detail: "the place chosen for the file is no longer open")
        }
        return CreatedSink(sink: CallFileSink(file: received, id: destination, registry: registry))
    }
}

/// One opening of a picked file, read from its start.
final class CallFileSource: FileSource, @unchecked Sendable {
    private let handle: FileHandle
    private let fileSize: UInt64
    private let lock = NSLock()
    private var closed = false

    init(handle: FileHandle, size: UInt64) {
        self.handle = handle
        self.fileSize = size
    }

    func size() -> UInt64 {
        fileSize
    }

    func read(maxBytes: UInt64) async throws -> Data {
        let handle = self.handle
        return try await Task.detached {
            do {
                return try handle.read(upToCount: Int(clamping: maxBytes)) ?? Data()
            } catch {
                throw FileError.Failed(detail: "the file could not be read: \(error.localizedDescription)")
            }
        }.value
    }

    func closeSource() {
        lock.lock()
        defer { lock.unlock() }
        guard !closed else { return }
        closed = true
        try? handle.close()
    }
}

/// A file being received into the place the picker chose, which the registry forgets once it is
/// finished or discarded.
final class CallFileSink: FileSink, @unchecked Sendable {
    private let file: FileRegistry.ReceivedFile
    private let id: String
    private let registry: FileRegistry

    init(file: FileRegistry.ReceivedFile, id: String, registry: FileRegistry) {
        self.file = file
        self.id = id
        self.registry = registry
    }

    func write(data: Data) async throws {
        let file = self.file
        try await Task.detached {
            do {
                try file.write(data)
            } catch {
                throw FileError.Failed(detail: "the file could not be written: \(error.localizedDescription)")
            }
        }.value
    }

    func finish() async throws {
        registry.removeReceived(id)
        let file = self.file
        try await Task.detached {
            do {
                try file.finish()
            } catch {
                throw FileError.Failed(detail: "the file could not be kept: \(error.localizedDescription)")
            }
        }.value
    }

    func discard() async {
        registry.removeReceived(id)
        let file = self.file
        await Task.detached {
            file.discard()
        }.value
    }
}
