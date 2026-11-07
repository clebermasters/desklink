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
import com.desklink.android.SharingService
import com.desklink.android.bluetooth.HidKeyboardMouse
import com.desklink.android.bluetooth.KeyboardSettings

/**
 * The adb-shell side door. Only the shell user (uid 2000, i.e. a computer the user authorized for
 * USB/wireless debugging) or root may call it:
 *
 * - `register`: the input helper announces its port and token.
 * - `pairing-code`: prints this phone's public pairing code (used by scripts/android-pair.sh).
 * - `pair`: trusts a computer's pairing code. adb access already implies full device control.
 * - `keyboard-pair`, `keyboard-pairing-code`: the same for the computer this phone controls as a
 *   Bluetooth keyboard and mouse.
 * - `hid-*`: test hooks that drive the Bluetooth keyboard and mouse directly.
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
            // The computer this phone controls as a Bluetooth keyboard and mouse (scripts/android-pair.sh --keyboard).
            "keyboard-pair" -> Bundle().apply {
                val keyboard = KeyboardSettings(context)
                NativeBridge.keyboardEnsure(keyboard.profileDir, KeyboardSettings.DEFAULT_NAME)
                val name = NativeBridge.keyboardPair(keyboard.profileDir, arg ?: "")
                keyboard.enabled = true
                SharingService.keyboardChanged(context)
                putString("result", "paired $name")
                com.desklink.android.ProfileEvents.changed()
            }
            "keyboard-pairing-code" -> Bundle().apply {
                val keyboard = KeyboardSettings(context)
                NativeBridge.keyboardEnsure(keyboard.profileDir, KeyboardSettings.DEFAULT_NAME)
                val info = org.json.JSONObject(NativeBridge.info(keyboard.profileDir, Network.wifiAddress(context)))
                putString("result", info.getString("pairing_code"))
            }
            // Test hooks for the Bluetooth keyboard and mouse; adb already implies full control.
            "hid-start" -> Bundle().apply {
                val hid = HidKeyboardMouse.get(context)
                hid.preferredAddress = arg?.takeIf { it.isNotBlank() }
                hid.start()
                putString("result", hid.state.name)
            }
            "hid-status" -> Bundle().apply {
                val hid = HidKeyboardMouse.get(context)
                putString("result", "${hid.state.name} host=${hid.host?.address ?: "none"}")
            }
            // Comma-separated "id:hex" reports, sent 8 ms apart like a real keyboard.
            "hid-send" -> Bundle().apply {
                val hid = HidKeyboardMouse.get(context)
                val reports = (arg ?: "").split(',').filter { it.isNotBlank() }.map { item ->
                    val (id, hex) = item.trim().split(':', limit = 2).let { it[0].toInt() to it.getOrElse(1) { "" } }
                    require(hex.length % 2 == 0 && hex.all { it in "0123456789abcdef" }) { "invalid report" }
                    id to hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
                }
                require(reports.size <= 2000) { "too many reports" }
                val sent = reports.count { (id, data) -> hid.send(id, data).also { Thread.sleep(8) } }
                putString("result", "sent $sent of ${reports.size}")
            }
            "hid-stop" -> Bundle().apply {
                HidKeyboardMouse.get(context).stop()
                putString("result", "stopped")
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
