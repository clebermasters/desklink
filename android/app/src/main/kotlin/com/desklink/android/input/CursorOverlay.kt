package com.desklink.android.input

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PixelFormat
import android.os.Build
import android.view.Choreographer
import android.view.Gravity
import android.view.View
import android.view.WindowManager

/**
 * Draws the remote pointer. The window starts one pixel right of and below the hotspot, so the
 * pointer never covers the point being tapped (Android 12+ blocks touches that pass through
 * opaque overlays of other apps). Position updates are coalesced to one per display frame.
 */
class CursorOverlay(private val context: Context, private val windowType: Int) {
    private val windowManager = context.getSystemService(WindowManager::class.java)
    private val density = context.resources.displayMetrics.density
    private val view = Arrow(context, density)
    private val params = WindowManager.LayoutParams(
        (18 * density).toInt(),
        (26 * density).toInt(),
        windowType,
        WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
            WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
        PixelFormat.TRANSLUCENT,
    ).apply {
        gravity = Gravity.TOP or Gravity.START
        title = "Desklink pointer"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
        }
    }
    private var attached = false
    private var frameScheduled = false
    private var targetX = 0
    private var targetY = 0
    private val frame = Choreographer.FrameCallback {
        frameScheduled = false
        params.x = targetX + 1
        params.y = targetY + 1
        try {
            if (!attached) {
                windowManager.addView(view, params)
                attached = true
            } else {
                windowManager.updateViewLayout(view, params)
            }
        } catch (error: RuntimeException) {
            // The window token can disappear when the service is disabled; drawing is optional.
            attached = false
        }
    }

    fun show(x: Int, y: Int) {
        targetX = x
        targetY = y
        if (!frameScheduled) {
            frameScheduled = true
            Choreographer.getInstance().postFrameCallback(frame)
        }
    }

    fun hide() {
        if (frameScheduled) {
            Choreographer.getInstance().removeFrameCallback(frame)
            frameScheduled = false
        }
        if (attached) {
            try {
                windowManager.removeViewImmediate(view)
            } catch (_: RuntimeException) {
            }
            attached = false
        }
    }

    private class Arrow(context: Context, density: Float) : View(context) {
        private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE; style = Paint.Style.FILL }
        private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.BLACK
            style = Paint.Style.STROKE
            strokeWidth = 1.5f * density
            strokeJoin = Paint.Join.ROUND
        }
        private val path = Path().apply {
            val d = density
            moveTo(1 * d, 1 * d)
            lineTo(1 * d, 21 * d)
            lineTo(6 * d, 16 * d)
            lineTo(9.5f * d, 24 * d)
            lineTo(12.5f * d, 22.7f * d)
            lineTo(9 * d, 15 * d)
            lineTo(16 * d, 15 * d)
            close()
        }

        override fun onDraw(canvas: Canvas) {
            canvas.drawPath(path, fill)
            canvas.drawPath(path, stroke)
        }
    }
}
