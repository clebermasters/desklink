package com.desklink.android.clipboard

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Log
import com.desklink.android.NativeBridge
import java.io.ByteArrayOutputStream
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.Executors

/**
 * Text and PNG clipboard exchange. Android lets only the focused app (or the active keyboard) read
 * the clipboard, so copies made on the phone are sent when Desklink is in front: from the app,
 * the notification's "Send clipboard" action, or immediately on Android 9 and older.
 */
object ClipboardSync {
    private const val TAG = "Desklink"
    private const val MAX_BYTES = 16 * 1024 * 1024
    private const val MAX_PIXELS = 16L * 1024 * 1024
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "desklink-clipboard") }

    @Volatile
    private var lastDigest: String? = null
    private var listener: ClipboardManager.OnPrimaryClipChangedListener? = null

    /** Applies a copy made on a computer. Main thread. */
    fun apply(context: Context, kind: Int, data: ByteArray) {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        val clip = when (kind) {
            NativeBridge.CLIPBOARD_TEXT -> ClipData.newPlainText("Desklink", String(data, Charsets.UTF_8))
            NativeBridge.CLIPBOARD_PNG -> {
                val uri = ClipboardImageProvider.store(context, data) ?: return
                ClipData(ClipDescription("Desklink image", arrayOf("image/png")), ClipData.Item(uri))
            }
            else -> return
        }
        lastDigest = digest(kind, data)
        clipboard.setPrimaryClip(clip)
    }

    /** Listens for copies; delivery works whenever Android allows this app to read the clipboard. */
    fun watch(context: Context) {
        if (listener != null) return
        val app = context.applicationContext
        val clipboard = app.getSystemService(ClipboardManager::class.java)
        listener = ClipboardManager.OnPrimaryClipChangedListener { sendCurrent(app) }.also {
            clipboard.addPrimaryClipChangedListener(it)
        }
    }

    fun unwatch(context: Context) {
        // Same manager instance the listener was added to in watch().
        listener?.let {
            context.applicationContext.getSystemService(ClipboardManager::class.java).removePrimaryClipChangedListener(it)
        }
        listener = null
    }

    /**
     * Reads the clipboard now (call while an activity of this app has focus) and sends it to
     * connected computers. Returns false when there was nothing readable.
     */
    fun sendCurrent(context: Context): Boolean {
        val clip = try {
            context.getSystemService(ClipboardManager::class.java).primaryClip
        } catch (error: SecurityException) {
            null
        } ?: return false
        if (clip.itemCount == 0) return false
        val item = clip.getItemAt(0)
        val description = clip.description
        val imageUri = item.uri?.takeIf { description.hasMimeType("image/*") }
        val text = if (imageUri == null) item.coerceToText(context)?.toString() else null
        if (imageUri == null && text.isNullOrEmpty()) return false
        val app = context.applicationContext
        worker.execute {
            try {
                if (imageUri != null) sendImage(app, imageUri) else send(NativeBridge.CLIPBOARD_TEXT, text!!.toByteArray())
            } catch (error: Exception) {
                Log.w(TAG, "could not send clipboard: ${error.message}")
            }
        }
        return true
    }

    private fun sendImage(context: Context, uri: Uri) {
        val bytes = context.contentResolver.openInputStream(uri)?.use { stream ->
            val out = ByteArrayOutputStream()
            val buffer = ByteArray(64 * 1024)
            while (true) {
                val n = stream.read(buffer)
                if (n < 0) break
                out.write(buffer, 0, n)
                if (out.size() > MAX_BYTES) throw IllegalStateException("image is too large")
            }
            out.toByteArray()
        } ?: return
        if (bytes.size > 8 && bytes[0] == 0x89.toByte() && bytes[1] == 'P'.code.toByte()) {
            send(NativeBridge.CLIPBOARD_PNG, bytes)
            return
        }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outWidth.toLong() * bounds.outHeight > MAX_PIXELS) return
        val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: return
        val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
        bitmap.recycle()
        send(NativeBridge.CLIPBOARD_PNG, png)
    }

    private fun send(kind: Int, data: ByteArray) {
        if (data.isEmpty() || data.size > MAX_BYTES) return
        val digest = digest(kind, data)
        if (digest == lastDigest) return
        lastDigest = digest
        if (NativeBridge.isRunning()) NativeBridge.localClipboard(kind, data)
    }

    private fun digest(kind: Int, data: ByteArray): String =
        MessageDigest.getInstance("SHA-256").run {
            update(kind.toByte())
            update(data)
            digest().joinToString("") { "%02x".format(it) }
        }
}
