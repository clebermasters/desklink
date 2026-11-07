package com.desklink.android.input

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.graphics.Path
import android.graphics.PointF
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.accessibility.AccessibilityNodeInfo
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.roundToLong

/**
 * Turns mouse and keyboard input into accessibility gestures and key events.
 *
 * - Left button: a touch that follows the pointer while held (tap, drag, long press).
 * - Right button: long press. Back button and Escape: Android Back.
 * - Wheel: short swipes under the pointer.
 * - Keys: the accessibility input method (Android 13+), or text-field editing on older versions.
 */
class AccessibilityBackend(private val screenWidth: () -> Int, private val screenHeight: () -> Int) :
    InputBackend {
    override val name = "Accessibility"
    override val available: Boolean get() = DesklinkAccessibilityService.instance != null
    private val service: DesklinkAccessibilityService?
        get() = DesklinkAccessibilityService.instance.also { current ->
            // A rebound service never reports gestures dispatched through the old connection.
            if (current !== boundTo) {
                boundTo = current
                reset()
            }
        }
    private var boundTo: DesklinkAccessibilityService? = null
    private var dispatchedAt = 0L

    private val pointer = PointF()
    private var held: GestureDescription.StrokeDescription? = null
    private var heldEnd = PointF()
    private var releasing = false
    private var inFlight = false
    private var scrollX = 0
    private var scrollY = 0
    private val keyDownTimes = HashMap<Int, Long>()

    /** Gestures last at most ~650 ms; a callback missing for longer was lost by the system. */
    private fun recoverLostGesture() {
        service
        if (inFlight && SystemClock.uptimeMillis() - dispatchedAt > 2_000) reset()
    }

    override fun move(x: Int, y: Int) {
        recoverLostGesture()
        pointer.set(x.toFloat(), y.toFloat())
        if (held != null && !inFlight) continueHeld()
    }

    override fun button(button: Int, down: Boolean) {
        when (button) {
            Buttons.LEFT -> if (down) press() else release()
            Buttons.RIGHT -> if (down) longPress()
            Buttons.BACK -> if (down) service?.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
        }
    }

    private fun press() {
        recoverLostGesture()
        if (held != null || inFlight) return
        val stroke = GestureDescription.StrokeDescription(pointPath(pointer), 0, 1, true)
        held = stroke
        heldEnd.set(pointer)
        releasing = false
        dispatch(stroke)
    }

    private fun release() {
        if (held == null) return
        releasing = true
        if (!inFlight) continueHeld()
    }

    private fun continueHeld() {
        val previous = held ?: return
        val path = Path().apply {
            moveTo(heldEnd.x, heldEnd.y)
            lineTo(pointer.x, pointer.y)
        }
        val distance = hypot(pointer.x - heldEnd.x, pointer.y - heldEnd.y)
        val duration = (distance / 2f).roundToLong().coerceIn(1, 16)
        val next = previous.continueStroke(path, 0, duration, !releasing)
        heldEnd.set(pointer)
        held = if (releasing) null else next
        dispatch(next)
    }

    private fun longPress() {
        recoverLostGesture()
        if (held != null || inFlight) return
        dispatch(GestureDescription.StrokeDescription(pointPath(pointer), 0, 650))
    }

    override fun scroll(horizontal: Int, vertical: Int) {
        recoverLostGesture()
        scrollX += horizontal
        scrollY += vertical
        if (held == null && !inFlight) flushScroll()
    }

    private fun flushScroll() {
        if (scrollX == 0 && scrollY == 0) return
        val step = 64f * (service?.resources?.displayMetrics?.density ?: 2f)
        // Wheel up shows earlier content, which a finger does by moving down.
        val endX = (pointer.x - scrollX * step).coerceIn(1f, screenWidth() - 2f)
        val endY = (pointer.y + scrollY * step).coerceIn(1f, screenHeight() - 2f)
        scrollX = 0
        scrollY = 0
        if (abs(endX - pointer.x) < 1 && abs(endY - pointer.y) < 1) return
        val path = Path().apply {
            moveTo(pointer.x, pointer.y)
            lineTo(endX, endY)
        }
        dispatch(GestureDescription.StrokeDescription(path, 0, 160))
    }

    private fun dispatch(stroke: GestureDescription.StrokeDescription) {
        val service = service ?: run { reset(); return }
        inFlight = true
        dispatchedAt = SystemClock.uptimeMillis()
        val accepted = service.dispatchGesture(
            GestureDescription.Builder().addStroke(stroke).build(),
            object : AccessibilityService.GestureResultCallback() {
                override fun onCompleted(gestureDescription: GestureDescription?) = next()
                override fun onCancelled(gestureDescription: GestureDescription?) {
                    android.util.Log.d("Desklink", "gesture cancelled")
                    // A real touch or the system interrupted us; the held touch is gone.
                    held = null
                    next()
                }
            },
            null,
        )
        if (!accepted) {
            android.util.Log.w("Desklink", "Android refused a gesture")
            reset()
        }
    }

    private fun next() {
        inFlight = false
        when {
            held != null && (releasing || heldEnd.x != pointer.x || heldEnd.y != pointer.y) -> continueHeld()
            held == null -> flushScroll()
        }
    }

    private fun reset() {
        inFlight = false
        held = null
        releasing = false
    }

    override fun key(keycode: Int, meta: Int, down: Boolean) {
        val service = service ?: return
        if (keycode == KeyEvent.KEYCODE_ESCAPE || keycode == KeyEvent.KEYCODE_BACK) {
            if (down) service.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
            return
        }
        if (keycode == KeyEvent.KEYCODE_HOME) {
            if (down) service.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME)
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val connection = service.inputMethod?.currentInputConnection ?: return
            val now = SystemClock.uptimeMillis()
            val downTime = if (down) now.also { keyDownTimes[keycode] = it } else keyDownTimes.remove(keycode) ?: now
            connection.sendKeyEvent(
                KeyEvent(
                    downTime, now, if (down) KeyEvent.ACTION_DOWN else KeyEvent.ACTION_UP, keycode, 0, meta,
                    KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0,
                ),
            )
        } else if (down) {
            LegacyTextInput.key(service, keycode, meta)
        }
    }

    override fun releaseAll() {
        if (held != null) {
            releasing = true
            if (!inFlight) continueHeld()
        }
        scrollX = 0
        scrollY = 0
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val connection = service?.inputMethod?.currentInputConnection
            val now = SystemClock.uptimeMillis()
            keyDownTimes.forEach { (code, downTime) ->
                connection?.sendKeyEvent(
                    KeyEvent(downTime, now, KeyEvent.ACTION_UP, code, 0, 0, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0),
                )
            }
        }
        keyDownTimes.clear()
    }

    private fun pointPath(point: PointF) = Path().apply {
        moveTo(point.x, point.y)
        lineTo(point.x, point.y)
    }
}

