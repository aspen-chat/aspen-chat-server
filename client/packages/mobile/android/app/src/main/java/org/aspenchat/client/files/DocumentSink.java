package org.aspenchat.client.files;

import android.content.ContentResolver;
import android.net.Uri;
import android.provider.DocumentsContract;
import java.io.BufferedOutputStream;
import java.io.IOException;
import java.io.OutputStream;

/**
 * A file being received into a document the user chose (`AspenFilesPlugin`): written as the
 * transfer delivers it, kept once closed, and deleted when the transfer ends otherwise.
 */
public final class DocumentSink {

    /** How much is gathered before a write reaches the document's provider. */
    private static final int BUFFER_BYTES = 1 << 20;

    private final ContentResolver resolver;
    private final Uri uri;
    private final OutputStream out;
    private boolean finished;

    private DocumentSink(ContentResolver resolver, Uri uri, OutputStream out) {
        this.resolver = resolver;
        this.uri = uri;
        this.out = out;
    }

    /** Opens `uri` for writing from the start, replacing anything already in it. */
    public static DocumentSink open(ContentResolver resolver, Uri uri) throws IOException {
        OutputStream out = resolver.openOutputStream(uri, "wt");
        if (out == null) {
            throw new IOException("the document could not be opened for writing");
        }
        return new DocumentSink(resolver, uri, new BufferedOutputStream(out, BUFFER_BYTES));
    }

    public Uri uri() {
        return uri;
    }

    public synchronized void write(byte[] data) throws IOException {
        if (finished) {
            throw new IOException("the document is closed");
        }
        out.write(data);
    }

    /** Keeps what was written. */
    public synchronized void close() throws IOException {
        if (finished) {
            return;
        }
        finished = true;
        out.close();
    }

    /** Discards the document: it held only part of a file. */
    public synchronized void abort() {
        if (!finished) {
            finished = true;
            try {
                out.close();
            } catch (IOException ignored) {
                // It is deleted next whether or not it closed cleanly.
            }
        }
        try {
            if (DocumentsContract.deleteDocument(resolver, uri)) {
                return;
            }
        } catch (Exception ignored) {
            // Not a document of a documents provider; deleted as a plain content row below.
        }
        try {
            resolver.delete(uri, null, null);
        } catch (Exception ignored) {
            // Nothing more can be done; the user may delete it themself.
        }
    }
}
