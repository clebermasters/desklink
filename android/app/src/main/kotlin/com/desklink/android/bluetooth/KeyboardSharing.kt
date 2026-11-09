package com.desklink.android.bluetooth

import android.content.Context
import android.os.Process
import android.os.SystemClock
import android.util.Log
import com.desklink.android.Action
import com.desklink.android.NativeBridge
import kotlin.concurrent.thread

/**
 * Runs while sharing is on and a Bluetooth computer is set up: the keyboard agent (Rust) receives
 * input from the controlling computer, and its HID reports go out through [HidKeyboardMouse].
 *
 * Android withdraws a HID registration from apps that leave the foreground, so this lives in the
 * foreground [com.desklink.android.SharingService].
 */
class KeyboardSharing(context: Context) {
    private val settings = KeyboardSettings(context)
    private val hid = HidKeyboardMouse.get(context)
    private var pump: Thread? = null
    @Volatile private var stopping = false

    @Volatile var error: String? = null
        private set

    private val rememberHost: (HidKeyboardMouse.State) -> Unit = { state ->
        val host = hid.host
        if (state == HidKeyboardMouse.State.CONNECTED && host != null && settings.deviceAddress == null) {
            settings.deviceAddress = host.address
            hid.preferredAddress = host.address
        }
    }

    @Synchronized
    fun start() {
        if (pump != null || !settings.enabled) return
        stopping = false
        error = null
        hid.preferredAddress = settings.deviceAddress
        hid.listeners += rememberHost
        hid.start()
        val dir = settings.profileDir
        pump = thread(name = "desklink-keyboard") {
            runCatching { Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_DISPLAY) }
            try {
                NativeBridge.keyboardEnsure(dir, KeyboardSettings.DEFAULT_NAME)
                NativeBridge.keyboardStart(
                    dir, settings.width, settings.height, settings.relativePointer, settings.swapControlAndCommand,
                )
            } catch (failure: IllegalStateException) {
                error = failure.message
                Log.e(TAG, "the Bluetooth keyboard could not start: ${failure.message}")
                return@thread
            }
            var lastBatch = 0L
            var lastReconnect = SystemClock.elapsedRealtime()
            while (!stopping) {
                val bytes = try {
                    NativeBridge.keyboardNextActions(1_000)
                } catch (failure: IllegalStateException) {
                    null
                } ?: break
                val now = SystemClock.elapsedRealtime()
                if (now - lastReconnect > RECONNECT_MS && hid.state == HidKeyboardMouse.State.READY) {
                    lastReconnect = now
                    hid.start() // the computer slept or walked out of range; try again
                }
                if (bytes.isEmpty()) continue
                for (action in Action.decode(bytes)) {
                    if (action is Action.Report) hid.send(action.id, action.data)
                }
                // Bluetooth carries about 125 reports a second. While this thread waits, newer
                // pointer positions replace older ones in the native queue.
                val wait = BATCH_MS - (SystemClock.elapsedRealtime() - lastBatch)
                if (wait > 0) SystemClock.sleep(wait)
                lastBatch = SystemClock.elapsedRealtime()
            }
        }
    }

    @Synchronized
    fun stop() {
        stopping = true
        hid.listeners -= rememberHost
        runCatching { NativeBridge.keyboardStop() }
        pump?.join(4_000)
        pump = null
        hid.stop()
    }

    companion object {
        private const val TAG = "Desklink"
        private const val BATCH_MS = 8L
        private const val RECONNECT_MS = 10_000L
    }
}
