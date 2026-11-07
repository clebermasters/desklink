package com.desklink.android.shell

import android.os.Build
import android.os.Looper
import android.os.SystemClock
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import java.io.BufferedInputStream
import java.io.DataInputStream
import java.io.IOException
import java.lang.reflect.Method
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom
import kotlin.concurrent.thread
import kotlin.system.exitProcess

/**
 * ADB developer mode. Started from a computer with `adb shell` (see scripts/android-input-helper.sh),
 * this process runs as the shell user and injects real mouse and keyboard events, including hover,
 * right click, wheel and keyboard shortcuts in every app.
 *
 * Security:
 * - Listens only on 127.0.0.1, on a random port.
 * - A client must present a random 32-byte token before any command is accepted.
 * - The port and token are delivered only to the Desklink app, through its content provider,
 *   which accepts calls only from the shell user. Other apps can neither learn the token nor
 *   impersonate the helper.
 */
object InputHelper {
    private const val PACKAGE = "com.desklink.android"
    private const val MOVE = 1
    private const val BUTTON = 2
    private const val KEY = 3
    private const val SCROLL = 4
    private const val RELEASE_ALL = 5
    private const val QUIT = 6

    private lateinit var injector: Injector

    @JvmStatic
    fun main(args: Array<String>) {
        Looper.prepare()
        injector = Injector()
        val token = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val server = ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"))
        val announcement = "${server.localPort}:${token.toHex()}"
        log("listening on 127.0.0.1:${server.localPort}")
        thread(isDaemon = true, name = "announce") {
            // Re-announce periodically so a restarted app can reconnect without restarting us.
            while (true) {
                announce(announcement)
                Thread.sleep(5_000)
            }
        }
        while (true) {
            val client = try {
                server.accept()
            } catch (error: IOException) {
                log("accept failed: $error")
                return
            }
            // One thread per connection: an unauthenticated or stale client never blocks the app.
            thread(isDaemon = true, name = "client") { handle(client, token) }
        }
    }

    private val lock = Any()
    private var current: Socket? = null

    private fun handle(client: Socket, token: ByteArray) {
        try {
            val input = authenticate(client, token) ?: return
            synchronized(lock) {
                runCatching { current?.close() }
                current = client
            }
            log("app connected")
            serve(input)
        } catch (error: QuitRequested) {
            log("stopped by the app")
            synchronized(lock) { injector.releaseAll() }
            exitProcess(0)
        } catch (error: Exception) {
            log("client ended: $error")
        } finally {
            synchronized(lock) {
                if (current === client) {
                    injector.releaseAll()
                    current = null
                }
            }
            runCatching { client.close() }
        }
    }

    private class QuitRequested : Exception()

    private fun authenticate(socket: Socket, token: ByteArray): DataInputStream? {
        socket.tcpNoDelay = true
        socket.soTimeout = 3_000
        val input = DataInputStream(BufferedInputStream(socket.getInputStream()))
        val presented = ByteArray(token.size)
        input.readFully(presented)
        if (!MessageDigest.isEqual(presented, token)) {
            log("rejected a client with a wrong token")
            return null
        }
        socket.getOutputStream().apply { write(1); flush() }
        socket.soTimeout = 0
        return input
    }

    private fun serve(input: DataInputStream) {
        while (true) {
            val op = input.readUnsignedByte()
            synchronized(lock) {
                when (op) {
                    MOVE -> injector.move(input.readInt(), input.readInt())
                    BUTTON -> injector.button(input.readUnsignedByte(), input.readUnsignedByte() != 0)
                    KEY -> injector.key(input.readInt(), input.readInt(), input.readUnsignedByte() != 0)
                    SCROLL -> injector.scroll(input.readInt(), input.readInt())
                    RELEASE_ALL -> injector.releaseAll()
                    QUIT -> throw QuitRequested()
                    else -> throw IOException("unknown command $op")
                }
            }
        }
    }

    private fun announce(value: String) {
        try {
            val process = ProcessBuilder(
                "content", "call", "--uri", "content://$PACKAGE.shell", "--method", "register", "--arg", value,
            ).redirectErrorStream(true).start()
            process.inputStream.readBytes()
            process.waitFor()
        } catch (error: Exception) {
            log("cannot reach the Desklink app: $error")
        }
    }

    private fun log(message: String) = System.out.println("desklink-helper: $message").also { System.out.flush() }

    private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
}

/** Injects events with the shell user's INJECT_EVENTS permission through hidden framework APIs. */
private class Injector {
    private val manager: Any
    private val inject: Method
    private val setActionButton: Method? = runCatching {
        MotionEvent::class.java.getMethod("setActionButton", Int::class.javaPrimitiveType)
    }.getOrNull()
    private var x = 0f
    private var y = 0f
    private var buttons = 0
    private var downTime = 0L
    private val heldKeys = HashMap<Int, Long>()
    private val repeats = HashMap<Int, Int>()

