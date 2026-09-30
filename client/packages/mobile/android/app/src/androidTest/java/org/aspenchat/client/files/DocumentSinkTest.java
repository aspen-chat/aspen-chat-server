package org.aspenchat.client.files;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.database.Cursor;
import android.net.Uri;
import android.provider.MediaStore;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.UUID;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * A received file written into a real content provider's document: kept whole when closed, gone
 * when aborted. The Downloads collection stands in for the document the picker returns.
 */
@RunWith(AndroidJUnit4.class)
public class DocumentSinkTest {

    private final ContentResolver resolver =
            InstrumentationRegistry.getInstrumentation().getTargetContext().getContentResolver();

    private Uri newDownload() {
        ContentValues values = new ContentValues();
        values.put(MediaStore.Downloads.DISPLAY_NAME, "aspen-sink-" + UUID.randomUUID() + ".bin");
        Uri uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
        assertNotNull(uri);
        return uri;
    }

    private static byte[] pattern(int size) {
        byte[] bytes = new byte[size];
        for (int i = 0; i < size; i++) {
            bytes[i] = (byte) (i * 31);
        }
        return bytes;
    }

    @Test
    public void closingKeepsEveryByteInOrder() throws Exception {
        Uri uri = newDownload();
        try {
            byte[] expected = pattern(2_500_000);
            DocumentSink sink = DocumentSink.open(resolver, uri);
            for (int at = 0; at < expected.length; at += 700_000) {
                int end = Math.min(expected.length, at + 700_000);
                byte[] piece = new byte[end - at];
                System.arraycopy(expected, at, piece, 0, piece.length);
                sink.write(piece);
            }
            sink.close();
            ByteArrayOutputStream read = new ByteArrayOutputStream();
            try (InputStream in = resolver.openInputStream(uri)) {
                assertNotNull(in);
                byte[] buffer = new byte[65536];
                int n;
                while ((n = in.read(buffer)) > 0) {
                    read.write(buffer, 0, n);
                }
            }
            assertArrayEquals(expected, read.toByteArray());
        } finally {
            resolver.delete(uri, null, null);
        }
    }

    @Test
    public void abortingDeletesTheDocument() throws Exception {
        Uri uri = newDownload();
        DocumentSink sink = DocumentSink.open(resolver, uri);
        sink.write(pattern(100_000));
        sink.abort();
        try (Cursor cursor = resolver.query(uri, null, null, null, null)) {
            assertFalse("the document is gone", cursor != null && cursor.moveToFirst());
        }
    }
}
