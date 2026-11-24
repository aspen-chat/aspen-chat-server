package com.aspen_chat.client.call

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.push.SessionRefresh
import java.net.HttpURLConnection
import org.json.JSONObject

/**
 * A DM's call ringing this phone, from a `call` push (spec/push.md, Calls and Rings): the push
 * handler, having decrypted a `call` pointer, hands it here (by name, since a build without the
 * call layer has no such class), and the deployment's engine's `CallTelecom` shows the incoming
 * call, within the five seconds a service started by a push has. Declining ends the ring at the
 * deployment, so the person's other devices stop too; nothing is ever pushed to end a ring, which
 * `CallTelecom` learns of through the engine's ring watch.
 */
object CallRings {
    private const val TAG = "AspenCall"

    /** What a `call` pointer says. */
    data class IncomingRing(
        /** The account the push's subscription names, which the ring is for. */
        val account: CallAccount,
        val channel: String,
        val session: String,
        val callerName: String,
        /** RFC 3339, on the deployment's clock. */
        val until: String,
    )

    /**
     * Shows the call `pointer` describes as ringing the phone, for `kept`, the push state's account
     * the push's subscription names; `false` for a pointer that is not a whole `call`. Called from
     * the push handler's thread; the call is made on the main thread.
     */
    @JvmStatic
    fun incoming(context: Context, kept: JSONObject, pointer: JSONObject): Boolean {
        val ring = read(CallAccount.of(kept), pointer) ?: return false
        val application = context.applicationContext
        Handler(Looper.getMainLooper()).post {
            CallEngines.forAccount(application, ring.account).telecom.ring(ring)
        }
        return true
    }

    /** The ring a `call` pointer describes, or `null` when a field it needs is missing. */
    fun read(account: CallAccount, pointer: JSONObject): IncomingRing? {
        if (pointer.optString("kind") != "call") {
            return null
        }
        val channel = pointer.optString("channel")
        val session = pointer.optString("session")
        val callerName = pointer.optString("callerName")
        val until = pointer.optString("until")
        if (channel.isEmpty() || session.isEmpty() || until.isEmpty()) {
            return null
        }
        return IncomingRing(account, channel, session, callerName, until)
    }

    /**
     * Ends the ring at the deployment (`DELETE /channels/{channel}/voice/rings/@me`) as `ringing`,
     * the account it rang, renewing its session once when refused. Runs on a thread of its own;
     * nothing waits for it.
     */
    fun decline(context: Context, ringing: CallAccount, channel: String) {
        val application = context.applicationContext
        Thread {
            try {
                val store = PushStore(application)
                val account = store.accountWhere(ringing::names) ?: return@Thread
                val url = "${ringing.origin}/api/v1/channels/$channel/voice/rings/@me"
                var status = delete(application, url, account.getString("sessionToken"))
                if (status == 401) {
                    val token = SessionRefresh.refresh(application, account) ?: return@Thread
                    store.setSessionToken(account.optString("subscription"), token)
                    status = delete(application, url, token)
                }
                if (status != 204) {
                    Log.w(TAG, "declining a ring answered $status")
                }
            } catch (e: Exception) {
                Log.w(TAG, "the ring could not be declined at the deployment", e)
            }
        }.start()
    }

    private fun delete(context: Context, url: String, token: String): Int {
        val connection: HttpURLConnection = SessionRefresh.open(context, url)
        connection.requestMethod = "DELETE"
        connection.setRequestProperty("Authorization", "Bearer $token")
        try {
            return connection.responseCode
        } finally {
            connection.disconnect()
        }
    }
}