    init {
        manager = if (Build.VERSION.SDK_INT >= 34) {
            Class.forName("android.hardware.input.InputManagerGlobal").getMethod("getInstance").invoke(null)!!
        } else {
            android.hardware.input.InputManager::class.java.getDeclaredMethod("getInstance").invoke(null)!!
        }
        inject = manager.javaClass.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
    }

    private fun send(event: InputEvent) {
        // INJECT_INPUT_EVENT_MODE_ASYNC: never block the socket reader on a busy app.
        inject.invoke(manager, event, 0)
    }

    fun move(px: Int, py: Int) {
        x = px.toFloat()
        y = py.toFloat()
        motion(if (buttons == 0) MotionEvent.ACTION_HOVER_MOVE else MotionEvent.ACTION_MOVE, 0)
    }

    fun button(button: Int, down: Boolean) {
        when (button) {
            4 -> return key(KeyEvent.KEYCODE_BACK, 0, down)
            5 -> return key(KeyEvent.KEYCODE_FORWARD, 0, down)
        }
        val flag = when (button) {
            1 -> MotionEvent.BUTTON_PRIMARY
            2 -> MotionEvent.BUTTON_TERTIARY
            3 -> MotionEvent.BUTTON_SECONDARY
            else -> return
        }
        if (down == (buttons and flag != 0)) return
        if (down) {
            if (buttons == 0) {
                downTime = SystemClock.uptimeMillis()
                buttons = flag
                motion(MotionEvent.ACTION_DOWN, 0)
            } else {
                buttons = buttons or flag
            }
            motion(MotionEvent.ACTION_BUTTON_PRESS, flag)
        } else {
            buttons = buttons and flag.inv()
            motion(MotionEvent.ACTION_BUTTON_RELEASE, flag)
            if (buttons == 0) {
                motion(MotionEvent.ACTION_UP, 0)
                motion(MotionEvent.ACTION_HOVER_MOVE, 0)
            }
        }
    }

    fun scroll(horizontal: Int, vertical: Int) {
        motion(MotionEvent.ACTION_SCROLL, 0) { coords ->
            coords.setAxisValue(MotionEvent.AXIS_VSCROLL, vertical.toFloat())
            coords.setAxisValue(MotionEvent.AXIS_HSCROLL, horizontal.toFloat())
        }
    }

    fun key(keycode: Int, meta: Int, down: Boolean) {
        val now = SystemClock.uptimeMillis()
        val event = if (down) {
            val repeat = if (heldKeys.containsKey(keycode)) (repeats[keycode] ?: 0) + 1 else 0
            if (repeat == 0) heldKeys[keycode] = now
            repeats[keycode] = repeat
            KeyEvent(heldKeys[keycode]!!, now, KeyEvent.ACTION_DOWN, keycode, repeat, meta,
                KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)
        } else {
            val pressed = heldKeys.remove(keycode) ?: now
            repeats.remove(keycode)
            KeyEvent(pressed, now, KeyEvent.ACTION_UP, keycode, 0, meta,
                KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)
        }
        send(event)
    }

    fun releaseAll() {
        for (flag in listOf(MotionEvent.BUTTON_PRIMARY, MotionEvent.BUTTON_SECONDARY, MotionEvent.BUTTON_TERTIARY)) {
            if (buttons and flag != 0) {
                button(
                    when (flag) {
                        MotionEvent.BUTTON_PRIMARY -> 1
                        MotionEvent.BUTTON_TERTIARY -> 2
                        else -> 3
                    },
                    false,
                )
            }
        }
        heldKeys.keys.toList().forEach { key(it, 0, false) }
    }

    private fun motion(action: Int, actionButton: Int, axes: (MotionEvent.PointerCoords) -> Unit = {}) {
        val now = SystemClock.uptimeMillis()
        val properties = arrayOf(MotionEvent.PointerProperties().apply {
            id = 0
            toolType = MotionEvent.TOOL_TYPE_MOUSE
        })
        val coords = arrayOf(MotionEvent.PointerCoords().apply {
            this.x = this@Injector.x
            this.y = this@Injector.y
            pressure = if (buttons != 0) 1f else 0f
            size = 1f
            axes(this)
        })
        val start = if (buttons != 0 || action == MotionEvent.ACTION_UP) downTime else now
        val event = MotionEvent.obtain(
            start, now, action, 1, properties, coords, 0, buttons, 1f, 1f, 0, 0, InputDevice.SOURCE_MOUSE, 0,
        )
        if (actionButton != 0) setActionButton?.invoke(event, actionButton)
        try {
            send(event)
        } finally {
            event.recycle()
        }
    }
}
