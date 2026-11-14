package com.aspen_chat.client.input;

import android.content.Context;
import android.util.AttributeSet;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import androidx.core.view.inputmethod.EditorInfoCompat;
import androidx.core.view.inputmethod.InputConnectionCompat;
import com.getcapacitor.CapacitorWebView;

/**
 * Capacitor's web view, which also takes what a keyboard inserts besides text (a GIF, a
 * sticker, an image) into the field the page says takes it (`AspenKeyboardContentPlugin`). The
 * web view's own input connection offers keyboards text alone in a `<textarea>`, so the
 * keyboard is told of the types here and what it commits goes to the plugin. Put in place of
 * Capacitor's by `res/layout/capacitor_bridge_layout_main.xml`.
 */
public class AspenWebView extends CapacitorWebView {

    public AspenWebView(Context context, AttributeSet attrs) {
        super(context, attrs);
    }

    @Override
    @SuppressWarnings("deprecation")
    public InputConnection onCreateInputConnection(EditorInfo outAttrs) {
        InputConnection connection = super.onCreateInputConnection(outAttrs);
        if (connection == null || !AspenKeyboardContentPlugin.accepting()) {
            return connection;
        }
        EditorInfoCompat.setContentMimeTypes(outAttrs, AspenKeyboardContentPlugin.MIME_TYPES);
        // The listener form rather than the view's own receiver
        // (`ViewCompat.setOnReceiveContentListener`), which would also tell the system the whole
        // web view takes content in every field, and catch its pastes and drops.
        return InputConnectionCompat.createWrapper(
            connection,
            outAttrs,
            (content, flags, options) -> AspenKeyboardContentPlugin.receive(getContext(), content, flags)
        );
    }
}
