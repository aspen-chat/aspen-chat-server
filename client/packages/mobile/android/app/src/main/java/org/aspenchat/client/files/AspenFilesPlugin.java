package org.aspenchat.client.files;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.util.Base64;
import androidx.activity.result.ActivityResult;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.ActivityCallback;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.IOException;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Receiving a file into a place the user chose before it arrives (`AspenFiles` in the app's
 * `src/api/filesBridge.ts`): the system's create-document picker names the file, and the page
 * writes what arrives to it by id, in base64, until it closes or aborts it.
 */
@CapacitorPlugin(name = "AspenFiles")
public class AspenFilesPlugin extends Plugin {

    private final Map<String, DocumentSink> open = new ConcurrentHashMap<>();

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
            String id = UUID.randomUUID().toString();
            open.put(id, DocumentSink.open(getContext().getContentResolver(), uri));
            answer.put("id", id);
            call.resolve(answer);
        } catch (IOException e) {
            call.reject("the chosen file could not be opened: " + e.getMessage());
        }
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
        open.remove(call.getString("id"));
        try {
            sink.close();
            call.resolve();
        } catch (IOException e) {
            call.reject("saving the file failed: " + e.getMessage());
        }
    }

    @PluginMethod
    public void abort(PluginCall call) {
        DocumentSink sink = open.remove(call.getString("id", ""));
        if (sink != null) {
            sink.abort();
        }
        call.resolve();
    }

    private DocumentSink sinkOf(PluginCall call) {
        DocumentSink sink = open.get(call.getString("id", ""));
        if (sink == null) {
            call.reject("no such file is open");
        }
        return sink;
    }

    /** A file still open when the app goes held only part of what was sent. */
    @Override
    protected void handleOnDestroy() {
        for (DocumentSink sink : open.values()) {
            sink.abort();
        }
        open.clear();
    }
}
