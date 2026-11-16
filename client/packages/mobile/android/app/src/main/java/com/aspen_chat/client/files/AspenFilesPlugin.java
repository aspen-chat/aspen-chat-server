package com.aspen_chat.client.files;

import android.app.Activity;
import android.content.ContentResolver;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;
import android.util.Base64;
import androidx.activity.result.ActivityResult;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.ActivityCallback;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.IOException;

/**
 * The files a call sends and receives, as the page reaches them (`AspenFiles` in the app's
 * `src/api/filesBridge.ts`). Receiving: the system's create-document picker names the file
 * before it arrives, and it is written to by id as it comes, by the page (`write`, in base64,
 * until it closes or aborts it) for a call in the web view, or by the call engine for a native
 * call, which is given the same id (`CallFiles`). Offering: the system's open-document picker
 * chooses a file (`pick`), which is held by reference in `FileRegistry` with a persistable read
 * permission for the engine to read, until `release`; its bytes never cross the bridge.
 */
@CapacitorPlugin(name = "AspenFiles")
public class AspenFilesPlugin extends Plugin {

    private final FileRegistry registry = FileRegistry.shared();

    /** Asks where to save `name`; answers an id to write to, or `null` when the user backed out. */
    @PluginMethod
    public void create(PluginCall call) {
        String name = call.getString("name");
        if (name == null || name.isEmpty()) {
            call.reject("name is required");
            return;
        }
        Intent intent = new Intent(Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType(call.getString("mimeType", "application/octet-stream"));
        intent.putExtra(Intent.EXTRA_TITLE, name);
        startActivityForResult(call, intent, "created");
    }

    @ActivityCallback
    private void created(PluginCall call, ActivityResult result) {
        JSObject answer = new JSObject();
        Intent data = result.getData();
        Uri uri = data == null ? null : data.getData();
        if (result.getResultCode() != Activity.RESULT_OK || uri == null) {
            answer.put("id", JSObject.NULL);
            call.resolve(answer);
            return;
        }
        try {
            answer.put("id", registry.addReceived(DocumentSink.open(getContext().getContentResolver(), uri)));
            call.resolve(answer);
        } catch (IOException e) {
            call.reject("the chosen file could not be opened: " + e.getMessage());
        }
    }

    /**
     * Asks for a file to offer; answers its reference, name, and size, or a `null` reference when
     * the user backed out. The read permission is taken as persistable where the provider allows,
     * so the engine can read the file after this activity is gone, and given back on `release`.
     */
    @PluginMethod
    public void pick(PluginCall call) {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(call, intent, "picked");
    }

    @ActivityCallback
    private void picked(PluginCall call, ActivityResult result) {
        JSObject answer = new JSObject();
        Intent data = result.getData();
        Uri uri = data == null ? null : data.getData();
        if (result.getResultCode() != Activity.RESULT_OK || uri == null) {
            answer.put("reference", JSObject.NULL);
            call.resolve(answer);
            return;
        }
        ContentResolver resolver = getContext().getContentResolver();
        try {
            resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION);
        } catch (SecurityException notPersistable) {
            // The provider grants the permission for this activity's life alone, which is as
            // long as the page can offer the file.
        }
        String name = uri.getLastPathSegment();
        long size = -1;
        try (Cursor cursor = resolver.query(uri, new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                int nameAt = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                int sizeAt = cursor.getColumnIndex(OpenableColumns.SIZE);
                if (nameAt >= 0 && !cursor.isNull(nameAt)) {
                    name = cursor.getString(nameAt);
                }
                if (sizeAt >= 0 && !cursor.isNull(sizeAt)) {
                    size = cursor.getLong(sizeAt);
                }
            }
        }
        if (size < 0) {
            // A provider that does not say the size: the open file does.
            try (ParcelFileDescriptor descriptor = resolver.openFileDescriptor(uri, "r")) {
                if (descriptor == null) {
                    throw new IOException("the file could not be opened");
                }
                size = descriptor.getStatSize();
            } catch (IOException e) {
                call.reject("the file's size could not be read: " + e.getMessage());
                return;
            }
        }
        if (name == null || name.isEmpty()) {
            name = "file";
        }
        answer.put("reference", registry.addPicked(uri));
        answer.put("name", name);
        answer.put("size", size);
        call.resolve(answer);
    }

    @PluginMethod
    public void write(PluginCall call) {
        DocumentSink sink = sinkOf(call);
        String data = call.getString("data");
        if (sink == null || data == null) {
            return;
        }
        try {
            sink.write(Base64.decode(data, Base64.DEFAULT));
            call.resolve();
        } catch (IOException | IllegalArgumentException e) {
            call.reject("writing the file failed: " + e.getMessage());
        }
    }

    @PluginMethod
    public void close(PluginCall call) {
        DocumentSink sink = sinkOf(call);
        if (sink == null) {
            return;
        }
        registry.removeReceived(call.getString("id"));
        try {
            sink.close();
            call.resolve();
        } catch (IOException e) {
            call.reject("saving the file failed: " + e.getMessage());
        }
    }

    @PluginMethod
    public void abort(PluginCall call) {
        DocumentSink sink = registry.removeReceived(call.getString("id", ""));
        if (sink != null) {
            sink.abort();
        }
        call.resolve();
    }

    /** Lets go of a picked file the page did not offer. */
    @PluginMethod
    public void release(PluginCall call) {
        String reference = call.getString("reference");
        if (reference == null || reference.isEmpty()) {
            call.reject("reference is required");
            return;
        }
        registry.releasePicked(reference, getContext().getContentResolver());
        call.resolve();
    }

    private DocumentSink sinkOf(PluginCall call) {
        DocumentSink sink = registry.received(call.getString("id", ""));
        if (sink == null) {
            call.reject("no such file is open");
        }
        return sink;
    }

    /** A file still open when the app goes held only part of what was sent. */
    @Override
    protected void handleOnDestroy() {
        registry.discardAllReceived();
    }
}
