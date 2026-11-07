package com.desklink.android

import android.content.Context
import android.graphics.Point
import android.hardware.display.DisplayManager
import android.os.Build
import android.view.Display
import android.view.WindowManager
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * The phone advertises its screen in logical units so a computer's mouse moves the pointer at a
 * comfortable physical speed. One unit is `scale` physical pixels.
 */
data class ScreenGeometry(val widthPx: Int, val heightPx: Int, val scale: Float) {
    val logicalWidth: Int get() = max(3, (widthPx / scale).roundToInt())
    val logicalHeight: Int get() = max(3, (heightPx / scale).roundToInt())

    fun toPixels(x: Int, y: Int): Point = Point(
        (x * scale).roundToInt().coerceIn(0, widthPx - 1),
        (y * scale).roundToInt().coerceIn(0, heightPx - 1),
    )

    companion object {
        fun current(context: Context, speed: Float): ScreenGeometry {
            val size = Point()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val bounds = context.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
                size.set(bounds.width(), bounds.height())
            } else {
                @Suppress("DEPRECATION")
                context.getSystemService(DisplayManager::class.java)
                    .getDisplay(Display.DEFAULT_DISPLAY).getRealSize(size)
            }
            val density = context.resources.displayMetrics.density
            return ScreenGeometry(size.x, size.y, (density * speed).coerceAtLeast(0.5f))
        }
    }
}
