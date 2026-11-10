package com.aspen_chat.client.uploads;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.IOException;
import java.io.InputStream;
import java.io.RandomAccessFile;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;

/**
 * {@link Uploads} against a stand-in for storage and a deployment on this machine, as the page
 * drives it: bytes begun and written, then sent.
 */
public class UploadsTest {

    private static final String ID = "0199f3a0-7c1e-7b54-9d2a-3f6b8c0d1e2f";
    private static final String CONFIRM = "/api/v1/attachments/" + ID + "/confirm";
    private static final String RENEW = "/api/v1/attachments/" + ID + "/renew";
    private static final String READ = "/api/v1/attachments/" + ID;
    private static final String REFRESH = "/api/v1/auth/token-refresh";
    private static final String RECORD = "{\"id\":\"" + ID + "\",\"fileName\":\"notes.txt\"}";
    private static final String PROBLEM = "application/problem+json";
    private static final long NOW = 1_800_000_000_000L;
    private static final byte[] BYTES = "what the file holds".getBytes(StandardCharsets.UTF_8);

    /** One request the stand-in was sent. */
    private static final class Request {

        final String method;
        final String path;
        final String authorization;
        final String contentType;
        final String contentLength;
        final byte[] body;

        Request(StandIn.Exchange exchange, byte[] body) {
            method = exchange.method;
            path = exchange.path;
            authorization = exchange.headers.get("authorization");
            contentType = exchange.headers.get("content-type");
            contentLength = exchange.headers.get("content-length");
            this.body = body;
        }

        String text() {
            return new String(body, StandardCharsets.UTF_8);
        }
    }

    /** What the stand-in answers one request with. */
    private interface Answerer {
        void answer(StandIn.Exchange exchange, Request request) throws Exception;
    }

    private static final class Memory implements Uploads.Persistence {

        volatile String state;

        @Override
        public String load() {
            return state;
        }

        @Override
        public void save(String state) {
            this.state = state;
        }
    }

    private static final class Heard implements Uploads.Listener {

        final BlockingQueue<UploadRecord> settled = new LinkedBlockingQueue<>();
        final List<Long> progress = new CopyOnWriteArrayList<>();
        final CountDownLatch idle = new CountDownLatch(1);

        @Override
        public void progress(String id, long sent, long total) {
            progress.add(sent);
        }

        @Override
        public void settled(UploadRecord upload) {
            settled.add(upload);
        }

        @Override
        public void idle() {
            idle.countDown();
        }
    }

    private StandIn server;
    private String origin;
    private File directory;
    private final Memory memory = new Memory();
    private final Heard heard = new Heard();
    private final List<Request> requests = new CopyOnWriteArrayList<>();
    private final AtomicInteger pauses = new AtomicInteger();
    /** Held by an answerer that must not answer until the test is over. */
    private final CountDownLatch over = new CountDownLatch(1);
    private Uploads uploads;

    @Before
    public void start() throws IOException {
        server = new StandIn();
        origin = server.origin();
        directory = Files.createTempDirectory("aspen-uploads").toFile();
        uploads = open(UploadSender.STALLED_MILLISECONDS);
    }

    @After
    public void stop() {
        over.countDown();
        server.close();
        File[] files = directory.listFiles();
        if (files != null) {
            for (File file : files) {
                file.delete();
            }
        }
        directory.delete();
    }

    /** The uploads as a process starting at {@link #NOW} finds them, waiting no time between tries. */
    private Uploads open(long stalledMilliseconds) {
        Uploads opened = new Uploads(
            directory,
            memory,
            NOW,
            (failures, atLeast) -> pauses.incrementAndGet(),
            stalledMilliseconds
        );
        opened.listen(heard);
        return opened;
    }

    /**
     * Has the stand-in answer requests to `path` with `answerer`, once it has read the body or,
     * for an answerer that reads it itself or not at all, before.
     */
    private void route(String path, boolean readBody, Answerer answerer) {
        server.route(path, exchange -> {
            Request request = new Request(exchange, readBody ? bytes(exchange.body()) : new byte[0]);
            requests.add(request);
            answerer.answer(exchange, request);
        });
    }

    private void route(String path, Answerer answerer) {
        route(path, true, answerer);
    }

    private static void answer(StandIn.Exchange exchange, int status, String contentType, String body)
        throws IOException {
        exchange.answer(status, contentType, body);
    }

    private static byte[] bytes(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buffer = new byte[65536];
        int read;
        while ((read = in.read(buffer)) != -1) {
            out.write(buffer, 0, read);
        }
        return out.toByteArray();
    }

