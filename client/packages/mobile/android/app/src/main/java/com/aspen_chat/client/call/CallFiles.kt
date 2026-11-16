package com.aspen_chat.client.call

import android.content.ContentResolver
import android.content.Context
import com.aspen_chat.client.files.DocumentSink
import com.aspen_chat.client.files.FileRegistry
import com.aspen_chat.client.voice.CreatedSink
import com.aspen_chat.client.voice.FileException
import com.aspen_chat.client.voice.FileSink
import com.aspen_chat.client.voice.FileSource
import com.aspen_chat.client.voice.Files
import com.aspen_chat.client.voice.OpenedFile
import java.io.IOException
import java.io.InputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The call engine's files (`Files`, `FileSource`, `FileSink`) over `FileRegistry`, where the
 * `AspenFiles` plugin puts what the user picks: a picked document is opened anew for each
 * transfer of it and read a chunk at a time from its start, and a received document is written a
 * chunk at a time into the place the picker chose, kept when every byte is in and deleted
 * otherwise. The bytes never cross to the page. Reads and writes run on the IO dispatcher, off
 * the engine's thread, since a provider may be slow.
 */
class CallFiles(context: Context, private val registry: FileRegistry = FileRegistry.shared()) : Files {
    private val resolver: ContentResolver = context.applicationContext.contentResolver

    override suspend fun openFile(reference: String): OpenedFile {
        val uri = registry.picked(reference) ?: throw FileException.Failed("the file offered is no longer held")
        return withContext(Dispatchers.IO) {
            try {
                val size = resolver.openFileDescriptor(uri, "r")?.use { it.statSize }
                    ?: throw IOException("the file could not be opened")
                val stream = resolver.openInputStream(uri) ?: throw IOException("the file could not be opened")
                OpenedFile(CallFileSource(stream, size.toULong()))
            } catch (e: Exception) {
                throw FileException.Failed("the file could not be opened: ${e.message}")
            }
        }
    }

    override fun release(reference: String) {
        registry.releasePicked(reference, resolver)
    }

    override suspend fun createSink(destination: String): CreatedSink {
        val sink = registry.received(destination)
            ?: throw FileException.Failed("the place chosen for the file is no longer open")
        return CreatedSink(CallFileSink(sink, destination, registry))
    }
}

/** One opening of a picked document, read from its start. */
class CallFileSource(private val stream: InputStream, private val fileSize: ULong) : FileSource {
    private var closed = false

    override fun size(): ULong = fileSize

    override suspend fun read(maxBytes: ULong): ByteArray = withContext(Dispatchers.IO) {
        val buffer = ByteArray(minOf(maxBytes, MOST_PER_READ).toInt())
        try {
            // A stream may answer fewer bytes than asked before its end; the engine asks again.
            val read = stream.read(buffer)
            if (read <= 0) ByteArray(0) else buffer.copyOf(read)
        } catch (e: IOException) {
            throw FileException.Failed("the file could not be read: ${e.message}")
        }
    }

    override fun closeSource() {
        synchronized(this) {
            if (closed) {
                return
            }
            closed = true
        }
        try {
            stream.close()
        } catch (ignored: IOException) {
            // Nothing more is read from it.
        }
    }

    private companion object {
        /** The most read at once (1 MiB), however much the engine asks for. */
        const val MOST_PER_READ = 1_048_576uL
    }
}

/**
 * A file being received into the document the picker chose, which the registry forgets once it
 * is finished or discarded.
 */
class CallFileSink(
    private val sink: DocumentSink,
    private val id: String,
    private val registry: FileRegistry,
) : FileSink {
    override suspend fun write(data: ByteArray) = withContext(Dispatchers.IO) {
        try {
            sink.write(data)
        } catch (e: IOException) {
            throw FileException.Failed("the file could not be written: ${e.message}")
        }
    }

    override suspend fun finish() = withContext(Dispatchers.IO) {
        registry.removeReceived(id)
        try {
            sink.close()
        } catch (e: IOException) {
            throw FileException.Failed("the file could not be kept: ${e.message}")
        }
    }

    override suspend fun discard() = withContext(Dispatchers.IO) {
        registry.removeReceived(id)
        sink.abort()
    }
}
