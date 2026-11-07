package com.desklink.android

import android.content.Context
import android.content.SharedPreferences

/** App-only preferences. Pairing and identity live in the Rust profile under [profileDir]. */
class Settings(context: Context) {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences("desklink", Context.MODE_PRIVATE)
    val profileDir: String = context.applicationContext.filesDir.resolve("profile").absolutePath

    var inputMode: InputMode
        get() = InputMode.entries.firstOrNull { it.name == prefs.getString(KEY_MODE, null) }
            ?: InputMode.ACCESSIBILITY
        set(value) = prefs.edit().putString(KEY_MODE, value.name).apply()

    /** Pointer speed multiplier; 1.0 makes one computer pixel move one density-independent pixel. */
    var pointerSpeed: Float
        get() = prefs.getFloat(KEY_SPEED, 1.0f)
        set(value) = prefs.edit().putFloat(KEY_SPEED, value).apply()

    var sharingWanted: Boolean
        get() = prefs.getBoolean(KEY_WANTED, false)
        set(value) = prefs.edit().putBoolean(KEY_WANTED, value).apply()

    companion object {
        private const val KEY_MODE = "input_mode"
        private const val KEY_SPEED = "pointer_speed"
        private const val KEY_WANTED = "sharing_wanted"
    }
}

enum class InputMode { ACCESSIBILITY, SHELL }
