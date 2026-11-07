package com.desklink.android.shell

import android.content.Context

/** Where the most recently announced input helper listens. Written only by [ShellBridgeProvider]. */
object ShellHelperChannel {
    data class Endpoint(val port: Int, val token: String)

    private const val PREFS = "shell_helper"
    private val listeners = mutableListOf<() -> Unit>()

    @Synchronized
    fun announce(context: Context, port: Int, token: String) {
        if (endpoint(context) == Endpoint(port, token)) return
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putInt("port", port).putString("token", token).commit()
        listeners.toList().forEach { it() }
    }

    @Synchronized
    fun endpoint(context: Context): Endpoint? {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val port = prefs.getInt("port", 0)
        val token = prefs.getString("token", null)
        return if (port > 0 && token != null) Endpoint(port, token) else null
    }

    @Synchronized
    fun addListener(listener: () -> Unit) {
        listeners += listener
    }

    @Synchronized
    fun removeListener(listener: () -> Unit) {
        listeners -= listener
    }
}
