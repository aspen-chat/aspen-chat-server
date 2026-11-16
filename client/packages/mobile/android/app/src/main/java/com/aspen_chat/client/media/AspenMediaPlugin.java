package com.aspen_chat.client.media;

import android.Manifest;
import android.app.Activity;
import android.content.ContentResolver;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.ext.SdkExtensions;
import android.provider.MediaStore;
import android.util.Log;
import android.webkit.MimeTypeMap;
import androidx.activity.result.ActivityResult;
import androidx.activity.result.PickVisualMediaRequest;
import androidx.activity.result.contract.ActivityResultContracts.PickMultipleVisualMedia;
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia;
import androidx.core.content.FileProvider;
import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.PermissionState;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.ActivityCallback;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.getcapacitor.annotation.Permission;
import com.getcapacitor.annotation.PermissionCallback;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Photos and videos from the system's own screens for a message's attachments (`AspenMedia` in
 * the app's `src/api/mediaBridge.ts`, which iOS's plugin of the same name answers too): the photo
 * picker, which runs apart from the app and hands it only what was chosen, so the app holds no
 * permission to read the device's photos, and the camera app, through its capture intents. The
 * camera app needs no permission of the app's own, but Android refuses those intents to an app
 * that declares `CAMERA` (as this one does, for calls) without holding it, so it is asked for
 * first.
 *
 * Each thing chosen or taken is copied into a directory of its own under the app's cache and
 * answered as a `file:` URL the page reads it from and then `release`s.
 */
@CapacitorPlugin(
    name = "AspenMedia",
    permissions = { @Permission(strings = { Manifest.permission.CAMERA }, alias = AspenMediaPlugin.CAMERA) }
)
public class AspenMediaPlugin extends Plugin {

    private static final String TAG = "AspenMedia";

    static final String CAMERA = "camera";

    private static final String DIRECTORY = "media";

    private final ExecutorService copier = Executors.newSingleThreadExecutor();

    /**
     * The key in a capture's call of where the camera app is to write what it takes, kept in
     * the call so that it outlasts the activity, which Android may end while the camera is in
     * front.
     */
    private static final String OUTPUT = "output";

    @Override
    public void load() {
        // Copies the page never released when the app last went.
        deleteWithin(directory());
    }

