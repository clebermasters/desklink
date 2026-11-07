package com.desklink.android

import java.nio.ByteBuffer

/** Work requested by the sharing agent. Coordinates are logical units; see [ScreenGeometry]. */
sealed interface Action {
    data class Move(val x: Int, val y: Int) : Action
    data object Hide : Action
    data class Button(val button: Int, val down: Boolean) : Action
    data class Key(val keycode: Int, val meta: Int, val down: Boolean) : Action
    data class Scroll(val horizontal: Int, val vertical: Int) : Action
    class Clipboard(val kind: Int, val data: ByteArray) : Action

    companion object {
        /** Mirrors `bridge::encode` in crates/android/src/bridge.rs. */
        fun decode(bytes: ByteArray): List<Action> {
            val buffer = ByteBuffer.wrap(bytes)
            val actions = ArrayList<Action>()
            while (buffer.hasRemaining()) {
                actions += when (val tag = buffer.get().toInt()) {
                    1 -> Move(buffer.int, buffer.int)
                    2 -> Hide
                    3 -> Button(buffer.get().toInt() and 0xff, buffer.get().toInt() != 0)
                    4 -> Key(buffer.int, buffer.int, buffer.get().toInt() != 0)
                    5 -> Scroll(buffer.int, buffer.int)
                    6 -> {
                        val kind = buffer.get().toInt()
                        val data = ByteArray(buffer.int)
                        buffer.get(data)
                        Clipboard(kind, data)
                    }
                    else -> throw IllegalStateException("unknown action $tag")
                }
            }
            return actions
        }
    }
}
