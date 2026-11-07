package com.desklink.android.input

import android.accessibilityservice.AccessibilityService
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent

/**
 * The default input mode. Needs only the user's accessibility approval, no developer settings.
 * It performs gestures and key presses for the paired computer; it ignores window events.
 */
class DesklinkAccessibilityService : AccessibilityService() {
    lateinit var overlay: CursorOverlay
        private set

    override fun onServiceConnected() {
        overlay = CursorOverlay(this, WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY)
        instance = this
        listeners.forEach { it() }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        disconnect()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        disconnect()
        super.onDestroy()
    }

    private fun disconnect() {
        if (instance === this) {
            overlay.hide()
            instance = null
            listeners.forEach { it() }
        }
    }

    companion object {
        /** Set while the user has the service enabled. Accessed on the main thread. */
        @Volatile
        var instance: DesklinkAccessibilityService? = null
            private set
        val listeners = mutableListOf<() -> Unit>()
    }
}
