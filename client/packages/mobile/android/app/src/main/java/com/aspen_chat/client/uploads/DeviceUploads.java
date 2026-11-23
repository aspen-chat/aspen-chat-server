package com.aspen_chat.client.uploads;

import android.content.Context;
import com.aspen_chat.client.secrets.SecretStore;
import java.io.File;

/**
 * The process's one {@link Uploads}: its files under the app's own files directory and its
 * records, which hold session and refresh tokens, sealed by {@link SecretStore}.
 */
final class DeviceUploads {

    private static final String FILE = "aspen_uploads";
    private static final String KEY = "state";
    private static final String DIRECTORY = "uploads";

    private static Uploads uploads;

    private DeviceUploads() {}

    static synchronized Uploads of(Context context) {
        if (uploads == null) {
            Context app = context.getApplicationContext();
            SecretStore store = new SecretStore(app, FILE);
            uploads = new Uploads(
                new File(app.getFilesDir(), DIRECTORY),
                new Uploads.Persistence() {
                    @Override
                    public String load() {
                        return store.load(KEY);
                    }

                    // Written before returning: an upload whose record is not on disk when the
                    // process dies is never sent again.
                    @Override
                    public void save(String state) {
                        store.write(KEY, state);
                    }
                },
                System.currentTimeMillis()
            );
            uploads.listen(new UploadNotices(app));
        }
        return uploads;
    }
}
