package com.aspen_chat.client.call

import android.content.Context
import android.util.Log
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.push.SessionRefresh
import com.aspen_chat.client.voice.Credentials
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/**
 * The session the call engine acts as (`Credentials`): the account at one deployment among those
 * the app keeps for push (`PushStore`, the state the web view writes on sign-in and the messaging
 * service reads). The token kept there is sent; when the API server refuses it, a new one is got
 * from the account's refresh token through `SessionRefresh`, as the push handler gets one, and
 * written back, so pushes and later calls use it too. Nothing is cached here: the store is read
 * for each request, so a token the web view renewed meanwhile is the one sent.
 */
class CallCredentials(context: Context, origin: String) : Credentials {
    private val context = context.applicationContext
    private val store = PushStore(context)
    /** The deployment's `public_url`, as the account's `origin` names it. */
    private val origin = origin.trimEnd('/')

    private fun account(): JSONObject? = store.accountAt(origin)

    /** The account's session token, or `null` when it has none. */
    private fun sessionToken(account: JSONObject): String? =
        if (account.isNull("sessionToken")) null else account.getString("sessionToken")

    override suspend fun sessionToken(): String? = account()?.let(::sessionToken)

    override suspend fun renewedSessionToken(rejected: String): String? {
        val account = account() ?: return null
        // Another reader of the state (the push handler) may have renewed the token already; the
        // one kept is used before another is asked for.
        val kept = sessionToken(account)
        if (kept != null && kept != rejected) {
            return kept
        }
        val token = withContext(Dispatchers.IO) {
            try {
                SessionRefresh.refresh(context, account)
            } catch (e: Exception) {
                Log.w(TAG, "the session could not be renewed", e)
                null
            }
        }
        if (token != null) {
            store.setSessionToken(account.optString("subscription"), token)
        }
        return token
    }

    private companion object {
        const val TAG = "AspenCall"
    }
}
