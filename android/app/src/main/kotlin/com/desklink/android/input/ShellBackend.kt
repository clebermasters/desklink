package com.desklink.android.input

import android.content.Context
import android.os.SystemClock
import android.util.Log
import com.desklink.android.shell.ShellHelperChannel
import java.io.BufferedOutputStream
import java.io.DataOutputStream
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger

/** Client of the ADB input helper ([com.desklink.android.shell.InputHelper]). */
class ShellBackend(private val context: Context) : InputBackend {
    override val name = "ADB developer mode"

    @Volatile
    private var connected = false
    override val available: Boolean get() = connected

    private val io = Executors.newSingleThreadExecutor { Thread(it, "desklink-shell-input") }
    private val queued = AtomicInteger()
    private var socket: Socket? = null
    private var output: DataOutputStream? = null
    private var lastAttempt = 0L
    private val onAnnounce: () -> Unit = { io.execute { disconnect(); connect(force = true) } }

    init {
        ShellHelperChannel.addListener(onAnnounce)
        io.execute { connect(force = true) }
    }

    fun close() {
        ShellHelperChannel.removeListener(onAnnounce)
        io.execute { disconnect() }
        io.shutdown()
    }

    /** Asks the helper process to exit. */
    fun stopHelper() = command { writeByte(QUIT) }

    private fun connect(force: Boolean) {
        if (connected) return
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastAttempt < 2_000) return
        lastAttempt = now
        val endpoint = ShellHelperChannel.endpoint(context) ?: return
        try {
            val s = Socket()
            s.tcpNoDelay = true
            s.connect(InetSocketAddress(InetAddress.getByName("127.0.0.1"), endpoint.port), 1_000)
            s.soTimeout = 2_000
            val out = DataOutputStream(BufferedOutputStream(s.getOutputStream()))
            out.write(endpoint.token.chunked(2).map { it.toInt(16).toByte() }.toByteArray())
            out.flush()
            if (s.getInputStream().read() != 1) throw IOException("helper rejected the token")
            socket = s
            output = out
            connected = true
            Log.i(TAG, "connected to the ADB input helper")
        } catch (error: IOException) {
            Log.i(TAG, "ADB input helper unavailable: ${error.message}")
            disconnect()
        }
    }

    private fun disconnect() {
        connected = false
        runCatching { socket?.close() }
        socket = null
        output = null
    }

    private fun command(droppable: Boolean = false, write: DataOutputStream.() -> Unit) {
        if (droppable && queued.get() > 256) return
        queued.incrementAndGet()
        io.execute {
            queued.decrementAndGet()
            connect(force = false)
            val out = output ?: return@execute
            try {
                out.write()
                out.flush()
            } catch (error: IOException) {
                Log.w(TAG, "ADB input helper connection lost: ${error.message}")
                disconnect()
            }
        }
    }

    override fun move(x: Int, y: Int) = command(droppable = true) { writeByte(MOVE); writeInt(x); writeInt(y) }
    override fun button(button: Int, down: Boolean) =
        command { writeByte(BUTTON); writeByte(button); writeByte(if (down) 1 else 0) }
    override fun key(keycode: Int, meta: Int, down: Boolean) =
        command { writeByte(KEY); writeInt(keycode); writeInt(meta); writeByte(if (down) 1 else 0) }
    override fun scroll(horizontal: Int, vertical: Int) =
        command { writeByte(SCROLL); writeInt(horizontal); writeInt(vertical) }
    override fun releaseAll() = command { writeByte(RELEASE_ALL) }

    companion object {
        private const val TAG = "Desklink"
        private const val MOVE = 1
        private const val BUTTON = 2
        private const val KEY = 3
        private const val SCROLL = 4
        private const val RELEASE_ALL = 5
        private const val QUIT = 6

        /** The command a computer runs to start the helper, shown in the app. */
        fun launchCommand(context: Context): String {
            val apk = context.applicationInfo.sourceDir
            return "adb shell \"CLASSPATH=$apk nohup app_process /system/bin " +
                "com.desklink.android.shell.InputHelper > /data/local/tmp/desklink-helper.log 2>&1 &\""
        }
    }
}
