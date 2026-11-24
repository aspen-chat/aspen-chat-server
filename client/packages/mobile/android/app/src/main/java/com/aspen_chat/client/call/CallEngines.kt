package com.aspen_chat.client.call

import android.content.Context
import android.content.pm.ApplicationInfo
import org.json.JSONObject

/**
 * An account the call engines act as: the deployment at `origin` (its `public_url`, with no
 * trailing slash) and the user signed in there, as the push state's accounts name them. The phone
 * may hold several accounts on one deployment, so an engine, its session, and whatever follows its
 * call are known by both, never by the deployment alone. Its origin is kept trimmed, however it
 * is given.
 */
class CallAccount(origin: String, val userId: String) {
    val origin: String = CallEngines.trimmed(origin)

    /** Whether `account`, kept for push (`PushStore`), is this one. */
    fun names(account: JSONObject): Boolean =
        CallEngines.trimmed(account.optString("origin")) == origin && account.optString("userId") == userId

    override fun equals(other: Any?): Boolean =
        other is CallAccount && other.origin == origin && other.userId == userId

    override fun hashCode(): Int = 31 * origin.hashCode() + userId.hashCode()

    override fun toString(): String = "$userId at $origin"

    companion object {
        /** The account `kept`, an account of the push state, is. */
        fun of(kept: JSONObject): CallAccount = CallAccount(kept.getString("origin"), kept.getString("userId"))
    }
}

/**
 * The call engines of this process, one per account: whatever acts as an account's engine looks it
 * up here, so a DM call ringing the phone (`CallRings`, answered through `CallTelecom`) and the
 * plugin the web view commands drive the same engine, and the one call the engine holds is the one
 * Telecom holds. An engine is made the first time its account is asked for and kept for the
 * process's life.
 */
object CallEngines {
    private val engines = HashMap<CallAccount, CallEngine>()

    /**
     * The engine of `account`, made on first use. A debug build lets a development deployment's
     * voice servers be reached over plain HTTP at this device's own addresses; a release build never
     * does.
     */
    @Synchronized
    fun forAccount(context: Context, account: CallAccount): CallEngine =
        engines.getOrPut(account) { CallEngine(context.applicationContext, account, loopbackPlainText(context)) }

    /** Whether this is a debuggable build, which alone may reach a deployment over plain HTTP at this device. */
    fun loopbackPlainText(context: Context): Boolean =
        (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    /** The engine already made for `account`, if any. */
    @Synchronized
    fun existing(account: CallAccount): CallEngine? = engines[account]

    /** Every engine made so far, in no order; at most one of them is in a call, the one Telecom holds. */
    @Synchronized
    fun all(): List<CallEngine> = engines.values.toList()

    fun trimmed(origin: String): String = origin.trimEnd('/')
}
