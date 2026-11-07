package com.desklink.android.input

/** Performs remote input on this phone. Called on the main thread with physical pixels. */
interface InputBackend {
    val name: String
    val available: Boolean
    fun move(x: Int, y: Int)
    fun button(button: Int, down: Boolean)
    fun key(keycode: Int, meta: Int, down: Boolean)
    fun scroll(horizontal: Int, vertical: Int)
    /** Ends any held touch, button or key, e.g. when control returns to the computer. */
    fun releaseAll()
}

/** Desklink button numbers (X11 order with back/forward compacted). */
object Buttons {
    const val LEFT = 1
    const val MIDDLE = 2
    const val RIGHT = 3
    const val BACK = 4
    const val FORWARD = 5
}
