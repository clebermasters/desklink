package com.desklink.android.clipboard

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File
import java.io.FileNotFoundException

/**
 * Serves the most recent image copied from a computer. Not exported: Android's clipboard grants
 * read access to the app that pastes it.
 */
class ClipboardImageProvider : ContentProvider() {
    override fun onCreate() = true

    private fun file(uri: Uri): File {
        val name = uri.lastPathSegment ?: throw FileNotFoundException()
        if (!NAME.matches(name)) throw FileNotFoundException()
        val file = File(directory(context!!), name)
        if (!file.isFile) throw FileNotFoundException()
        return file
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("read-only")
        return ParcelFileDescriptor.open(file(uri), ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri) = "image/png"

    override fun query(uri: Uri, projection: Array<String>?, s: String?, a: Array<String>?, o: String?): Cursor {
        val file = file(uri)
        return MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)).apply {
            addRow(arrayOf<Any>("desklink-clipboard.png", file.length()))
        }
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<String>?) = 0

    companion object {
        private val NAME = Regex("clip-[0-9]{1,19}\\.png")
        private fun directory(context: Context) = File(context.cacheDir, "clipboard").apply { mkdirs() }

        /** Keeps only the newest image so copied data does not accumulate. */
        fun store(context: Context, png: ByteArray): Uri? = try {
            val dir = directory(context)
            dir.listFiles()?.forEach { it.delete() }
            val name = "clip-${System.currentTimeMillis()}.png"
            File(dir, name).writeBytes(png)
            Uri.parse("content://com.desklink.android.clipboard/$name")
        } catch (error: Exception) {
            null
        }
    }
}