    /** Which of the system's screens this device has: the photo picker, and a camera app for each. */
    @PluginMethod
    public void sources(PluginCall call) {
        PackageManager packages = getContext().getPackageManager();
        boolean camera = packages.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY);
        JSObject answer = new JSObject();
        answer.put("library", PickVisualMedia.isPhotoPickerAvailable(getContext()));
        answer.put("photo", camera && resolves(new Intent(MediaStore.ACTION_IMAGE_CAPTURE)));
        answer.put("video", camera && resolves(new Intent(MediaStore.ACTION_VIDEO_CAPTURE)));
        call.resolve(answer);
    }

    /**
     * Opens `source`'s screen: `library` (photos and videos, at most `limit`), `photo`, or
     * `video`. Answers what was chosen or taken, none when the user backed out.
     */
    @PluginMethod
    public void choose(PluginCall call) {
        String source = call.getString("source", "");
        switch (source) {
            case "library" -> pick(call);
            case "photo", "video" -> {
                if (getPermissionState(CAMERA) == PermissionState.GRANTED) {
                    capture(call);
                } else {
                    requestPermissionForAlias(CAMERA, call, "cameraAnswered");
                }
            }
            default -> call.reject("unknown source: " + source);
        }
    }

    /** Removes `url`, a copy this plugin answered, and the directory it was made in. */
    @PluginMethod
    public void release(PluginCall call) {
        release(call.getString("url", ""));
        call.resolve();
    }

    private void pick(PluginCall call) {
        int limit = Math.max(1, call.getInt("limit", 1));
        PickVisualMediaRequest request = new PickVisualMediaRequest.Builder()
            .setMediaType(PickVisualMedia.ImageAndVideo.INSTANCE)
            .build();
        // The picker refuses a limit past its own, and a limit of one from the contract for
        // several.
        int most = Math.min(limit, pickerLimit());
        Intent intent = most > 1
            ? new PickMultipleVisualMedia(most).createIntent(getContext(), request)
            : new PickVisualMedia().createIntent(getContext(), request);
        startActivityForResult(call, intent, "picked");
    }

    @ActivityCallback
    private void picked(PluginCall call, ActivityResult result) {
        if (result.getResultCode() != Activity.RESULT_OK || result.getData() == null) {
            answer(call, new JSArray());
            return;
        }
        // The contract for several reads one chosen as well as many.
        List<Uri> chosen = new PickMultipleVisualMedia(2).parseResult(result.getResultCode(), result.getData());
        ContentResolver resolver = getContext().getContentResolver();
        copier.execute(() -> {
            JSArray items = new JSArray();
            try {
                for (Uri uri : chosen) {
                    String mimeType = resolver.getType(uri);
                    String extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mimeType);
                    File copied = copy(uri, extension == null ? "picked" : "picked." + extension);
                    JSObject item = item(copied, mimeType == null ? "" : mimeType);
                    Long takenAt = takenAtOf(resolver, uri);
                    if (takenAt != null) {
                        item.put("takenAt", takenAt.longValue());
                    }
                    items.put(item);
                }
                answer(call, items);
            } catch (IOException | SecurityException e) {
                Log.w(TAG, "a chosen item could not be copied", e);
                for (int i = 0; i < items.length(); i++) {
                    release(items.optJSONObject(i).optString("url"));
                }
                call.reject("what was chosen could not be read: " + e.getMessage());
            }
        });
    }

    @PermissionCallback
    private void cameraAnswered(PluginCall call) {
        if (getPermissionState(CAMERA) == PermissionState.GRANTED) {
            capture(call);
        } else {
            call.reject("the camera is not allowed", "cameraDenied");
        }
    }

    private void capture(PluginCall call) {
        boolean video = "video".equals(call.getString("source"));
        File holding = new File(directory(), UUID.randomUUID().toString());
        holding.mkdirs();
        File file = new File(holding, video ? "capture.mp4" : "capture.jpg");
        call.getData().put(OUTPUT, file.getPath());
        Uri output = FileProvider.getUriForFile(getContext(), getContext().getPackageName() + ".fileprovider", file);
        Intent intent = new Intent(video ? MediaStore.ACTION_VIDEO_CAPTURE : MediaStore.ACTION_IMAGE_CAPTURE);
        intent.putExtra(MediaStore.EXTRA_OUTPUT, output);
        intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_READ_URI_PERMISSION);
        startActivityForResult(call, intent, "captured");
    }

    @ActivityCallback
    private void captured(PluginCall call, ActivityResult result) {
        String path = call.getString(OUTPUT);
        File file = path == null ? null : new File(path);
        boolean video = "video".equals(call.getString("source"));
        String mimeType = video ? "video/mp4" : "image/jpeg";
        Intent data = result.getData();
        Uri elsewhere = data == null ? null : data.getData();
        if (file == null || result.getResultCode() != Activity.RESULT_OK) {
            if (file != null) {
                release(Uri.fromFile(file).toString());
            }
            answer(call, new JSArray());
        } else if (file.length() > 0) {
            JSArray items = new JSArray();
            items.put(item(file, mimeType));
            answer(call, items);
        } else if (elsewhere != null) {
            // Some camera apps keep a video where they like and answer where it is.
            File holding = file.getParentFile();
            copier.execute(() -> {
                try {
                    String type = getContext().getContentResolver().getType(elsewhere);
                    JSArray items = new JSArray();
                    items.put(item(copyInto(holding, elsewhere, file.getName()), type == null ? mimeType : type));
                    answer(call, items);
                } catch (IOException | SecurityException e) {
                    release(Uri.fromFile(file).toString());
                    call.reject("what the camera took could not be read: " + e.getMessage());
                }
            });
        } else {
            release(Uri.fromFile(file).toString());
            call.reject("the camera app answered nothing");
        }
    }

    /**
     * One thing answered, with no name: the picker names what it hands over by its id in the
     * device's media, and the camera names nothing, so the page names each for when it was taken.
     */
    private static JSObject item(File file, String mimeType) {
        JSObject item = new JSObject();
        item.put("url", Uri.fromFile(file).toString());
        item.put("name", "");
        item.put("mimeType", mimeType);
        item.put("byteSize", file.length());
        return item;
    }

    private void answer(PluginCall call, JSArray items) {
        JSObject answer = new JSObject();
        answer.put("items", items);
        call.resolve(answer);
    }

    private File copy(Uri uri, String name) throws IOException {
        File holding = new File(directory(), UUID.randomUUID().toString());
        holding.mkdirs();
        return copyInto(holding, uri, name);
    }

    private File copyInto(File holding, Uri uri, String name) throws IOException {
        File file = new File(holding, name);
        try (
            InputStream in = getContext().getContentResolver().openInputStream(uri);
            OutputStream out = new FileOutputStream(file)
        ) {
            if (in == null) {
                throw new IOException("the picker gave nothing to read");
            }
            byte[] buffer = new byte[64 * 1024];
            for (int read; (read = in.read(buffer)) != -1;) {
                out.write(buffer, 0, read);
            }
        }
        return file;
    }

    /** When what `uri` names was taken, in milliseconds since the epoch, where the picker says. */
    private static Long takenAtOf(ContentResolver resolver, Uri uri) {
        String[] columns = { MediaStore.Images.ImageColumns.DATE_TAKEN };
        try (Cursor cursor = resolver.query(uri, columns, null, null, null)) {
            if (cursor != null && cursor.moveToFirst() && !cursor.isNull(0)) {
                return cursor.getLong(0);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "when a chosen item was taken could not be read", e);
        }
        return null;
    }

    /** The most the system's photo picker lets one choose at once, where it says. */
    private static int pickerLimit() {
        boolean says =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU ||
            (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && SdkExtensions.getExtensionVersion(Build.VERSION_CODES.R) >= 2);
        return says ? MediaStore.getPickImagesMaxLimit() : Integer.MAX_VALUE;
    }

    private boolean resolves(Intent intent) {
        return intent.resolveActivity(getContext().getPackageManager()) != null;
    }

    private void release(String url) {
        String path = Uri.parse(url).getPath();
        if (path != null) {
            File holding = new File(path).getParentFile();
            if (holding != null && directory().equals(holding.getParentFile())) {
                deleteWithin(holding);
                holding.delete();
            }
        }
    }

    private static void deleteWithin(File directory) {
        File[] inside = directory.listFiles();
        if (inside != null) {
            for (File file : inside) {
                if (file.isDirectory()) {
                    deleteWithin(file);
                }
                file.delete();
            }
        }
    }

    private File directory() {
        File directory = new File(getContext().getCacheDir(), DIRECTORY);
        directory.mkdirs();
        return directory;
    }

    @Override
    protected void handleOnDestroy() {
        copier.shutdown();
    }
}
