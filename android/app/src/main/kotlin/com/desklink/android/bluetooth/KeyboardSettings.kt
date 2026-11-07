package com.desklink.android.bluetooth

import android.content.Context
import android.content.SharedPreferences

/**
 * The computer this phone controls as a Bluetooth keyboard and mouse. Its Desklink identity,
 * port and trusted controller live in the Rust profile under [profileDir].
 */
class KeyboardSettings(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences("desklink", Context.MODE_PRIVATE)
    val profileDir: String = context.applicationContext.filesDir.resolve("keyboard").absolutePath

    /** A Bluetooth computer has been set up; its agent runs whenever sharing is on. */
    var enabled: Boolean
        get() = prefs.getBoolean(KEY_ENABLED, false)
        set(value) = prefs.edit().putBoolean(KEY_ENABLED, value).apply()

    /** The computer's Bluetooth address; `null` until it first connects to the phone. */
    var deviceAddress: String?
        get() = prefs.getString(KEY_DEVICE, null)
        set(value) = prefs.edit().putString(KEY_DEVICE, value).apply()

    /** Screen size in the computer's pointer units: macOS "looks like" points, or pixels. */
    var width: Int
        get() = prefs.getInt(KEY_WIDTH, DEFAULT_WIDTH)
        set(value) = prefs.edit().putInt(KEY_WIDTH, value).apply()
    var height: Int
        get() = prefs.getInt(KEY_HEIGHT, DEFAULT_HEIGHT)
        set(value) = prefs.edit().putInt(KEY_HEIGHT, value).apply()

    /** Linux/Windows Control acts as ⌘ on a Mac, so copy, paste and other shortcuts keep working. */
    var swapControlAndCommand: Boolean
        get() = prefs.getBoolean(KEY_SWAP, true)
        set(value) = prefs.edit().putBoolean(KEY_SWAP, value).apply()

    /** Movement deltas instead of exact positions, for computers that ignore absolute pointers. */
    var relativePointer: Boolean
        get() = prefs.getBoolean(KEY_RELATIVE, false)
        set(value) = prefs.edit().putBoolean(KEY_RELATIVE, value).apply()

    fun clear() {
        prefs.edit()
            .remove(KEY_ENABLED).remove(KEY_DEVICE).remove(KEY_WIDTH).remove(KEY_HEIGHT)
            .remove(KEY_SWAP).remove(KEY_RELATIVE)
            .apply()
    }

    companion object {
        /** A 14-inch MacBook Pro at its default "looks like" resolution. */
        const val DEFAULT_WIDTH = 1512
        const val DEFAULT_HEIGHT = 982
        const val DEFAULT_NAME = "Computer (Bluetooth)"
        private const val KEY_ENABLED = "keyboard_enabled"
        private const val KEY_DEVICE = "keyboard_device"
        private const val KEY_WIDTH = "keyboard_width"
        private const val KEY_HEIGHT = "keyboard_height"
        private const val KEY_SWAP = "keyboard_swap_control_command"
        private const val KEY_RELATIVE = "keyboard_relative_pointer"
    }
}
