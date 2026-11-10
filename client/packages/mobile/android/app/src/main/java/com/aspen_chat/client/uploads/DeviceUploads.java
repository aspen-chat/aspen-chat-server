package com.aspen_chat.client.uploads;

import android.content.Context;
import android.content.SharedPreferences;
import java.io.File;

/**
 * The process's one {@link Uploads}: its files under the app's own files directory and its
 * records, which hold session and refresh tokens, in private preferences, where only this app
 * can read either.
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
            SharedPreferences preferences = app.getSharedPreferences(FILE, Context.MODE_PRIVATE);
            uploads = new Uploads(
                new File(app.getFilesDir(), DIRECTORY),
                new Uploads.Persistence() {
                    @Override
                    public String load() {
                        return preferences.getString(KEY, null);
                    }

                    // `commit`, not `apply`: an upload whose record is not on disk when the
                    // process dies is never sent again.
                    @Override
                    public void save(String state) {
                        preferences.edit().putString(KEY, state).commit();
                    }
                },
                System.currentTimeMillis()
            );
            uploads.listen(new UploadNotices(app));
        }
        return uploads;
    }
}
