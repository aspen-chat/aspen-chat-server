package com.aspen_chat.client.uploads;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.net.URLConnection;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ThreadLocalRandom;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * Sends one upload's bytes to storage and confirms it with its deployment, as its user: the
 * requests of `send` in the app's `src/api/uploadsBridge.ts`. It runs on the thread that calls
 * {@link #run}, and any other may {@link #cancel} it.
 */
final class UploadSender {

    /** How many times a request that gets no answer, or one that says to try later, is made. */
    static final int TRIES = 5;
    /** How many other URLs one run asks for when storage refuses the one it has. */
    static final int RENEWALS = 2;

    private static final int CONNECT_MILLISECONDS = 15_000;
    private static final int ANSWER_MILLISECONDS = 15_000;
    /** Storage answers a `PUT` only once it has stored the whole file. */
    private static final int STORED_MILLISECONDS = 60_000;
    private static final int BUFFER_BYTES = 64 * 1024;
    /** The most of an answer that is read: every answer wanted here is a small record. */
    private static final int ANSWER_MAX_BYTES = 1 << 20;
    private static final long PROGRESS_EVERY_NANOSECONDS = 150_000_000L;
    /** How long one buffer may take to be written before the connection counts as stalled. */
    static final long STALLED_MILLISECONDS = 60_000;
    /** The longest a deployment's `Retry-After` holds the next try back. */
    private static final long RETRY_AFTER_MAX_MILLISECONDS = 60_000;

    /** What a run changes of its upload on the way, for whoever keeps the record. */
    interface Events {
        /** Called at most every 150 ms while the file is written, and at its start and end. */
        void progress(long sent);

        /** The deployment gave another URL, which the bytes go to from here on. */
        void renewed(String uploadUrl, String contentType);

        void refreshed(String sessionToken);
    }

    /** The wait before a request is tried again. */
    interface Pacing {
        /**
         * Waits before the try after `failures` failed ones, at least `atLeastMilliseconds`.
         */
        void pause(int failures, long atLeastMilliseconds) throws InterruptedException;
    }

    /**
     * Two, five, fifteen, and forty seconds, each up to a quarter longer so that phones coming
     * back from one outage do not return together: {@link #TRIES} tries span about a minute.
     */
    static final Pacing BACKOFF = new Pacing() {
        private final long[] waits = { 2_000, 5_000, 15_000, 40_000 };

        @Override
        public void pause(int failures, long atLeastMilliseconds) throws InterruptedException {
            long wait = waits[Math.min(failures, waits.length) - 1];
            wait += ThreadLocalRandom.current().nextLong(wait / 4 + 1);
            Thread.sleep(Math.max(wait, atLeastMilliseconds));
        }
    };

    /** How a run ended: with the confirmed record, or with why there is none. */
    static final class Outcome {

        /** The confirmed attachment record, JSON text; {@code null} for a failure. */
        final String attachment;
        final String cause;
        final String reason;

        private Outcome(String attachment, String cause, String reason) {
            this.attachment = attachment;
            this.cause = cause;
            this.reason = reason;
        }

        static Outcome done(String attachment) {
            return new Outcome(attachment, null, null);
        }

        static Outcome unreachable() {
            return new Outcome(null, UploadRecord.UNREACHABLE, null);
        }

        static Outcome refused(String reason) {
            return new Outcome(null, UploadRecord.REFUSED, reason);
        }
    }

    private static final class Cancelled extends Exception {}

    private static final class Answer {

        final int status;
        final String body;
        final long retryAfterMilliseconds;

        Answer(int status, String body, long retryAfterMilliseconds) {
            this.status = status;
            this.body = body;
            this.retryAfterMilliseconds = retryAfterMilliseconds;
        }

        /** The answer as a JSON object; {@code null} when it is not one. */
        JSONObject json() {
            try {
                return new JSONObject(body);
            } catch (JSONException e) {
                return null;
            }
        }

        /**
         * Whether asking again later may be answered otherwise: the server failed, or is
         * limiting requests. A 429 with another code is a quota, which waiting a minute does
         * not lift.
         */
        boolean passing() {
            if (status >= 500 || status == 408) {
                return true;
            }
            if (status != 429) {
                return false;
            }
            JSONObject problem = json();
            String code = problem == null ? null : UploadRecord.text(problem, "code");
            return code == null || code.equals("rateLimited");
        }
    }

    private final String id;
    private final String origin;
    private final String refreshToken;
    private final String mimeType;
    /** The languages the deployment is asked to answer in; empty to leave it unsaid. */
    private final String acceptLanguage;
    private final File file;
    private final Events events;
    private final Pacing pacing;
    private String sessionToken;
    private String uploadUrl;
    private String contentType;

    private volatile boolean cancelled;
    private volatile HttpURLConnection current;
    /** When the write to storage under way began, by `System.nanoTime`; 0 while none is. */
    private volatile long writingSince;
    /** The thread inside {@link #run}, which {@link #cancel} wakes from a wait between tries. */
    private Thread runner;

    UploadSender(UploadRecord record, File file, Events events, Pacing pacing) {
        this.id = record.id;
        this.origin = record.origin;
        this.refreshToken = record.refreshToken;
        this.mimeType = record.mimeType;
        this.acceptLanguage = record.acceptLanguage == null ? "" : record.acceptLanguage;
        this.sessionToken = record.sessionToken;
        this.uploadUrl = record.uploadUrl;
        this.contentType = record.contentType;
        this.file = file;
        this.events = events;
        this.pacing = pacing;
    }

    /** Sends and confirms the upload; {@code null} when it was cancelled first. */
    Outcome run() {
        synchronized (this) {
            runner = Thread.currentThread();
        }
        try {
            Outcome failure = store();
            return failure != null ? failure : confirm();
        } catch (Cancelled | InterruptedException e) {
            return null;
        } finally {
            synchronized (this) {
                runner = null;
                // An interrupt that arrived after the last wait is not the thread's next task's.
                Thread.interrupted();
            }
        }
    }

    /** Stops the run: the request under way is dropped, and no other is made. */
    void cancel() {
        cancelled = true;
        drop();
        synchronized (this) {
            if (runner != null) {
                runner.interrupt();
            }
        }
    }

    /**
     * Drops the connection to storage when a write to it has made no way for
     * `stalledMilliseconds`, which fails that try. A socket has no timeout for writes, so a
     * connection that carries nothing would otherwise hold the upload until TCP gave it up,
     * many minutes later.
     */
    void unstick(long stalledMilliseconds) {
        long since = writingSince;
        if (since != 0 && System.nanoTime() - since > stalledMilliseconds * 1_000_000) {
            drop();
        }
    }

    /** Closes the connection under way, from a thread other than its own, failing its request. */
    private void drop() {
        HttpURLConnection connection = current;
        if (connection != null) {
            try {
                connection.disconnect();
            } catch (RuntimeException e) {
                // Android refuses network work, a socket's closing included, on its main
                // thread; the request then ends by its own timeouts.
            }
        }
    }

    /** Gets the bytes to storage; {@code null} once they are there, else why they are not. */
    private Outcome store() throws Cancelled, InterruptedException {
        int renewals = 0;
        int failures = 0;
        while (true) {
            int status;
            try {
                status = put();
            } catch (IOException | RuntimeException e) {
                // The platform's HTTP client throws unchecked for an address or header it
                // will not send, as well.
                status = -1;
            }
            check();
            if (status / 100 == 2) {
                return null;
            }
            // Storage refuses a URL whose signature has expired with 403, and a URL it was
            // given again for the same reservation is good.
            if (status == 403 && renewals < RENEWALS) {
                renewals++;
                Outcome failure = renew();
                if (failure != null) {
                    return failure;
                }
                continue;
            }
            if (status != -1 && status < 500) {
                return Outcome.unreachable();
            }
            if (++failures == TRIES) {
                return Outcome.unreachable();
            }
            pacing.pause(failures, 0);
        }
    }

    /**
     * Writes the file to `uploadUrl` and answers storage's status. The length is fixed
     * beforehand: the URL is signed for exactly that many bytes, and a streamed body of known
     * length is not held in memory.
     */
    private int put() throws IOException, Cancelled {
        long total = file.length();
        HttpURLConnection connection = open(uploadUrl, "PUT");
        try {
            connection.setReadTimeout(STORED_MILLISECONDS);
            connection.setInstanceFollowRedirects(false);
            connection.setDoOutput(true);
            connection.setFixedLengthStreamingMode(total);
            connection.setRequestProperty("Content-Type", contentType);
            events.progress(0);
            OutputStream body = connection.getOutputStream();
            try (InputStream in = new FileInputStream(file); OutputStream out = body) {
                byte[] buffer = new byte[BUFFER_BYTES];
                long sent = 0;
                long told = System.nanoTime();
                int read;
                while ((read = in.read(buffer)) != -1) {
                    check();
                    writingSince = System.nanoTime();
                    out.write(buffer, 0, read);
                    writingSince = 0;
                    sent += read;
                    long now = System.nanoTime();
                    if (sent == total || now - told >= PROGRESS_EVERY_NANOSECONDS) {
                        told = now;
                        events.progress(sent);
                    }
                }
            } catch (IOException e) {
                // Storage that refuses a URL may answer and close before it has taken the
                // bytes; its answer, when it can still be read, says more than the broken write.
                try {
                    return connection.getResponseCode();
                } catch (IOException | RuntimeException unanswered) {
                    throw e;
                }
            }
            return connection.getResponseCode();
        } finally {
            writingSince = 0;
            current = null;
            connection.disconnect();
        }
    }

    /** Asks the deployment for another URL; {@code null} once it has one, else why not. */
    private Outcome renew() throws Cancelled, InterruptedException {
        String body;
        try {
            body = new JSONObject().put("byteSize", file.length()).toString();
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
        Answer answer = ask("POST", "/api/v1/attachments/" + id + "/renew", body);
        if (answer == null) {
            return Outcome.unreachable();
        }
        if (answer.status != 200) {
            return unconfirmed(answer);
        }
        JSONObject handle = answer.json();
        String url = handle == null ? null : UploadRecord.text(handle, "uploadUrl");
        if (url == null) {
            return Outcome.unreachable();
        }
        // A deployment whose handles name no type signs its URLs for the declared one.
        String type = UploadRecord.text(handle, "contentType");
        uploadUrl = url;
        contentType = type == null ? mimeType : type;
        events.renewed(uploadUrl, contentType);
        return null;
    }

    private Outcome confirm() throws Cancelled, InterruptedException {
        Answer answer = ask("POST", "/api/v1/attachments/" + id + "/confirm", "");
        if (answer == null) {
            return Outcome.unreachable();
        }
        if (answer.status == 200) {
            return answer.json() == null ? Outcome.unreachable() : Outcome.done(answer.body);
        }
        return unconfirmed(answer);
    }

    /**
     * What a refusal to confirm or renew comes to. The deployment answers 404 to both for an
     * upload already confirmed, as one is whose confirmation was answered but never heard (the
     * connection dropped, the process died): only a confirmed attachment can be read, so
     * reading it tells that from an upload the deployment does not know.
     */
    private Outcome unconfirmed(Answer refusal) throws Cancelled, InterruptedException {
        if (refusal.status == 404) {
            Answer held = ask("GET", "/api/v1/attachments/" + id, null);
            if (held != null && held.status == 200 && held.json() != null) {
                return Outcome.done(held.body);
            }
        }
        return Outcome.refused(reasonOf(refusal));
    }

    /** A Problem's words for its reader: its `detail`, or else its `title`. */
    private static String reasonOf(Answer answer) {
        JSONObject problem = answer.json();
        if (problem == null) {
            return null;
        }
        String detail = UploadRecord.text(problem, "detail");
        if (detail != null && !detail.isEmpty()) {
            return detail;
        }
        String title = UploadRecord.text(problem, "title");
        return title == null || title.isEmpty() ? null : title;
    }

    /**
     * The deployment's answer to a request made as the user, with a new session token got once
     * if the one held has expired, and tried again while it does not answer or says to wait;
     * {@code null} when {@link #TRIES} tries got no answer to act on. An answer that is the
     * token refresh's own is a refusal of the refresh token.
     */
    private Answer ask(String method, String path, String body) throws Cancelled, InterruptedException {
        boolean refreshed = false;
        int failures = 0;
        while (true) {
            Answer answer = call(method, origin + path, sessionToken, body);
            check();
            if (answer != null && answer.status == 401 && !refreshed) {
                String refresh;
                try {
                    refresh = new JSONObject().put("refreshToken", refreshToken).toString();
                } catch (JSONException e) {
                    throw new IllegalStateException(e);
                }
                answer = call("POST", origin + "/api/v1/auth/token-refresh", null, refresh);
                check();
                JSONObject session = answer == null || answer.status != 200 ? null : answer.json();
                String token = session == null ? null : UploadRecord.text(session, "sessionToken");
                if (token != null) {
                    refreshed = true;
                    sessionToken = token;
                    events.refreshed(token);
                    continue;
                }
                if (answer != null && answer.status == 200) {
                    answer = null;
                }
            }
            if (answer != null && !answer.passing()) {
                return answer;
            }
            if (++failures == TRIES) {
                return null;
            }
            long asked = answer == null ? 0 : answer.retryAfterMilliseconds;
            pacing.pause(failures, Math.min(asked, RETRY_AFTER_MAX_MILLISECONDS));
        }
    }

    /**
     * One request to the deployment, with `body` as JSON when it has one; {@code null} when it
     * went unanswered.
     */
    private Answer call(String method, String url, String token, String body) throws Cancelled {
        HttpURLConnection connection = null;
        try {
            connection = open(url, method);
            connection.setReadTimeout(ANSWER_MILLISECONDS);
            connection.setRequestProperty("Accept", "application/json");
            if (!acceptLanguage.isEmpty()) {
                // So a refusal reads in the app's language rather than the phone's.
                connection.setRequestProperty("Accept-Language", acceptLanguage);
            }
            if (token != null) {
                connection.setRequestProperty("Authorization", "Bearer " + token);
            }
            if (body != null) {
                byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
                connection.setDoOutput(true);
                if (bytes.length > 0) {
                    connection.setRequestProperty("Content-Type", "application/json");
                }
                try (OutputStream out = connection.getOutputStream()) {
                    out.write(bytes);
                }
            }
            int status = connection.getResponseCode();
            InputStream in = status < 400 ? connection.getInputStream() : connection.getErrorStream();
            return new Answer(status, in == null ? "" : text(in), retryAfter(connection));
        } catch (IOException | RuntimeException e) {
            return null;
        } finally {
            current = null;
            if (connection != null) {
                connection.disconnect();
            }
        }
    }

    /** The wait a `Retry-After` in seconds asks for; 0 for none, or one given as a date. */
    private static long retryAfter(HttpURLConnection connection) {
        String header = connection.getHeaderField("Retry-After");
        if (header == null) {
            return 0;
        }
        try {
            return Math.max(Long.parseLong(header.trim()), 0) * 1000;
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    private HttpURLConnection open(String url, String method) throws IOException, Cancelled {
        URLConnection opened = new URL(url).openConnection();
        if (!(opened instanceof HttpURLConnection)) {
            throw new IOException("not an HTTP address");
        }
        HttpURLConnection connection = (HttpURLConnection) opened;
        connection.setConnectTimeout(CONNECT_MILLISECONDS);
        connection.setRequestMethod(method);
        current = connection;
        // A cancel that came before the connection was its to drop.
        check();
        return connection;
    }

    private void check() throws Cancelled {
        if (cancelled) {
            throw new Cancelled();
        }
    }

    private static String text(InputStream in) throws IOException {
        try (InputStream stream = in) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int read;
            while (out.size() < ANSWER_MAX_BYTES && (read = stream.read(buffer)) != -1) {
                out.write(buffer, 0, read);
            }
            return out.toString("UTF-8");
        }
    }
}
