package com.desklink.android.clipboard

import android.app.Activity
import android.widget.Toast

/** Gains focus for an instant so Android allows reading the clipboard, sends it, and closes. */
class SendClipboardActivity : Activity() {
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) return
        val sent = ClipboardSync.sendCurrent(this)
        Toast.makeText(this, if (sent) "Clipboard sent" else "The clipboard is empty", Toast.LENGTH_SHORT).show()
        finish()
    }
}
