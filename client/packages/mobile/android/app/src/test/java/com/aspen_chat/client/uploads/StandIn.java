package com.aspen_chat.client.uploads;

import java.io.BufferedInputStream;
import java.io.ByteArrayOutputStream;
import java.io.Closeable;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * An HTTP server on this machine for {@link UploadsTest} to stand storage and a deployment in
 * with: one request to a connection, answered by whatever the test gave its path. It is written
 * on sockets because JVM tests of the Android app compile against the Android jar, which has
 * no HTTP server (the JDK's `com.sun.net.httpserver` is not in it).
 */
final class StandIn implements Closeable {

    /** Answers one request; one that returns without answering has the connection closed on it. */
    interface Answerer {
        void answer(Exchange exchange) throws Exception;
    }

    /** One request, and the way to answer it. */
    static final class Exchange {

        final String method;
        final String path;
        /** By name in lower case. */
        final Map<String, String> headers;
        private final InputStream in;
        private final OutputStream out;

        Exchange(String method, String path, Map<String, String> headers, InputStream in, OutputStream out) {
            this.method = method;
            this.path = path;
            this.headers = headers;
            this.in = in;
            this.out = out;
        }

        /** The body's bytes, as many as `Content-Length` says, which may be read once. */
        InputStream body() {
            String length = headers.get("content-length");
            long[] left = { length == null ? 0 : Long.parseLong(length) };
            return new InputStream() {
                @Override
                public int read() throws IOException {
                    byte[] one = new byte[1];
                    return read(one, 0, 1) == -1 ? -1 : one[0] & 0xFF;
                }

                @Override
                public int read(byte[] buffer, int offset, int count) throws IOException {
                    if (left[0] == 0) {
                        return -1;
                    }
                    int read = in.read(buffer, offset, (int) Math.min(count, left[0]));
                    if (read == -1) {
                        throw new IOException("the request ended before its body did");
                    }
                    left[0] -= read;
                    return read;
                }
            };
        }

        void answer(int status, String contentType, String body) throws IOException {
            byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
            StringBuilder head = new StringBuilder("HTTP/1.1 " + status + " Answered\r\n");
            if (contentType != null) {
                head.append("Content-Type: ").append(contentType).append("\r\n");
            }
            head.append("Content-Length: ").append(bytes.length).append("\r\nConnection: close\r\n\r\n");
            out.write(head.toString().getBytes(StandardCharsets.ISO_8859_1));
            out.write(bytes);
            out.flush();
        }
    }

    private final ServerSocket listening = new ServerSocket(0, 50, InetAddress.getLoopbackAddress());
    private final Map<String, Answerer> routes = new ConcurrentHashMap<>();
    private final Set<Socket> open = ConcurrentHashMap.newKeySet();

    StandIn() throws IOException {
        Thread accepting = new Thread(this::accept, "stand-in");
        accepting.setDaemon(true);
        accepting.start();
    }

    String origin() {
        return "http://127.0.0.1:" + listening.getLocalPort();
    }

    /** Has requests to exactly `path` answered by `answerer`; any other path is answered 404. */
    void route(String path, Answerer answerer) {
        routes.put(path, answerer);
    }

    /** Stops listening and drops every connection, which frees whoever waits on one. */
    @Override
    public void close() {
        try {
            listening.close();
        } catch (IOException ignored) {
            // Nothing listens either way.
        }
        for (Socket socket : open) {
            try {
                socket.close();
            } catch (IOException ignored) {
                // Already gone.
            }
        }
    }

    private void accept() {
        while (true) {
            Socket socket;
            try {
                socket = listening.accept();
            } catch (IOException closed) {
                return;
            }
            open.add(socket);
            Thread serving = new Thread(() -> serve(socket), "stand-in-connection");
            serving.setDaemon(true);
            serving.start();
        }
    }

    private void serve(Socket socket) {
        try (Socket connection = socket) {
            InputStream in = new BufferedInputStream(connection.getInputStream());
            String[] head = head(in).split("\r\n");
            String[] request = head[0].split(" ");
            Map<String, String> headers = new HashMap<>();
            for (int i = 1; i < head.length; i++) {
                int colon = head[i].indexOf(':');
                headers.put(head[i].substring(0, colon).trim().toLowerCase(Locale.ROOT), head[i].substring(colon + 1).trim());
            }
            int query = request[1].indexOf('?');
            String path = query == -1 ? request[1] : request[1].substring(0, query);
            Exchange exchange = new Exchange(request[0], path, headers, in, connection.getOutputStream());
            Answerer answerer = routes.get(path);
            if (answerer == null) {
                exchange.answer(404, null, "");
            } else {
                answerer.answer(exchange);
            }
        } catch (Exception gone) {
            // A client that went away, as a discarded upload's does, or a test that is over.
        } finally {
            open.remove(socket);
        }
    }

    /** The request line and headers, up to the blank line that ends them. */
    private static String head(InputStream in) throws IOException {
        ByteArrayOutputStream head = new ByteArrayOutputStream();
        int ended = 0;
        while (ended < 4) {
            int next = in.read();
            if (next == -1) {
                throw new IOException("the connection closed before a request arrived");
            }
            head.write(next);
            ended = next == "\r\n\r\n".charAt(ended) ? ended + 1 : next == '\r' ? 1 : 0;
        }
        return head.toString("ISO-8859-1");
    }
}
