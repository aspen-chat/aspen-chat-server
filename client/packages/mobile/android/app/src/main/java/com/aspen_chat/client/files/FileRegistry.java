package com.aspen_chat.client.files;

import android.content.ContentResolver;
import android.content.Intent;
import android.net.Uri;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The files a call's transfers read and write, by the ids the page passes around and the native
 * side alone resolves: a document the system's picker chose to offer, held with the read
 * permission taken on it until released, and a document the picker chose for a file being
 * received (`DocumentSink`). `AspenFilesPlugin` puts them here as the user chooses, and writes
 * received files for the web's own calls; the call engine's `CallFiles` reads and writes them for
 * a native call. Kept in this package, which every build has, since the call layer is left out
 * of a build without WebRTC and the engine.
 */
public final class FileRegistry {

    private static final FileRegistry SHARED = new FileRegistry();

    public static FileRegistry shared() {
        return SHARED;
    }

    private final Map<String, Uri> picked = new ConcurrentHashMap<>();
    private final Map<String, DocumentSink> received = new ConcurrentHashMap<>();

    /** Holds `uri`, a document the picker chose to offer; the reference it is named by. */
    public String addPicked(Uri uri) {
        String reference = UUID.randomUUID().toString();
        picked.put(reference, uri);
        return reference;
    }

    /** The document the picked file `reference` names, or `null`. */
    public Uri picked(String reference) {
        return picked.get(reference);
    }

    /**
     * Lets the picked file `reference` names go, giving back the persistable read permission
     * taken on it when it was picked, where one was.
     */
    public void releasePicked(String reference, ContentResolver resolver) {
        Uri uri = picked.remove(reference);
        if (uri == null) {
            return;
        }
        try {
            resolver.releasePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException notPersisted) {
            // The provider granted no persistable permission to give back.
        }
    }

    /** Holds `sink`, a document being received into; the id it is written to by. */
    public String addReceived(DocumentSink sink) {
        String id = UUID.randomUUID().toString();
        received.put(id, sink);
        return id;
    }

    /** The received file `id` names, or `null`. */
    public DocumentSink received(String id) {
        return received.get(id);
    }

    /** Forgets the received file `id` names, once finished or discarded; what it was. */
    public DocumentSink removeReceived(String id) {
        return received.remove(id);
    }

    /**
     * Discards every received file still open, as when the app is ending with transfers under
     * way: each holds only part of what was sent.
     */
    public void discardAllReceived() {
        List<DocumentSink> open = new ArrayList<>(received.values());
        received.clear();
        for (DocumentSink sink : open) {
            sink.abort();
        }
    }
}
