package com.desklink.android.bluetooth

import android.Manifest
import android.annotation.SuppressLint
import android.annotation.TargetApi
import android.bluetooth.BluetoothClass
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothHidDevice
import android.bluetooth.BluetoothHidDeviceAppSdpSettings
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import com.desklink.android.NativeBridge
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors

/**
 * Presents this phone to one computer as a standard Bluetooth keyboard and mouse, using
 * Android's HID device profile (Android 9+). The computer needs no Desklink software: it sees an
 * ordinary accessory, paired from its own Bluetooth settings.
 *
 * Only one app can be registered as a HID device at a time. Desklink registers only while
 * Bluetooth sharing is active and unregisters afterwards, so the phone stops presenting itself
 * as a keyboard.
 */
// Every entry point checks permission() first, and `supported` (Android 9+) before any HID call.
@SuppressLint("MissingPermission")
@TargetApi(Build.VERSION_CODES.P)
class HidKeyboardMouse private constructor(private val context: Context) {
    enum class State { UNAVAILABLE, NO_PERMISSION, STARTING, READY, CONNECTING, CONNECTED }

    private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
    private val callbacks = Executors.newSingleThreadExecutor()
    private var proxy: BluetoothHidDevice? = null
    private var wanted = false
    @Volatile private var registered = false
    @Volatile var host: BluetoothDevice? = null
        private set
    /** The computer to reconnect to once registered; `null` waits for the computer to connect. */
    @Volatile var preferredAddress: String? = null
    @Volatile var state = State.UNAVAILABLE
        private set
    /** Called on Bluetooth or caller threads. */
    val listeners = CopyOnWriteArrayList<(State) -> Unit>()

    val supported: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P && adapter != null

    fun permission(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
            context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

    /** Computers already paired with this phone, for choosing which one to control. */
    /** (address, name) of computers already paired with this phone. */
    fun bondedComputers(): List<Pair<String, String>> {
        if (!supported || !permission()) return emptyList()
        return adapter?.bondedDevices.orEmpty()
            .filter { it.bluetoothClass?.majorDeviceClass == BluetoothClass.Device.Major.COMPUTER }
            .map { it.address to (it.name ?: it.address) }
            .sortedBy { it.second }
    }

    /** The name computers see when they search for Bluetooth devices. */
    fun phoneName(): String? = if (supported && permission()) runCatching { adapter?.name }.getOrNull() else null

    fun nameOf(address: String): String? =
        if (supported && permission()) runCatching { adapter?.getRemoteDevice(address)?.name }.getOrNull() else null

    @Synchronized
    fun start() {
        wanted = true
        when {
            !supported -> return publish(State.UNAVAILABLE)
            !permission() -> return publish(State.NO_PERMISSION)
            proxy != null -> return register()
        }
        publish(State.STARTING)
        val ok = adapter!!.getProfileProxy(context, object : BluetoothProfile.ServiceListener {
            override fun onServiceConnected(profile: Int, service: BluetoothProfile) {
                synchronized(this@HidKeyboardMouse) {
                    proxy = service as BluetoothHidDevice
                    if (wanted) register() else closeProxy()
                }
            }

            override fun onServiceDisconnected(profile: Int) {
                synchronized(this@HidKeyboardMouse) {
                    proxy = null
                    registered = false
                    host = null
                    publish(if (wanted) State.STARTING else State.UNAVAILABLE)
                }
            }
        }, BluetoothProfile.HID_DEVICE)
        if (!ok) publish(State.UNAVAILABLE)
    }

    private fun register() {
        val hid = proxy ?: return
        if (registered) return reconnect()
        val sdp = BluetoothHidDeviceAppSdpSettings(
            "Desklink",
            "Keyboard and mouse shared by Desklink",
            "Desklink",
            BluetoothHidDevice.SUBCLASS1_COMBO,
            NativeBridge.hidDescriptor(),
        )
        if (!hid.registerApp(sdp, null, null, callbacks, callback)) {
            Log.w(TAG, "HID registration was refused; another app may be using the keyboard profile")
            publish(State.UNAVAILABLE)
        }
    }

    private fun reconnect() {
        val hid = proxy ?: return
        val address = preferredAddress
        if (host != null) return publish(State.CONNECTED)
        if (address == null) return publish(State.READY)
        val device = runCatching { adapter?.getRemoteDevice(address) }.getOrNull() ?: return publish(State.READY)
        publish(State.CONNECTING)
        if (!hid.connect(device)) publish(State.READY)
    }

    // Created on first registration, so Android 8 never loads the Android 9 callback class.
    private val callback by lazy { HidCallback() }

    private inner class HidCallback : BluetoothHidDevice.Callback() {
        override fun onAppStatusChanged(pluggedDevice: BluetoothDevice?, registered: Boolean) {
            synchronized(this@HidKeyboardMouse) {
                this@HidKeyboardMouse.registered = registered
                Log.i(TAG, "keyboard and mouse ${if (registered) "registered" else "unregistered"}")
                if (registered && wanted) reconnect() else if (!registered) {
                    host = null
                    publish(if (wanted) State.STARTING else State.UNAVAILABLE)
                }
            }
        }

        override fun onConnectionStateChanged(device: BluetoothDevice, state: Int) {
            synchronized(this@HidKeyboardMouse) {
                when (state) {
                    BluetoothProfile.STATE_CONNECTED -> {
                        host = device
                        Log.i(TAG, "keyboard and mouse connected to a computer")
                        publish(State.CONNECTED)
                    }
                    BluetoothProfile.STATE_DISCONNECTED -> if (host == null || host == device) {
                        host = null
                        publish(if (registered) State.READY else State.STARTING)
                    }
                }
            }
        }

        override fun onGetReport(device: BluetoothDevice, type: Byte, id: Byte, bufferSize: Int) {
            // Hosts may poll the current state; an idle report is always correct to start with.
            val size = when (id.toInt()) {
                KEYBOARD -> 8
                POINTER_ABSOLUTE -> 6
                PEN -> 5
                else -> 7
            }
            proxy?.replyReport(device, type, id, ByteArray(size))
        }

        override fun onSetReport(device: BluetoothDevice, type: Byte, id: Byte, data: ByteArray) {
            proxy?.reportError(device, BluetoothHidDevice.ERROR_RSP_SUCCESS)
        }
    }

    /** False when no computer is connected; the report is then dropped, like an unplugged mouse. */
    fun send(id: Int, data: ByteArray): Boolean {
        val hid = proxy ?: return false
        val device = host ?: return false
        return hid.sendReport(device, id, data)
    }

    @Synchronized
    fun stop() {
        wanted = false
        val hid = proxy ?: return publish(State.UNAVAILABLE)
        if (registered) hid.unregisterApp()
        registered = false
        host = null
        closeProxy()
        publish(State.UNAVAILABLE)
    }

    private fun closeProxy() {
        proxy?.let { adapter?.closeProfileProxy(BluetoothProfile.HID_DEVICE, it) }
        proxy = null
    }

    private fun publish(next: State) {
        state = next
        listeners.forEach { it(next) }
    }

    companion object {
        private const val TAG = "Desklink"
        const val KEYBOARD = 1
        const val POINTER_ABSOLUTE = 2
        const val PEN = 4

        @Volatile private var instance: HidKeyboardMouse? = null

        fun get(context: Context): HidKeyboardMouse =
            instance ?: synchronized(this) {
                instance ?: HidKeyboardMouse(context.applicationContext).also { instance = it }
            }
    }
}
