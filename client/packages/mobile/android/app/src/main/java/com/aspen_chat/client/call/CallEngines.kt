package com.aspen_chat.client.call

import android.content.Context
import android.content.pm.ApplicationInfo

/**
 * The call engines of this process, one per deployment: whatever acts as an account's engine looks
 * its deployment up here, so a DM call ringing the phone (`CallRings`, answered through
 * `CallTelecom`) and the plugin the web view commands drive the same engine, and the one call the
 * engine holds is the one Telecom holds. An engine is made the first time its deployment is asked
 * for and kept for the process's life.
 */
object CallEngines {
    /** By the deployment's `public_url`, with no trailing slash. */
    private val engines = HashMap<String, CallEngine>()

    /**
     * The engine of the deployment at `origin` (its `public_url`), made on first use. A debug build
     * lets a development deployment's voice servers be reached over plain HTTP at this device's own
     * addresses; a release build never does.
     */
    @Synchronized
    fun forOrigin(context: Context, origin: String): CallEngine {
        val key = trimmed(origin)
        return engines.getOrPut(key) { CallEngine(context.applicationContext, key, loopbackPlainText(context)) }
    }

    /** Whether this is a debuggable build, which alone may reach a deployment over plain HTTP at this device. */
    fun loopbackPlainText(context: Context): Boolean =
        (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    /** The engine already made for `origin`, if any. */
    @Synchronized
    fun existing(origin: String): CallEngine? = engines[trimmed(origin)]

    /** Every engine made so far, in no order; at most one of them is in a call, the one Telecom holds. */
    @Synchronized
    fun all(): List<CallEngine> = engines.values.toList()

    fun trimmed(origin: String): String = origin.trimEnd('/')
}
