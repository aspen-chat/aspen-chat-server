package org.aspenchat.client.push;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertNull;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationManager;
import android.content.Context;
import android.os.Build;
import android.service.notification.StatusBarNotification;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.math.BigInteger;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.SecureRandom;
import java.security.interfaces.ECPrivateKey;
import java.security.interfaces.ECPublicKey;
import java.security.spec.ECGenParameterSpec;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Base64;
import java.util.List;
import javax.crypto.Cipher;
import javax.crypto.KeyAgreement;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * The notification code end to end, on a device: a pointer encrypted as a deployment encrypts
 * it, the message fetched from a stand-in deployment (whose first answer says the session has
 * expired, so the code must refresh it), and the notification that results, then taken down.
 */
@RunWith(AndroidJUnit4.class)
public class PushHandlerTest {

    private static final String SUBSCRIPTION = "0190f0a0-0000-7000-8000-00000000aaaa";
    private static final String CHANNEL = "0190f0a0-0000-7000-8000-000000000011";
    private static final String COMMUNITY = "0190f0a0-0000-7000-8000-000000000010";
    private static final String AUTHOR = "0190f0a0-0000-7000-8000-000000000002";
    private static final String READER = "0190f0a0-0000-7000-8000-000000000001";
    private static final String FIRST = "0190f0a0-0000-7000-8001-000000000201";
    private static final String SECOND = "0190f0a0-0000-7000-8001-000000000202";

    private Context context;
    private NotificationManager notifications;
    private ServerSocket server;
    private final List<String> requests = new ArrayList<>();
    private byte[] phonePublic;
    private byte[] auth;

    @Before
    public void setUp() throws Exception {
        context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        notifications = context.getSystemService(NotificationManager.class);
        notifications.cancelAll();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            InstrumentationRegistry.getInstrumentation()
                .getUiAutomation()
                .grantRuntimePermission(context.getPackageName(), Manifest.permission.POST_NOTIFICATIONS);
        }
        server = new ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"));
        Thread serving = new Thread(this::serve);
        serving.setDaemon(true);
        serving.start();

        KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
        generator.initialize(new ECGenParameterSpec("secp256r1"));
        KeyPair phone = generator.generateKeyPair();
        phonePublic = point((ECPublicKey) phone.getPublic());
        auth = new byte[16];
        new SecureRandom().nextBytes(auth);
        JSONObject privateKey = new JSONObject()
            .put("kty", "EC")
            .put("crv", "P-256")
            .put("d", b64(unsigned(((ECPrivateKey) phone.getPrivate()).getS())));
        JSONObject account = new JSONObject()
            .put("subscription", SUBSCRIPTION)
            .put("origin", "http://127.0.0.1:" + server.getLocalPort())
            .put("userId", READER)
            .put("refreshToken", "refresh")
            .put("sessionToken", "stale")
            .put("keys", new JSONObject().put("privateKey", privateKey).put("publicKey", b64(phonePublic)).put("auth", b64(auth)))
            .put("applicationServerKey", "key")
            .put("deploymentSubscription", "dep");
        JSONObject state = new JSONObject()
            .put("version", 1)
            .put("device", JSONObject.NULL)
            .put("accounts", new JSONArray().put(account));
        new PushStore(context).save(state.toString());
    }

    @After
    public void tearDown() throws Exception {
        server.close();
        notifications.cancelAll();
    }

    @Test
    public void showsAMessageAndTakesItDownWhenReadElsewhere() throws Exception {
        PushHandler handler = new PushHandler(context);
        handler.handle(SUBSCRIPTION, pointer("message", FIRST, 2));
        StatusBarNotification shown = shown(FIRST);
        assertNotNull("the message is shown", shown);
        Notification notification = shown.getNotification();
        assertEquals("Bob · #general", notification.extras.getCharSequence(Notification.EXTRA_TITLE).toString());
        assertEquals("@Kate lunch?", notification.extras.getCharSequence(Notification.EXTRA_TEXT).toString());
        assertEquals(
            "the expired session was replaced, and the new one kept",
            Arrays.asList(
                "GET /api/v1/messages/" + FIRST + " stale",
                "POST /api/v1/auth/token-refresh",
                "GET /api/v1/messages/" + FIRST + " fresh"
            ),
            requests
        );
        assertEquals("fresh", new PushStore(context).account(SUBSCRIPTION).getString("sessionToken"));

        handler.handle(SUBSCRIPTION, pointer("message", SECOND, 3));
        assertNotNull(shown(SECOND));
        handler.handle(SUBSCRIPTION, pointer("read", FIRST, 1));
        assertNull("what was read elsewhere is taken down", gone(FIRST));
        assertNotNull("and what came after it stays", shown(SECOND));
        handler.handle(SUBSCRIPTION, pointer("deleted", SECOND, 0));
        assertNull("a deleted message is taken down", gone(SECOND));
    }

    @Test
    public void ignoresAPushForNoAccountOrThatDoesNotDecrypt() throws Exception {
        PushHandler handler = new PushHandler(context);
        handler.handle("someone-else", pointer("message", FIRST, 1));
        byte[] tampered = Base64.getUrlDecoder().decode(pointer("message", FIRST, 1));
        tampered[tampered.length - 1] ^= 1;
        handler.handle(SUBSCRIPTION, b64(tampered));
        assertEquals(0, notifications.getActiveNotifications().length);
        assertEquals(0, requests.size());
    }

    /**
     * The notification shown for {@code message}, waiting a moment for it to come or go: the
     * system posts and cancels notifications after the call that asks.
     */
    private StatusBarNotification shown(String message) throws InterruptedException {
        return shown(message, true);
    }

    private StatusBarNotification gone(String message) throws InterruptedException {
        return shown(message, false);
    }

    private StatusBarNotification shown(String message, boolean wanted) throws InterruptedException {
        StatusBarNotification found = null;
        for (int i = 0; i < 40; i++) {
            found = find(message);
            if ((found != null) == wanted) {
                return found;
            }
            Thread.sleep(50);
        }
        return found;
    }

    private StatusBarNotification find(String message) {
        for (StatusBarNotification notification : notifications.getActiveNotifications()) {
            if ((CHANNEL + "/" + message).equals(notification.getTag())) {
                return notification;
            }
        }
        return null;
    }

    /** A pointer, encrypted to the phone as a deployment does (RFC 8291). */
    private String pointer(String kind, String message, int badge) throws Exception {
        JSONObject pointer = new JSONObject().put("v", 1).put("kind", kind).put("channel", CHANNEL).put("message", message);
        if (!kind.equals("deleted")) {
            pointer.put("badge", badge);
        }
        KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
        generator.initialize(new ECGenParameterSpec("secp256r1"));
        KeyPair sender = generator.generateKeyPair();
        byte[] senderPublic = point((ECPublicKey) sender.getPublic());
        KeyAgreement agreement = KeyAgreement.getInstance("ECDH");
        agreement.init(sender.getPrivate());
        java.security.KeyFactory factory = java.security.KeyFactory.getInstance("EC");
        agreement.doPhase(
            factory.generatePublic(new java.security.spec.ECPublicKeySpec(WebPush.point(phonePublic), WebPush.p256())),
            true
        );
        byte[] secret = agreement.generateSecret();
        byte[] salt = new byte[16];
        new SecureRandom().nextBytes(salt);
        byte[] ikm = WebPush.hkdf(auth, secret, concat("WebPush: info\0".getBytes(StandardCharsets.US_ASCII), phonePublic, senderPublic), 32);
        byte[] cek = WebPush.hkdf(salt, ikm, "Content-Encoding: aes128gcm\0".getBytes(StandardCharsets.US_ASCII), 16);
        byte[] nonce = WebPush.hkdf(salt, ikm, "Content-Encoding: nonce\0".getBytes(StandardCharsets.US_ASCII), 12);
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(cek, "AES"), new GCMParameterSpec(128, nonce));
        byte[] record = cipher.doFinal(concat(pointer.toString().getBytes(StandardCharsets.UTF_8), new byte[] { 2 }));
        return b64(concat(salt, new byte[] { 0, 0, 16, 0, 65 }, senderPublic, record));
    }

    /** The stand-in deployment: a message whose read fails once with an expired session. */
    private void serve() {
        while (!server.isClosed()) {
            try (Socket socket = server.accept()) {
                BufferedReader in = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.UTF_8));
                String[] line = in.readLine().split(" ");
                String authorization = "";
                int length = 0;
                String header;
                while ((header = in.readLine()) != null && !header.isEmpty()) {
                    String lower = header.toLowerCase();
                    if (lower.startsWith("authorization: bearer ")) {
                        authorization = header.substring("authorization: bearer ".length());
                    } else if (lower.startsWith("content-length: ")) {
                        length = Integer.parseInt(header.substring("content-length: ".length()).trim());
                    }
                }
                char[] body = new char[length];
                int read = 0;
                while (read < length) {
                    read += in.read(body, read, length - read);
                }
                String path = line[1].split("\\?")[0];
                String status;
                String reply;
                if (line[0].equals("POST") && path.equals("/api/v1/auth/token-refresh")) {
                    requests.add("POST " + path);
                    status = new JSONObject(new String(body)).getString("refreshToken").equals("refresh") ? "200 OK" : "401 Unauthorized";
                    reply = "{\"sessionToken\":\"fresh\",\"sessionTokenExpires\":\"2030-01-01T00:00:00Z\"}";
                } else {
                    requests.add("GET " + path + " " + authorization);
                    if (!authorization.equals("fresh")) {
                        status = "401 Unauthorized";
                        reply = "{\"code\":\"unauthorized\"}";
                    } else {
                        status = "200 OK";
                        reply = messageRead(path.substring(path.lastIndexOf('/') + 1));
                    }
                }
                byte[] bytes = reply.getBytes(StandardCharsets.UTF_8);
                OutputStream out = socket.getOutputStream();
                out.write(("HTTP/1.1 " + status + "\r\nContent-Type: application/json\r\nContent-Length: " + bytes.length + "\r\nConnection: close\r\n\r\n").getBytes(StandardCharsets.US_ASCII));
                out.write(bytes);
                out.flush();
            } catch (Exception e) {
                if (server.isClosed()) {
                    return;
                }
            }
        }
    }

    private static String messageRead(String id) throws Exception {
        JSONObject message = new JSONObject()
            .put("id", id)
            .put("channelId", CHANNEL)
            .put("author", AUTHOR)
            .put("content", "<@" + READER + "> lunch?")
            .put("kind", "standard");
        JSONObject channel = new JSONObject()
            .put("id", CHANNEL)
            .put("name", "general")
            .put("ty", "text")
            .put("community", COMMUNITY)
            .put("parentChannel", JSONObject.NULL);
        JSONArray users = new JSONArray()
            .put(new JSONObject().put("id", AUTHOR).put("name", "bob").put("displayName", "Bob"))
            .put(new JSONObject().put("id", READER).put("name", "kate").put("displayName", "Kate"));
        return new JSONObject()
            .put("data", message)
            .put("included", new JSONObject().put("users", users).put("channels", new JSONArray().put(channel)))
            .toString();
    }

    private static byte[] point(ECPublicKey key) {
        return concat(new byte[] { 4 }, unsigned(key.getW().getAffineX()), unsigned(key.getW().getAffineY()));
    }

    /** A coordinate or scalar as exactly 32 bytes. */
    private static byte[] unsigned(BigInteger value) {
        byte[] raw = value.toByteArray();
        byte[] out = new byte[32];
        int copy = Math.min(raw.length, 32);
        System.arraycopy(raw, raw.length - copy, out, 32 - copy, copy);
        return out;
    }

    private static String b64(byte[] bytes) {
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
    }

    private static byte[] concat(byte[]... parts) {
        int length = 0;
        for (byte[] part : parts) {
            length += part.length;
        }
        byte[] out = new byte[length];
        int at = 0;
        for (byte[] part : parts) {
            System.arraycopy(part, 0, out, at, part.length);
            at += part.length;
        }
        return out;
    }
}
