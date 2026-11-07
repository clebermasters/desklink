package com.desklink.android

/**
 * JNI surface of `crates/android`. Every call may throw [IllegalStateException] with a message
 * suitable for showing to the user. Calls that touch files or the network must not run on the
 * main thread.
 */
object NativeBridge {
    init {
        System.loadLibrary("desklink_android")
    }

    @JvmStatic external fun init(dir: String, name: String)
    @JvmStatic external fun info(dir: String, address: String?): String
    @JvmStatic external fun preview(code: String): String
    @JvmStatic external fun pair(dir: String, code: String, controller: Boolean): String
    @JvmStatic external fun unpair(dir: String, publicKey: String)
    @JvmStatic external fun setName(dir: String, name: String)
    @JvmStatic external fun setClipboard(dir: String, enabled: Boolean)
    @JvmStatic external fun setGeometry(width: Int, height: Int)
    @JvmStatic external fun start(dir: String)
    @JvmStatic external fun stop()
    @JvmStatic external fun isRunning(): Boolean
    @JvmStatic external fun status(dir: String): String

    /** Encoded [Action]s, an empty array on timeout, or null once sharing has stopped. */
    @JvmStatic external fun nextActions(timeoutMs: Int): ByteArray?
    @JvmStatic external fun localClipboard(kind: Int, data: ByteArray)

    /** HID report descriptor for the Bluetooth keyboard and mouse (crates/hid). */
    @JvmStatic external fun hidDescriptor(): ByteArray

    // A computer without Desklink, controlled as a Bluetooth keyboard and mouse. Its profile has
    // its own identity and port; `info`, `setName` and `unpair` also work on its directory.
    @JvmStatic external fun keyboardEnsure(dir: String, name: String)
    @JvmStatic external fun keyboardTrust(phoneDir: String, dir: String, publicKey: String): String
    @JvmStatic external fun keyboardPair(dir: String, code: String): String
    @JvmStatic external fun keyboardStart(dir: String, width: Int, height: Int, relative: Boolean, swapControlAndCommand: Boolean)
    @JvmStatic external fun keyboardStop()
    @JvmStatic external fun keyboardStatus(dir: String): String
    @JvmStatic external fun keyboardNextActions(timeoutMs: Int): ByteArray?

    const val CLIPBOARD_TEXT = 0
    const val CLIPBOARD_PNG = 1
}
