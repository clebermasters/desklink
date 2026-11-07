package com.desklink.android.input

import android.content.Context
import android.graphics.Point
import android.provider.Settings as AndroidSettings
import android.view.WindowManager
import com.desklink.android.Action
import com.desklink.android.InputMode
import com.desklink.android.ScreenGeometry
import com.desklink.android.Settings
import com.desklink.android.clipboard.ClipboardSync

/**
 * Applies agent actions on the main thread. A receiving session keeps the backend it started
 * with, so presses and their releases always go to the same place.
 */
class InputRouter(private val context: Context, private val settings: Settings) {
    @Volatile
    var geometry: ScreenGeometry = ScreenGeometry(1080, 1920, 2f)

    private val accessibility = AccessibilityBackend({ geometry.widthPx }, { geometry.heightPx })
    private val shell = ShellBackend(context)
    private var appOverlay: CursorOverlay? = null
    private var session: InputBackend? = null
    private var overlay: CursorOverlay? = null
    private val position = Point()

    /** True while a computer is controlling this phone. */
    var receiving = false
        private set
    var onReceivingChanged: (Boolean) -> Unit = {}

    val shellBackend: ShellBackend get() = shell

    fun preferredBackend(): InputBackend? {
        val ordered = if (settings.inputMode == InputMode.SHELL) listOf(shell, accessibility) else listOf(accessibility, shell)
        return ordered.firstOrNull { it.available }
    }

    fun perform(action: Action) {
        when (action) {
            is Action.Move -> {
                if (!receiving) begin()
                val px = geometry.toPixels(action.x, action.y)
                position.set(px.x, px.y)
                session?.move(px.x, px.y)
                overlay?.show(px.x, px.y)
            }
            Action.Hide -> end()
            is Action.Button -> session?.button(action.button, action.down)
            is Action.Key -> session?.key(action.keycode, action.meta, action.down)
            is Action.Scroll -> session?.scroll(action.horizontal, action.vertical)
            is Action.Clipboard -> ClipboardSync.apply(context, action.kind, action.data)
            is Action.Report -> Unit // only the Bluetooth keyboard agent produces reports
        }
    }

    private fun begin() {
        receiving = true
        session = preferredBackend()
        overlay = cursorOverlay()
        android.util.Log.i("Desklink", "receiving input through ${session?.name ?: "no input method (enable one in the app)"}")
        onReceivingChanged(true)
    }

    private fun end() {
        session?.releaseAll()
        overlay?.hide()
        session = null
        overlay = null
        if (receiving) {
            receiving = false
            onReceivingChanged(false)
        }
    }

    private fun cursorOverlay(): CursorOverlay? {
        DesklinkAccessibilityService.instance?.let { return it.overlay }
        if (!AndroidSettings.canDrawOverlays(context)) return null
        return appOverlay ?: CursorOverlay(
            context.applicationContext,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
        ).also { appOverlay = it }
    }

    fun close() {
        end()
        shell.close()
    }
}
