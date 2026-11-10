package com.aspen_chat.client.uploads;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CopyOnWriteArraySet;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.regex.Pattern;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The uploads the app's native side holds (`AspenUploads` in the app's
 * `src/api/uploadsBridge.ts`): each one's bytes, spooled to a file of the app's own as the page
 * hands them over, its record, and the run sending it. One instance serves the whole process
 * (`DeviceUploads`), so an upload goes on whether the page, the activity, or the service that
 * keeps the process alive is there or not, and what was still being sent when a process died is
 * sent again by the next ({@link #resume}).
 */
public final class Uploads {

    /** Where the records are kept between processes, as one text. */
    public interface Persistence {
        /** The text last saved; {@code null} before any was. */
        String load();

        /** Keeps `state` so that a process that dies as this returns still finds it. */
        void save(String state);
    }

    /** Called on the thread a change happened on, with none of the uploads' locks held. */
    public interface Listener {
        void progress(String id, long sent, long total);

        /** `upload` became done or failed; it is a copy, the listener's to keep. */
        void settled(UploadRecord upload);

        /** The last upload under way ended, or was discarded. */
        void idle();
    }

    /** How long a record is kept when nothing discards it, from when it was last sent. */
    static final long KEPT_MILLISECONDS = 2L * 24 * 60 * 60 * 1000;
    /** How many uploads are sent at once; the rest wait their turn. */
    private static final int AT_ONCE = 3;
    private static final Pattern ID = Pattern.compile("[0-9a-fA-F]{8}(-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}");

    private final File directory;
    private final Persistence persistence;
    private final UploadSender.Pacing pacing;
    private final long stalledMilliseconds;
    private final ExecutorService workers = Executors.newFixedThreadPool(AT_ONCE, threads("aspen-uploads"));
    private final ScheduledExecutorService watcher = Executors.newSingleThreadScheduledExecutor(
        threads("aspen-uploads-watch")
    );
    private final Set<Listener> listeners = new CopyOnWriteArraySet<>();

    /** Guarded by `this`, as is every record in it. */
    private final Map<String, UploadRecord> records = new LinkedHashMap<>();
    /** The run sending each upload under way or waiting its turn, guarded by `this`. */
    private final Map<String, Run> running = new HashMap<>();
    /** The watch for stalled connections, kept only while something runs; guarded by `this`. */
    private ScheduledFuture<?> watch;

    /**
     * Reads what `persistence` kept, dropping every record last sent more than
     * {@link #KEPT_MILLISECONDS} before `now`, and every file in `directory` no record left is
     * for: bytes begun by a page that went before it sent them.
     */
    public Uploads(File directory, Persistence persistence, long now) {
        this(directory, persistence, now, UploadSender.BACKOFF, UploadSender.STALLED_MILLISECONDS);
    }

    Uploads(
        File directory,
        Persistence persistence,
        long now,
        UploadSender.Pacing pacing,
        long stalledMilliseconds
    ) {
        this.directory = directory;
        this.persistence = persistence;
        this.pacing = pacing;
        this.stalledMilliseconds = stalledMilliseconds;
        synchronized (this) {
            load(now);
        }
    }

    public void listen(Listener listener) {
        listeners.add(listener);
    }

    public void unlisten(Listener listener) {
        listeners.remove(listener);
    }

    /** Whether `id` is shaped as an attachment's id is, and so is safe to name a file by. */
    public static boolean isId(String id) {
        return id != null && ID.matcher(id).matches();
    }

    /** Makes an empty file for upload `id`'s bytes, letting go of whatever was held for it. */
    public void begin(String id) throws IOException {
        discard(id);
        synchronized (this) {
            if (!directory.isDirectory() && !directory.mkdirs()) {
                throw new IOException("the uploads' directory could not be made");
            }
            new FileOutputStream(spool(id)).close();
        }
    }

    /** Adds `data` to the end of upload `id`'s bytes. */
    public synchronized void write(String id, byte[] data) throws IOException {
        File file = spool(id);
        if (!file.isFile()) {
            throw new IOException("no such upload was begun");
        }
        try (OutputStream out = new FileOutputStream(file, true)) {
            out.write(data);
        }
    }

    /**
     * Keeps `details` as upload `details.id`'s record and starts sending the bytes held for it,
     * in place of any run already sending them. An upload already confirmed stays as it is:
     * its bytes are gone, and `list` says how it ended.
     */
    public void send(UploadRecord details, long now) throws IOException {
        Run replaced;
        synchronized (this) {
            UploadRecord held = records.get(details.id);
            if (held != null && UploadRecord.DONE.equals(held.state)) {
                return;
            }
            File file = spool(details.id);
            if (!file.isFile()) {
                throw new IOException("no bytes are held for this upload");
            }
            if (file.length() != details.byteSize) {
                throw new IOException("the bytes held for this upload are not as many as it says");
            }
            UploadRecord record = details.copy();
            record.state = UploadRecord.SENDING;
            record.sent = 0;
            record.attachment = null;
            record.cause = null;
            record.reason = null;
            record.startedAt = now;
            records.put(record.id, record);
            save();
            replaced = running.remove(record.id);
            start(record);
        }
        if (replaced != null) {
            replaced.sender.cancel();
        }
    }

    /** Sends every upload a process that has gone left unfinished. */
    public synchronized void resume() {
        for (UploadRecord record : records.values()) {
            if (UploadRecord.SENDING.equals(record.state) && !running.containsKey(record.id)) {
                start(record);
            }
        }
    }

    /** Every upload held for `userId` at `origin`, as the page reads them. */
    public synchronized List<JSONObject> list(String origin, String userId) {
        List<JSONObject> uploads = new ArrayList<>();
        for (UploadRecord record : records.values()) {
            if (record.origin.equals(origin) && record.userId.equals(userId)) {
                uploads.add(record.toNative());
            }
        }
        return uploads;
    }

    /** Stops upload `id` if it is under way, and forgets its record and its bytes. */
    public void discard(String id) {
        File file = spool(id);
        Run run;
        boolean idle;
        synchronized (this) {
            run = running.remove(id);
            if (records.remove(id) != null) {
                save();
            }
            file.delete();
            idle = run != null && rested();
        }
        if (run != null) {
            run.sender.cancel();
        }
        if (idle) {
            for (Listener listener : listeners) {
                listener.idle();
            }
        }
    }

    /** Whether any upload is under way or waiting its turn. */
    public synchronized boolean busy() {
        return !running.isEmpty();
    }

    /**
     * The `sendingTitle` of the upload under way that was sent last; {@code null} when none is.
     */
    public synchronized String sendingTitle() {
        UploadRecord latest = null;
        for (String id : running.keySet()) {
            UploadRecord record = records.get(id);
            if (record != null && (latest == null || record.startedAt >= latest.startedAt)) {
                latest = record;
            }
        }
        return latest == null ? null : latest.sendingTitle;
    }

    /**
     * Ends every upload under way as failed for want of reaching storage, which leaves each
     * one's bytes to be sent again: for when the process may no longer work in the background.
     */
    public void giveUp() {
        List<Run> runs;
        synchronized (this) {
            runs = new ArrayList<>(running.values());
        }
        for (Run run : runs) {
            // Settled first: a run that ends by being cancelled records nothing of its own.
            settle(run, UploadSender.Outcome.unreachable());
            // Not on this thread, which may be the main one, where no connection may be closed.
            watcher.execute(run.sender::cancel);
        }
    }

    private File spool(String id) {
        if (!isId(id)) {
            throw new IllegalArgumentException("not an attachment's id");
        }
        return new File(directory, id);
    }

    private void load(long now) {
        String state = persistence.load();
        boolean dropped = false;
        if (state != null) {
            try {
                JSONArray kept = new JSONObject(state).getJSONArray("uploads");
                for (int i = 0; i < kept.length(); i++) {
                    UploadRecord record = UploadRecord.fromJson(kept.getJSONObject(i));
                    if (!isId(record.id) || now - record.startedAt > KEPT_MILLISECONDS) {
                        dropped = true;
                        continue;
                    }
                    if (UploadRecord.SENDING.equals(record.state)) {
                        record.sent = 0;
                    }
                    records.put(record.id, record);
                }
            } catch (JSONException e) {
                // A state that cannot be read names nothing to send; the files it was for go
                // below, and the next save replaces it.
                records.clear();
                dropped = true;
            }
        }
        File[] files = directory.listFiles();
        if (files != null) {
            for (File file : files) {
                if (!records.containsKey(file.getName())) {
                    file.delete();
                }
            }
        }
        if (dropped) {
            save();
        }
    }

    private void save() {
        try {
            JSONArray kept = new JSONArray();
            for (UploadRecord record : records.values()) {
                kept.put(record.toJson());
            }
            persistence.save(new JSONObject().put("uploads", kept).toString());
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }

    private void start(UploadRecord record) {
        Run run = new Run(record);
        running.put(record.id, run);
        if (watch == null) {
            long every = Math.max(stalledMilliseconds / 4, 1);
            watch = watcher.scheduleWithFixedDelay(this::unstick, every, every, TimeUnit.MILLISECONDS);
        }
        workers.execute(run);
    }

    /** Whether nothing runs any longer, stopping the watch if so. */
    private boolean rested() {
        if (!running.isEmpty()) {
            return false;
        }
        if (watch != null) {
            watch.cancel(false);
            watch = null;
        }
        return true;
    }

    private void unstick() {
        List<Run> runs;
        synchronized (this) {
            runs = new ArrayList<>(running.values());
        }
        for (Run run : runs) {
            run.sender.unstick(stalledMilliseconds);
        }
    }

    /** Records how `run` ended, unless its upload has since been discarded or sent again. */
    private void settle(Run run, UploadSender.Outcome outcome) {
        UploadRecord settled;
        boolean idle;
        synchronized (this) {
            UploadRecord record = records.get(run.id);
            if (running.get(run.id) != run || record == null) {
                return;
            }
            running.remove(run.id);
            idle = rested();
            if (outcome == null) {
                settled = null;
            } else {
                if (outcome.attachment != null) {
                    record.state = UploadRecord.DONE;
                    record.attachment = outcome.attachment;
                    record.sent = record.byteSize;
                    spool(run.id).delete();
                } else {
                    record.state = UploadRecord.FAILED;
                    record.cause = outcome.cause;
                    record.reason = outcome.reason;
                }
                save();
                settled = record.copy();
            }
        }
        for (Listener listener : listeners) {
            if (settled != null) {
                listener.settled(settled);
            }
            if (idle) {
                listener.idle();
            }
        }
    }

    /** One upload's sending, and what it changes of the record on the way. */
    private final class Run implements Runnable, UploadSender.Events {

        final String id;
        final long total;
        final UploadSender sender;

        Run(UploadRecord record) {
            id = record.id;
            total = record.byteSize;
            sender = new UploadSender(record, spool(record.id), this, pacing);
        }

        @Override
        public void run() {
            UploadSender.Outcome outcome;
            try {
                // Storage takes exactly as many bytes as the record says, so a file a process
                // that has gone left otherwise is not worth a try.
                outcome = spool(id).length() == total ? sender.run() : UploadSender.Outcome.unreachable();
            } catch (RuntimeException e) {
                outcome = UploadSender.Outcome.unreachable();
            }
            settle(this, outcome);
        }

        @Override
        public void progress(long sent) {
            synchronized (Uploads.this) {
                UploadRecord record = records.get(id);
                if (running.get(id) != this || record == null) {
                    return;
                }
                record.sent = sent;
            }
            for (Listener listener : listeners) {
                listener.progress(id, sent, total);
            }
        }

        @Override
        public void renewed(String uploadUrl, String contentType) {
            synchronized (Uploads.this) {
                UploadRecord record = records.get(id);
                if (running.get(id) == this && record != null) {
                    record.uploadUrl = uploadUrl;
                    record.contentType = contentType;
                    save();
                }
            }
        }

        @Override
        public void refreshed(String sessionToken) {
            synchronized (Uploads.this) {
                UploadRecord record = records.get(id);
                if (running.get(id) == this && record != null) {
                    record.sessionToken = sessionToken;
                    save();
                }
            }
        }
    }

    /** Daemon threads, so that an idle pool never holds a process open. */
    private static ThreadFactory threads(String name) {
        AtomicInteger count = new AtomicInteger();
        return task -> {
            Thread thread = new Thread(task, name + "-" + count.incrementAndGet());
            thread.setDaemon(true);
            return thread;
        };
    }
}
