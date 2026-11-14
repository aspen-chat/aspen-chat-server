package com.aspen_chat.client.input;

import android.content.ContentResolver;
import android.content.Context;
import android.net.Uri;
import android.util.Log;
import android.view.inputmethod.InputMethodManager;
import android.webkit.MimeTypeMap;
import androidx.core.view.inputmethod.InputConnectionCompat;
import androidx.core.view.inputmethod.InputContentInfoCompat;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.UUID;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * What a keyboard inserts besides text into a message box (`AspenKeyboardContent` in the app's
 * `src/api/keyboardContent.ts`). The page says while the focused field takes it
 * (`setAccepting`), and only then does the web view (`AspenWebView`) tell the keyboard so. Each
 * thing inserted is copied into the app's cache, since the keyboard's permission to read it
 * ends with the insertion, and announced as `content` with a `file:` URL the page reads it from
 * and then `release`s.
 */
@CapacitorPlugin(name = "AspenKeyboardContent")
public class AspenKeyboardContentPlugin extends Plugin {

    private static final String TAG = "AspenKeyboardContent";

    /** What keyboards are told a message box takes: whatever a message may attach of theirs. */
    static final String[] MIME_TYPES = { "image/*", "video/*" };

    /**
     * The most of one thing kept from a keyboard. Keyboards insert GIFs and stickers, far
     * smaller; the bound keeps a misbehaving one from filling the cache.
     */
    private static final long MAX_BYTES = 64L * 1024 * 1024;

    private static final String DIRECTORY = "keyboard-content";

    private static volatile AspenKeyboardContentPlugin instance;
    private static volatile boolean accepting;

    private final ExecutorService copier = Executors.newSingleThreadExecutor();

    @Override
    public void load() {
        instance = this;
        // Copies the page never released when the app last went.
        File[] left = directory().listFiles();
        if (left != null) {
            for (File file : left) {
                file.delete();
            }
        }
    }

    /** Whether the field focused in the page takes what keyboards insert. */
    static boolean accepting() {
        return accepting && instance != null;
    }

    @PluginMethod
    public void setAccepting(PluginCall call) {
        boolean now = Boolean.TRUE.equals(call.getBoolean("accepting", false));
        if (now != accepting) {
            accepting = now;
            // The keyboard reads what a field takes when its input starts, which focusing the
            // field has already done by the time the page says so.
            getActivity().runOnUiThread(() -> {
                InputMethodManager keyboards = getContext().getSystemService(InputMethodManager.class);
                if (keyboards != null) {
                    keyboards.restartInput(getBridge().getWebView());
                }
            });
        }
        call.resolve();
    }

    @PluginMethod
    public void release(PluginCall call) {
        String url = call.getString("url", "");
        String path = Uri.parse(url).getPath();
        if (path != null) {
            File file = new File(path);
            File parent = file.getParentFile();
            if (parent != null && parent.equals(directory())) {
                file.delete();
            }
        }
        call.resolve();
    }

    /**
     * Takes `content` the keyboard committed, answering whether it was taken; the copy is made
     * and announced after the keyboard is answered.
     */
    static boolean receive(Context context, InputContentInfoCompat content, int flags) {
        AspenKeyboardContentPlugin plugin = instance;
        if (plugin == null || !accepting) {
            return false;
        }
        if ((flags & InputConnectionCompat.INPUT_CONTENT_GRANT_READ_URI_PERMISSION) != 0) {
            try {
                content.requestPermission();
            } catch (Exception e) {
                Log.w(TAG, "the keyboard's content could not be read", e);
                return false;
            }
        }
        String mimeType = mimeTypeOf(context.getContentResolver(), content);
        plugin.copier.execute(() -> plugin.copy(content, mimeType));
        return true;
    }

    private void copy(InputContentInfoCompat content, String mimeType) {
        String extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(mimeType);
        File file = new File(directory(), UUID.randomUUID() + (extension == null ? "" : "." + extension));
        boolean copied = false;
        try (
            InputStream in = getContext().getContentResolver().openInputStream(content.getContentUri());
            OutputStream out = new FileOutputStream(file)
        ) {
            if (in == null) {
                throw new IOException("the keyboard gave nothing to read");
            }
            byte[] buffer = new byte[64 * 1024];
            long total = 0;
            for (int read; (read = in.read(buffer)) != -1;) {
                total += read;
                if (total > MAX_BYTES) {
                    throw new IOException("the keyboard's content is larger than " + MAX_BYTES + " bytes");
                }
                out.write(buffer, 0, read);
            }
            copied = true;
        } catch (IOException | SecurityException e) {
            Log.w(TAG, "the keyboard's content could not be copied", e);
        } finally {
            content.releasePermission();
        }
        if (!copied) {
            file.delete();
            return;
        }
        JSObject announced = new JSObject();
        announced.put("url", Uri.fromFile(file).toString());
        announced.put("mimeType", mimeType);
        notifyListeners("content", announced);
    }

    /** The type the content provider names, or the first the keyboard described it as. */
    private static String mimeTypeOf(ContentResolver resolver, InputContentInfoCompat content) {
        String named = resolver.getType(content.getContentUri());
        if (named != null) {
            return named;
        }
        return content.getDescription().getMimeTypeCount() > 0
            ? content.getDescription().getMimeType(0)
            : "application/octet-stream";
    }

    private File directory() {
        File directory = new File(getContext().getCacheDir(), DIRECTORY);
        directory.mkdirs();
        return directory;
    }

    @Override
    protected void handleOnDestroy() {
        if (instance == this) {
            instance = null;
            accepting = false;
        }
        copier.shutdown();
    }
}