/** Android 8–12: edits the focused text field directly, since there is no key-injection API. */
private object LegacyTextInput {
    private val characters = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD)
    private const val SHORTCUT_META = KeyEvent.META_CTRL_ON or KeyEvent.META_META_ON

    fun key(service: AccessibilityService, keycode: Int, meta: Int) {
        val node = service.rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return
        if (!node.isEditable) return
        val text = node.text?.toString() ?: ""
        val start = node.textSelectionStart.takeIf { it in 0..text.length } ?: text.length
        val end = node.textSelectionEnd.takeIf { it in start..text.length } ?: start
        if (meta and SHORTCUT_META != 0) {
            when (keycode) {
                KeyEvent.KEYCODE_A -> select(node, 0, text.length)
                KeyEvent.KEYCODE_C -> node.performAction(AccessibilityNodeInfo.ACTION_COPY)
                KeyEvent.KEYCODE_X -> node.performAction(AccessibilityNodeInfo.ACTION_CUT)
                KeyEvent.KEYCODE_V -> node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            }
            return
        }
        when (keycode) {
            KeyEvent.KEYCODE_DEL ->
                if (start != end) replace(node, text, start, end, "")
                else if (start > 0) replace(node, text, start - 1, start, "")
            KeyEvent.KEYCODE_FORWARD_DEL ->
                if (start != end) replace(node, text, start, end, "")
                else if (end < text.length) replace(node, text, start, end + 1, "")
            KeyEvent.KEYCODE_DPAD_LEFT -> (start - 1).coerceAtLeast(0).let { select(node, it, it) }
            KeyEvent.KEYCODE_DPAD_RIGHT -> (end + 1).coerceAtMost(text.length).let { select(node, it, it) }
            KeyEvent.KEYCODE_MOVE_HOME -> select(node, 0, 0)
            KeyEvent.KEYCODE_MOVE_END -> select(node, text.length, text.length)
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER ->
                if (node.isMultiLine) replace(node, text, start, end, "\n")
                else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                    node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
                }
            else -> {
                val unicode = characters.get(keycode, meta)
                if (unicode != 0 && unicode and KeyCharacterMap.COMBINING_ACCENT == 0) {
                    replace(node, text, start, end, String(Character.toChars(unicode)))
                }
            }
        }
    }

    private fun replace(node: AccessibilityNodeInfo, text: String, start: Int, end: Int, insert: String) {
        val updated = text.substring(0, start) + insert + text.substring(end)
        node.performAction(
            AccessibilityNodeInfo.ACTION_SET_TEXT,
            Bundle().apply {
                putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, updated)
            },
        )
        val cursor = start + insert.length
        select(node, cursor, cursor)
    }

    private fun select(node: AccessibilityNodeInfo, start: Int, end: Int) {
        node.performAction(
            AccessibilityNodeInfo.ACTION_SET_SELECTION,
            Bundle().apply {
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, start)
                putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, end)
            },
        )
    }
}
