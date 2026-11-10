package com.aspen_chat.client.uploads;

import android.content.Context;
import android.util.Base64;
import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.IOException;
import java.util.Locale;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * Sending attachments from the app's native side (`AspenUploads` in the app's
 * `src/api/uploadsBridge.ts`), so that they arrive, and are confirmed with their deployment,
 * after the app has left the screen and its page has stopped. The page hands over each file's
 * bytes in base64 and says where they go; {@link Uploads} keeps and sends them, and
 * {@link UploadService} keeps the process alive meanwhile. This plugin is only the page's way
 * in: what it starts outlives it.
 */
@CapacitorPlugin(name = "AspenUploads")
public class AspenUploadsPlugin extends Plugin implements Uploads.Listener {

    /**
     * Runs every call, in the order the page made them, on a thread of its own: a file's bytes
     * are decoded and written to storage there, which the bridge's one thread for every
     * plugin's calls would make the others wait on.
     */
    private final ExecutorService calls = Executors.newSingleThreadExecutor();
    /** Set by the first task on `calls`, so every call's task finds it. */
    private volatile Uploads uploads;

    private interface Work {
        void run() throws IOException;
    }

    @Override
    public void load() {
        calls.execute(() -> {
            uploads = DeviceUploads.of(getContext());
            uploads.listen(this);
        });
    }

    @PluginMethod
    public void begin(PluginCall call) {
        String id = idOf(call);
        if (id == null) {
            return;
        }
        work(call, "the upload could not be begun", () -> {
            uploads.begin(id);
            UploadNotices.clear(getContext(), id);
            call.resolve();
        });
    }

    @PluginMethod
    public void write(PluginCall call) {
        String id = idOf(call);
        if (id == null) {
            return;
        }
        String data = call.getString("data");
        if (data == null) {
            call.reject("data is required");
            return;
        }
        work(call, "the upload's bytes could not be kept", () -> {
            uploads.write(id, Base64.decode(data, Base64.DEFAULT));
            call.resolve();
        });
    }

    @PluginMethod
    public void send(PluginCall call) {
        String id = idOf(call);
        if (id == null) {
            return;
        }
        UploadRecord record = new UploadRecord();
        record.id = id;
        record.origin = call.getString("origin");
        record.userId = call.getString("userId");
        record.sessionToken = call.getString("sessionToken");
        record.refreshToken = call.getString("refreshToken");
        record.uploadUrl = call.getString("uploadUrl");
        record.contentType = call.getString("contentType");
        record.name = call.getString("name", "");
        record.mimeType = call.getString("mimeType", "");
        record.byteSize = call.getData().optLong("byteSize", -1);
        record.acceptLanguage = call.getString("acceptLanguage", "");
        record.failedTitle = call.getString("failedTitle", "");
        record.failedBody = call.getString("failedBody", "");
        record.sendingTitle = call.getString("sendingTitle", "");
        if (
            !overHttp(record.origin) ||
            !overHttp(record.uploadUrl) ||
            record.userId == null ||
            record.sessionToken == null ||
            record.refreshToken == null ||
            record.contentType == null ||
            record.byteSize < 0
        ) {
            call.reject("origin, userId, sessionToken, refreshToken, uploadUrl, contentType, and byteSize are required");
            return;
        }
        work(call, "the upload could not be sent", () -> {
            uploads.send(record, System.currentTimeMillis());
            Context context = getContext();
            UploadNotices.clear(context, id);
            UploadService.start(context);
            call.resolve();
        });
    }

    @PluginMethod
    public void list(PluginCall call) {
        String origin = call.getString("origin");
        String userId = call.getString("userId");
        if (origin == null || userId == null) {
            call.reject("origin and userId are required");
            return;
        }
        work(call, "the uploads could not be listed", () -> {
            JSArray held = new JSArray();
            for (JSONObject upload : uploads.list(origin, userId)) {
                held.put(upload);
            }
            JSObject answer = new JSObject();
            answer.put("uploads", held);
            call.resolve(answer);
        });
    }

    @PluginMethod
    public void discard(PluginCall call) {
        String id = idOf(call);
        if (id == null) {
            return;
        }
        work(call, "the upload could not be discarded", () -> {
            uploads.discard(id);
            UploadNotices.clear(getContext(), id);
            call.resolve();
        });
    }

    @Override
    public void progress(String id, long sent, long total) {
        JSObject progress = new JSObject();
        progress.put("id", id);
        progress.put("sent", sent);
        progress.put("total", total);
        tell("progress", progress, false);
    }

    /** Kept for a page that is not listening yet, as one still loading is not. */
    @Override
    public void settled(UploadRecord upload) {
        try {
            tell("settled", JSObject.fromJSONObject(upload.toNative()), true);
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }

    @Override
    public void idle() {}

    @Override
    protected void handleOnResume() {
        UploadNotices.inFront = true;
        UploadNotices.clearAll(getContext());
        calls.execute(this::carryOn);
    }

    @Override
    protected void handleOnPause() {
        UploadNotices.inFront = false;
    }

    /** The uploads go on; only this page's hearing of them ends. */
    @Override
    protected void handleOnDestroy() {
        UploadNotices.inFront = false;
        calls.execute(() -> uploads.unlisten(this));
        calls.shutdown();
    }

    /**
     * Sends what a process that has gone left unfinished, and starts the service for whatever
     * is under way. Run as the app comes to the front, its launch included: the one place the
     * system lets the service start, which the app may not have been when an upload was last
     * sent.
     */
    private void carryOn() {
        uploads.resume();
        if (uploads.busy()) {
            UploadService.start(getContext());
        }
    }

    /** Runs `work` on `calls`, rejecting `call` with `failure` and why should it fail. */
    private void work(PluginCall call, String failure, Work work) {
        calls.execute(() -> {
            try {
                work.run();
            } catch (IOException | RuntimeException e) {
                call.reject(failure + ": " + e.getMessage());
            }
        });
    }

    /**
     * Notifies the page on the bridge's own thread, where its listeners are added and removed:
     * the lists Capacitor keeps them in are not safe to read from another.
     */
    private void tell(String event, JSObject data, boolean retain) {
        execute(() -> notifyListeners(event, data, retain));
    }

    /** The call's `id` when it is an attachment's, and so safe to name a file by; else rejects. */
    private static String idOf(PluginCall call) {
        String id = call.getString("id");
        if (!Uploads.isId(id)) {
            call.reject("id must be an attachment's id");
            return null;
        }
        return id;
    }

    private static boolean overHttp(String url) {
        if (url == null) {
            return false;
        }
        String lower = url.toLowerCase(Locale.ROOT);
        return lower.startsWith("https://") || lower.startsWith("http://");
    }
}