    private void storageTakes(String path) {
        route(path, (exchange, request) -> answer(exchange, 200, null, ""));
    }

    private void deploymentConfirms() {
        route(CONFIRM, (exchange, request) -> answer(exchange, 200, "application/json", RECORD));
    }

    private UploadRecord details(String uploadPath) {
        UploadRecord record = new UploadRecord();
        record.id = ID;
        record.origin = origin;
        record.userId = "0199f3a0-0000-7000-8000-00000000000a";
        record.sessionToken = "session";
        record.refreshToken = "refresh";
        record.uploadUrl = origin + uploadPath;
        record.contentType = "text/plain";
        record.name = "notes.txt";
        record.mimeType = "text/plain; charset=utf-8";
        record.byteSize = BYTES.length;
        record.failedTitle = "Could not send";
        record.failedBody = "notes.txt";
        record.sendingTitle = "Sending files";
        return record;
    }

    /** Hands over the file as the page does, in two pieces, and sends it to `uploadPath`. */
    private void send(String uploadPath) throws IOException {
        uploads.begin(ID);
        uploads.write(ID, Arrays.copyOfRange(BYTES, 0, 7));
        uploads.write(ID, Arrays.copyOfRange(BYTES, 7, BYTES.length));
        uploads.send(details(uploadPath), NOW);
    }

    private UploadRecord settled() throws InterruptedException {
        UploadRecord upload = heard.settled.poll(20, TimeUnit.SECONDS);
        assertNotNull("the upload never settled", upload);
        return upload;
    }

    private List<Request> requestsTo(String path) {
        List<Request> found = new ArrayList<>();
        for (Request request : requests) {
            if (request.path.equals(path)) {
                found.add(request);
            }
        }
        return found;
    }

    private JSONObject listed() {
        List<JSONObject> held = uploads.list(origin, "0199f3a0-0000-7000-8000-00000000000a");
        assertEquals(1, held.size());
        return held.get(0);
    }

    private JSONObject kept() throws Exception {
        JSONArray kept = new JSONObject(memory.state).getJSONArray("uploads");
        assertEquals(1, kept.length());
        return kept.getJSONObject(0);
    }

    @Test
    public void aFileIsSentToStorageAndConfirmedAsItsUser() throws Exception {
        storageTakes("/storage");
        deploymentConfirms();
        send("/storage");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.DONE, upload.state);
        assertEquals(RECORD, upload.attachment);

        Request put = requestsTo("/storage").get(0);
        assertEquals("PUT", put.method);
        assertEquals("text/plain", put.contentType);
        assertEquals(String.valueOf(BYTES.length), put.contentLength);
        assertNull("storage is sent no session", put.authorization);
        assertEquals(new String(BYTES, StandardCharsets.UTF_8), put.text());

        Request confirm = requestsTo(CONFIRM).get(0);
        assertEquals("POST", confirm.method);
        assertEquals("Bearer session", confirm.authorization);
        assertEquals(0, confirm.body.length);

