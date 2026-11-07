package com.desklink.android.shell

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.Bundle
import android.os.Process
import com.desklink.android.NativeBridge
import com.desklink.android.Network
import com.desklink.android.Settings

/**
 * The adb-shell side door. Only the shell user (uid 2000, i.e. a computer the user authorized for
 * USB/wireless debugging) or root may call it:
 *
 * - `register`: the input helper announces its port and token.
 * - `pairing-code`: prints this phone's public pairing code (used by scripts/android-pair.sh).
 * - `pair`: trusts a computer's pairing code. adb access already implies full device control.
 */
class ShellBridgeProvider : ContentProvider() {
    override fun onCreate() = true

    override fun call(method: String, arg: String?, extras: Bundle?): Bundle {
        val caller = Binder.getCallingUid()
        if (caller != SHELL_UID && caller != Process.ROOT_UID) {
            throw SecurityException("Desklink accepts this call only from adb shell")
        }
        val context = context ?: throw IllegalStateException("no context")
        val settings = Settings(context)
        return when (method) {
            "register" -> {
                val (port, token) = parseAnnouncement(arg)
                ShellHelperChannel.announce(context, port, token)
                Bundle().apply { putString("result", "registered") }
            }
            "pairing-code" -> Bundle().apply {
                NativeBridge.init(settings.profileDir, android.os.Build.MODEL ?: "Android phone")
                val info = org.json.JSONObject(NativeBridge.info(settings.profileDir, Network.wifiAddress(context)))
                putString("result", info.getString("pairing_code"))
            }
            "pair" -> Bundle().apply {
                NativeBridge.init(settings.profileDir, android.os.Build.MODEL ?: "Android phone")
                val name = NativeBridge.pair(settings.profileDir, arg ?: "", true)
                putString("result", "paired $name")
                com.desklink.android.ProfileEvents.changed()
            }
            else -> throw IllegalArgumentException("unknown method $method")
        }
    }

    private fun parseAnnouncement(arg: String?): Pair<Int, String> {
        val parts = arg?.split(':') ?: emptyList()
        val port = parts.getOrNull(0)?.toIntOrNull()
        val token = parts.getOrNull(1)
        require(parts.size == 2 && port != null && port in 1..65535) { "invalid port" }
        require(token != null && token.length == 64 && token.all { it in '0'..'9' || it in 'a'..'f' }) { "invalid token" }
        return port to token
    }

    override fun query(u: Uri, p: Array<String>?, s: String?, a: Array<String>?, o: String?): Cursor? = null
    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<String>?) = 0

    companion object {
        const val SHELL_UID = 2000
    }
}
