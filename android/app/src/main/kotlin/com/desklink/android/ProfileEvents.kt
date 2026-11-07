package com.desklink.android

/** In-process notice that pairing changed outside the UI (e.g. through the adb shell provider). */
object ProfileEvents {
    private val listeners = mutableListOf<() -> Unit>()

    @Synchronized fun add(listener: () -> Unit) { listeners += listener }
    @Synchronized fun remove(listener: () -> Unit) { listeners -= listener }
    fun changed() = synchronized(this) { listeners.toList() }.forEach { it() }
}
