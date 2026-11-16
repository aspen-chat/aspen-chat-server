package com.aspen_chat.client.call

import com.aspen_chat.client.voice.Header
import com.aspen_chat.client.voice.Http
import com.aspen_chat.client.voice.HttpRequest
import com.aspen_chat.client.voice.HttpResponse
import com.aspen_chat.client.voice.TransportException
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.suspendCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response

/**
 * The call engine's HTTP requests (`Http`) over OkHttp, so TLS trust and proxies are the
 * system's. Each request is sent as the engine wrote it: no cookies, no redirects followed, no
 * header added, and the answer's status, headers, and body given back whatever the status.
 */
class CallHttp(client: OkHttpClient = CallHttp.client) : Http {
    private val client = client.newBuilder()
        .followRedirects(false)
        .followSslRedirects(false)
        .build()

    override suspend fun request(request: HttpRequest): HttpResponse {
        val built = Request.Builder().url(request.url)
        for (header in request.headers) {
            built.header(header.name, header.value)
        }
        // OkHttp refuses a POST with no body, which the engine's are; an empty one is sent.
        val body = request.body?.toRequestBody(JSON)
            ?: if (request.method == "POST") ByteArray(0).toRequestBody(null) else null
        built.method(request.method, body)
        val response = send(built.build())
        return response.use {
            HttpResponse(
                status = it.code.toUShort(),
                headers = it.headers.map { (name, value) -> Header(name, value) },
                body = it.body?.string() ?: "",
            )
        }
    }

    private suspend fun send(request: Request): Response = suspendCoroutine { continuation ->
        client.newCall(request).enqueue(object : Callback {
            override fun onResponse(call: Call, response: Response) {
                continuation.resume(response)
            }

            override fun onFailure(call: Call, e: IOException) {
                continuation.resumeWithException(TransportException.Failed(e.toString()))
            }
        })
    }

    companion object {
        private val JSON = "application/json".toMediaType()

        /**
         * One client for the process, whose connection pool and threads every call's HTTP and
         * WebSockets share; each use derives its own settings from it.
         */
        val client: OkHttpClient by lazy {
            OkHttpClient.Builder()
                .connectTimeout(30, TimeUnit.SECONDS)
                .readTimeout(30, TimeUnit.SECONDS)
                .writeTimeout(30, TimeUnit.SECONDS)
                .build()
        }
    }
}
