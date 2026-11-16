package com.aspen_chat.client.call

import com.aspen_chat.client.voice.ConnectedSocket
import com.aspen_chat.client.voice.Received
import com.aspen_chat.client.voice.Socket
import com.aspen_chat.client.voice.SocketFactory
import com.aspen_chat.client.voice.TransportException
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.Continuation
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.suspendCoroutine
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.ClosedReceiveChannelException
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString

/**
 * The call engine's WebSockets (`SocketFactory`, `Socket`) over OkHttp's, which answers the
 * server's pings itself. `connect` returns once the upgrade is answered, so a refused or
 * unreachable server is the engine's `TransportException` and never a socket that closes at once.
 */
class CallSockets(client: OkHttpClient = CallHttp.client) : SocketFactory {
    /** No read timeout: a signalling socket may carry nothing for minutes between frames. */
    private val client = client.newBuilder()
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build()

    override suspend fun connect(url: String, bearer: String?): ConnectedSocket {
        val request = Request.Builder().url(url)
        if (bearer != null) {
            request.header("Authorization", "Bearer $bearer")
        }
        val socket = CallSocket()
        val opened = suspendCoroutine { continuation -> socket.open(client, request.build(), continuation) }
        return ConnectedSocket(opened)
    }
}

/**
 * One WebSocket. Frames arrive on OkHttp's reader thread and wait in a queue for `receive`;
 * closing it, from either side, ends the queue, so the next `receive` answers `Closed`.
 */
class CallSocket : Socket {
    private val frames = Channel<Received>(Channel.UNLIMITED)
    private val ended = AtomicBoolean(false)
    private var webSocket: WebSocket? = null

    /**
     * The extensions the upgrade negotiated (`Sec-WebSocket-Extensions` of the server's answer),
     * `null` when none were. OkHttp offers `permessage-deflate` on every WebSocket and takes the
     * server's answer when it is one it reads (no unknown parameter, no `client_max_window_bits`),
     * so against the voice server this names it and the offers arrive compressed.
     */
    @Volatile
    var negotiatedExtensions: String? = null
        private set

    /** Opens the socket, resuming `opening` once the upgrade is answered, with the socket or the refusal. */
    internal fun open(client: OkHttpClient, request: Request, opening: Continuation<Socket>) {
        val waiting = AtomicReference<Continuation<Socket>?>(opening)
        webSocket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                negotiatedExtensions = response.header("Sec-WebSocket-Extensions")
                waiting.getAndSet(null)?.resume(this@CallSocket)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                frames.trySend(Received.Text(text))
            }

            /** Binary frames are skipped: the signalling is text. */
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {}

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                // The server's close is answered, so the socket ends on both sides.
                webSocket.close(code, reason)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                end()
            }

            /** Before the upgrade, a refusal; after it, a drop, which `receive` reads as `Closed`. */
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val detail = if (response != null) "${request.url} answered ${response.code}" else t.toString()
                waiting.getAndSet(null)?.resumeWithException(TransportException.Failed(detail))
                end()
            }
        })
    }

    private fun end() {
        if (ended.compareAndSet(false, true)) {
            frames.close()
        }
    }

    override suspend fun sendText(text: String) {
        val socket = webSocket
        if (ended.get() || socket == null || !socket.send(text)) {
            throw TransportException.Failed("the socket is closed")
        }
    }

    override suspend fun receive(): Received {
        return try {
            frames.receive()
        } catch (closed: ClosedReceiveChannelException) {
            Received.Closed
        }
    }

    override fun closeSocket() {
        // The queue is ended at once, so the engine reads `Closed` without waiting for the
        // server's answer to the close, which a dropped network never gives.
        webSocket?.close(NORMAL_CLOSURE, null)
        end()
    }

    private companion object {
        const val NORMAL_CLOSURE = 1000
    }
}