        JSONObject listed = listed();
        assertEquals("done", listed.getString("state"));
        assertEquals(BYTES.length, listed.getLong("sent"));
        assertEquals(RECORD, listed.getString("attachment"));
        assertEquals("notes.txt", listed.getString("name"));
        assertEquals(Long.valueOf(BYTES.length), heard.progress.get(heard.progress.size() - 1));
        assertFalse("the bytes are let go once confirmed", new File(directory, ID).exists());
        assertEquals("done", kept().getString("state"));
        assertTrue(heard.idle.await(5, TimeUnit.SECONDS));
        assertFalse(uploads.busy());
        assertEquals(0, pauses.get());
    }

    @Test
    public void storageRefusingAnExpiredAddressHasAnotherAskedFor() throws Exception {
        route("/expired", (exchange, request) -> answer(exchange, 403, "application/xml", "<Error/>"));
        storageTakes("/renewed");
        route(RENEW, (exchange, request) ->
            answer(
                exchange,
                200,
                "application/json",
                new JSONObject()
                    .put("id", ID)
                    .put("uploadUrl", origin + "/renewed")
                    .put("contentType", "application/octet-stream")
                    .toString()
            )
        );
        deploymentConfirms();
        send("/expired");

        assertEquals(UploadRecord.DONE, settled().state);
        Request renew = requestsTo(RENEW).get(0);
        assertEquals("POST", renew.method);
        assertEquals("Bearer session", renew.authorization);
        assertEquals(BYTES.length, new JSONObject(renew.text()).getLong("byteSize"));
        Request put = requestsTo("/renewed").get(0);
        assertEquals("application/octet-stream", put.contentType);
        assertEquals(new String(BYTES, StandardCharsets.UTF_8), put.text());
        assertEquals(origin + "/renewed", kept().getString("uploadUrl"));
    }

    @Test
    public void storageThatKeepsRefusingIsGivenUpAfterTwoMoreAddresses() throws Exception {
        route("/expired", (exchange, request) -> answer(exchange, 403, null, ""));
        route(RENEW, (exchange, request) ->
            answer(exchange, 200, "application/json", new JSONObject().put("uploadUrl", origin + "/expired").toString())
        );
        deploymentConfirms();
        send("/expired");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.UNREACHABLE, upload.cause);
        assertEquals(UploadSender.RENEWALS, requestsTo(RENEW).size());
        assertEquals(UploadSender.RENEWALS + 1, requestsTo("/expired").size());
        assertTrue(requestsTo(CONFIRM).isEmpty());
    }

    @Test
    public void anExpiredSessionIsRefreshedAndTheConfirmationMadeAgain() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) -> {
            if ("Bearer fresh".equals(request.authorization)) {
                answer(exchange, 200, "application/json", RECORD);
            } else {
                answer(exchange, 401, PROBLEM, "{\"code\":\"invalidToken\",\"title\":\"Sign in again\"}");
            }
        });
        route(REFRESH, (exchange, request) ->
            answer(exchange, 200, "application/json", "{\"sessionToken\":\"fresh\",\"sessionTokenExpires\":\"2027-01-01T00:00:00Z\"}")
        );
        send("/storage");

        assertEquals(UploadRecord.DONE, settled().state);
        Request refresh = requestsTo(REFRESH).get(0);
        assertNull(refresh.authorization);
        assertEquals("refresh", new JSONObject(refresh.text()).getString("refreshToken"));
        List<Request> confirms = requestsTo(CONFIRM);
        assertEquals(2, confirms.size());
        assertEquals("Bearer session", confirms.get(0).authorization);
        assertEquals("Bearer fresh", confirms.get(1).authorization);
        assertEquals("fresh", kept().getString("sessionToken"));
    }

    @Test
    public void aSessionThatCannotBeRefreshedIsARefusal() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) -> answer(exchange, 401, PROBLEM, "{\"code\":\"invalidToken\"}"));
        route(REFRESH, (exchange, request) ->
            answer(exchange, 401, PROBLEM, "{\"code\":\"invalidToken\",\"title\":\"Invalid token\",\"detail\":\"Sign in again.\"}")
        );
        send("/storage");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.REFUSED, upload.cause);
        assertEquals("Sign in again.", upload.reason);
        assertEquals(1, requestsTo(CONFIRM).size());
    }

    @Test
    public void aConfirmationTheDeploymentRefusesFailsInItsWords() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) ->
            answer(
                exchange,
                400,
                PROBLEM,
                "{\"code\":\"validation\",\"title\":\"Invalid request\",\"detail\":\"The file is larger than this server allows.\"}"
            )
        );
        send("/storage");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.REFUSED, upload.cause);
        assertEquals("The file is larger than this server allows.", upload.reason);
        JSONObject listed = listed();
        assertEquals("failed", listed.getString("state"));
        assertEquals("refused", listed.getString("cause"));
        assertEquals("The file is larger than this server allows.", listed.getString("reason"));
        assertFalse(listed.has("attachment"));
        assertTrue("the bytes are kept to send again", new File(directory, ID).exists());
        assertEquals(0, pauses.get());
    }

    @Test
    public void aRefusalWithoutDetailGivesItsTitle() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) -> answer(exchange, 403, PROBLEM, "{\"code\":\"forbidden\",\"title\":\"Not allowed\"}"));
        send("/storage");

        assertEquals("Not allowed", settled().reason);
    }

    @Test
    public void aConfirmationAnsweredButNeverHeardIsFoundByReadingTheAttachment() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) -> answer(exchange, 404, PROBLEM, "{\"code\":\"notFound\",\"title\":\"Not found\"}"));
        route(READ, (exchange, request) -> answer(exchange, "GET".equals(request.method) ? 200 : 405, "application/json", RECORD));
        send("/storage");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.DONE, upload.state);
        assertEquals(RECORD, upload.attachment);
    }

    @Test
    public void aDeploymentThatFailsIsAskedAgain() throws Exception {
        storageTakes("/storage");
        AtomicInteger asked = new AtomicInteger();
        route(CONFIRM, (exchange, request) -> {
            if (asked.incrementAndGet() < 3) {
                answer(exchange, 503, PROBLEM, "{\"code\":\"serverBusy\",\"title\":\"Busy\"}");
            } else {
                answer(exchange, 200, "application/json", RECORD);
            }
        });
        send("/storage");

        assertEquals(UploadRecord.DONE, settled().state);
        assertEquals(2, pauses.get());
    }

    @Test
    public void aDeploymentThatNeverAnswersWellIsGivenUpAsUnreachable() throws Exception {
        storageTakes("/storage");
        route(CONFIRM, (exchange, request) -> answer(exchange, 500, PROBLEM, "{\"code\":\"internal\",\"title\":\"Broken\"}"));
        send("/storage");

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.UNREACHABLE, upload.cause);
        assertNull(upload.reason);
        assertTrue(listed().isNull("reason"));
        assertEquals(UploadSender.TRIES, requestsTo(CONFIRM).size());
    }

    @Test
    public void storageThatCannotBeReachedFailsAndTheSameBytesAreSentAgain() throws Exception {
        int closed;
        try (ServerSocket socket = new ServerSocket(0, 1, InetAddress.getLoopbackAddress())) {
            closed = socket.getLocalPort();
        }
        deploymentConfirms();
        uploads.begin(ID);
        uploads.write(ID, BYTES);
        UploadRecord unreachable = details("/storage");
        unreachable.uploadUrl = "http://127.0.0.1:" + closed + "/storage";
        uploads.send(unreachable, NOW);

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.UNREACHABLE, upload.cause);
        assertEquals(UploadSender.TRIES - 1, pauses.get());
        assertTrue(requestsTo(CONFIRM).isEmpty());

        storageTakes("/storage");
        uploads.send(details("/storage"), NOW);
        assertEquals(UploadRecord.DONE, settled().state);
        assertEquals(new String(BYTES, StandardCharsets.UTF_8), requestsTo("/storage").get(0).text());
    }

    @Test
    public void anUploadDiscardedOnItsWayIsStoppedAndForgotten() throws Exception {
        CountDownLatch arrived = new CountDownLatch(1);
        route("/storage", (exchange, request) -> {
            arrived.countDown();
            over.await();
        });
        deploymentConfirms();
        send("/storage");
        assertTrue(arrived.await(20, TimeUnit.SECONDS));
        assertTrue(uploads.busy());

        uploads.discard(ID);

        assertTrue(heard.idle.await(5, TimeUnit.SECONDS));
        assertFalse(uploads.busy());
        assertTrue(uploads.list(origin, "0199f3a0-0000-7000-8000-00000000000a").isEmpty());
        assertFalse(new File(directory, ID).exists());
        assertEquals(0, new JSONObject(memory.state).getJSONArray("uploads").length());
        assertNull("a discarded upload settles nothing", heard.settled.poll(500, TimeUnit.MILLISECONDS));
        assertTrue(requestsTo(CONFIRM).isEmpty());
        uploads.discard(ID);
    }

    @Test
    public void aConnectionThatCarriesNothingIsDroppedAndTheUploadTriedAgain() throws Exception {
        uploads = open(500);
        AtomicInteger puts = new AtomicInteger();
        // The first request's body is never read, so the sender's writes stop once the
        // sockets' buffers are full.
        route("/storage", false, (exchange, request) -> {
            if (puts.incrementAndGet() == 1) {
                over.await();
            } else {
                byte[] buffer = new byte[65536];
                InputStream body = exchange.body();
                while (body.read(buffer) != -1) {
                    // More than the test should hold in memory.
                }
                answer(exchange, 200, null, "");
            }
        });
        deploymentConfirms();
        // More than the sockets between two ends of this machine buffer, and a file with no
        // bytes written takes no room.
        long size = 256L * 1024 * 1024;
        uploads.begin(ID);
        try (RandomAccessFile file = new RandomAccessFile(new File(directory, ID), "rw")) {
            file.setLength(size);
        }
        UploadRecord large = details("/storage");
        large.byteSize = size;
        uploads.send(large, NOW);

        assertEquals(UploadRecord.DONE, settled().state);
        assertTrue(puts.get() >= 2);
        assertTrue(pauses.get() >= 1);
    }

    @Test
    public void whatAProcessLeftUnfinishedIsSentAgainByTheNext() throws Exception {
        CountDownLatch arrived = new CountDownLatch(1);
        AtomicInteger puts = new AtomicInteger();
        route("/storage", (exchange, request) -> {
            if (puts.incrementAndGet() == 1) {
                arrived.countDown();
                over.await();
            } else {
                answer(exchange, 200, null, "");
            }
        });
        deploymentConfirms();
        send("/storage");
        assertTrue(arrived.await(20, TimeUnit.SECONDS));

        // Another process's uploads, read from what this one kept while the upload was under way.
        Uploads next = open(UploadSender.STALLED_MILLISECONDS);
        assertFalse(next.busy());
        next.resume();

        UploadRecord upload = settled();
        assertEquals(UploadRecord.DONE, upload.state);
        assertEquals(new String(BYTES, StandardCharsets.UTF_8), requestsTo("/storage").get(1).text());
        uploads.discard(ID);
    }

    @Test
    public void givingUpFailsWhatIsUnderWaySoItCanBeSentAgain() throws Exception {
        CountDownLatch arrived = new CountDownLatch(1);
        route("/storage", (exchange, request) -> {
            arrived.countDown();
            over.await();
        });
        send("/storage");
        assertTrue(arrived.await(20, TimeUnit.SECONDS));

        uploads.giveUp();

        UploadRecord upload = settled();
        assertEquals(UploadRecord.FAILED, upload.state);
        assertEquals(UploadRecord.UNREACHABLE, upload.cause);
        assertFalse(uploads.busy());
        assertTrue(new File(directory, ID).exists());
        assertNull(heard.settled.poll(500, TimeUnit.MILLISECONDS));
    }

    @Test
    public void recordsOlderThanTwoDaysAndBytesNoRecordIsForAreDroppedOnLoading() throws Exception {
        route(CONFIRM, (exchange, request) -> answer(exchange, 400, PROBLEM, "{\"title\":\"No\"}"));
        storageTakes("/storage");
        send("/storage");
        assertEquals(UploadRecord.FAILED, settled().state);
        String orphan = "0199f3a0-7c1e-7b54-9d2a-000000000000";
        uploads.begin(orphan);

        Uploads soon = new Uploads(directory, memory, NOW + Uploads.KEPT_MILLISECONDS);
        assertEquals(1, soon.list(origin, "0199f3a0-0000-7000-8000-00000000000a").size());
        assertTrue(new File(directory, ID).exists());
        assertFalse("bytes begun and never sent", new File(directory, orphan).exists());

        Uploads later = new Uploads(directory, memory, NOW + Uploads.KEPT_MILLISECONDS + 1);
        assertTrue(later.list(origin, "0199f3a0-0000-7000-8000-00000000000a").isEmpty());
        assertFalse(new File(directory, ID).exists());
        assertEquals(0, new JSONObject(memory.state).getJSONArray("uploads").length());
    }

    @Test
    public void onlyTheAccountsOwnUploadsAreListed() throws Exception {
        storageTakes("/storage");
        deploymentConfirms();
        send("/storage");
        settled();

        assertTrue(uploads.list(origin, "0199f3a0-0000-7000-8000-00000000000b").isEmpty());
        assertTrue(uploads.list("https://elsewhere.example", "0199f3a0-0000-7000-8000-00000000000a").isEmpty());
    }

    @Test
    public void bytesAreHeldOnlyUnderAnAttachmentsId() throws Exception {
        for (String id : new String[] { "../escape", "", "0199f3a0-7c1e-7b54-9d2a-3f6b8c0d1e2f/..", "0199f3a07c1e7b549d2a3f6b8c0d1e2f" }) {
            assertFalse(id, Uploads.isId(id));
            try {
                uploads.begin(id);
                fail(id);
            } catch (IllegalArgumentException expected) {
                // Refused before any file is named by it.
            }
        }
        assertFalse(Uploads.isId(null));
        assertTrue(Uploads.isId(ID));
    }

    @Test
    public void sendingNeedsTheBytesItWasToldOf() throws Exception {
        try {
            uploads.send(details("/storage"), NOW);
            fail("nothing was begun");
        } catch (IOException expected) {
            // No bytes are held.
        }
        uploads.begin(ID);
        uploads.write(ID, new byte[] { 1, 2, 3 });
        try {
            uploads.send(details("/storage"), NOW);
            fail("the size differs");
        } catch (IOException expected) {
            // The address is signed for another length.
        }
        assertTrue(uploads.list(origin, "0199f3a0-0000-7000-8000-00000000000a").isEmpty());
    }
}
