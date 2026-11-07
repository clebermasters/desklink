package com.desklink.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.hardware.display.DisplayManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import com.desklink.android.clipboard.ClipboardSync
import com.desklink.android.clipboard.SendClipboardActivity
import com.desklink.android.input.InputRouter
import org.json.JSONObject
import kotlin.concurrent.thread

/**
 * Keeps the Desklink agent alive while sharing is on. The agent (Rust) owns networking and the
 * handoff state machine; this service pumps its actions into [InputRouter] on the main thread.
 */
class SharingService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private lateinit var settings: Settings
    private lateinit var router: InputRouter
    private var pump: Thread? = null
    private var stopper: Thread? = null
    private var wifiLock: WifiManager.WifiLock? = null
    @Volatile private var stopping = false
    private var statusText = "Starting…"

    private val displayListener = object : DisplayManager.DisplayListener {
        override fun onDisplayAdded(displayId: Int) = Unit
        override fun onDisplayRemoved(displayId: Int) = Unit
        override fun onDisplayChanged(displayId: Int) = updateGeometry(force = false)
    }

    private val statusPoll = object : Runnable {
        override fun run() {
            val dir = settings.profileDir
            thread(name = "desklink-status") {
                val text = describe(runCatching { JSONObject(NativeBridge.status(dir)) }.getOrNull())
                main.post {
                    if (text != statusText) {
                        statusText = text
                        notifyStatus()
                    }
                }
            }
            main.postDelayed(this, 2_000)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        settings = Settings(this)
        router = InputRouter(this, settings)
        router.onReceivingChanged = { notifyStatus() }
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                settings.sharingWanted = false
                shutdown()
                return START_NOT_STICKY
            }
            ACTION_GEOMETRY -> if (pump != null) {
                updateGeometry(force = true)
                return START_STICKY
            }
            ACTION_STOP_HELPER -> if (pump != null) {
                router.shellBackend.stopHelper()
                return START_STICKY
            }
        }
        if (!settings.sharingWanted && intent == null) {
            // Restarted by the system after the user had turned sharing off.
            stopSelf()
            return START_NOT_STICKY
        }
        startInForeground()
        if (pump == null) begin()
        return START_STICKY
    }

    private fun startInForeground() {
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun begin() {
        stopping = false
        updateGeometry(force = true)
        getSystemService(DisplayManager::class.java).registerDisplayListener(displayListener, main)
        acquireWifiLock()
        ClipboardSync.watch(this)
        val dir = settings.profileDir
        val previousStop = stopper
        pump = thread(name = "desklink-pump") {
            previousStop?.join()
            try {
                NativeBridge.init(dir, Build.MODEL ?: "Android phone")
                NativeBridge.start(dir)
            } catch (error: IllegalStateException) {
                Log.e(TAG, "sharing could not start: ${error.message}")
                main.post { failed(error.message ?: "Sharing could not start.") }
                return@thread
            }
            running = true
            main.post { statusPoll.run() }
            while (true) {
                val bytes = try {
                    NativeBridge.nextActions(1_000)
                } catch (error: IllegalStateException) {
                    null
                } ?: break
                if (bytes.isEmpty()) continue
                val actions = Action.decode(bytes)
                main.post { actions.forEach(router::perform) }
            }
            running = false
            main.post {
                if (!stopping) {
                    val error = runCatching { JSONObject(NativeBridge.status(dir)).optString("error") }.getOrNull()
                    failed(error?.takeIf { it.isNotEmpty() && it != "null" } ?: "Sharing stopped.")
                }
            }
        }
    }

    private fun failed(message: String) {
        lastError = message
        statusText = message
        shutdown()
    }

    private fun shutdown() {
        stopping = true
        main.removeCallbacks(statusPoll)
        runCatching { getSystemService(DisplayManager::class.java).unregisterDisplayListener(displayListener) }
        router.perform(Action.Hide)
        ClipboardSync.unwatch(this)
        wifiLock?.takeIf { it.isHeld }?.release()
        wifiLock = null
        val previous = pump
        pump = null
        stopper = thread(name = "desklink-stop") {
            NativeBridge.stop()
            previous?.join(4_000)
            main.post {
                // Sharing may have been switched on again meanwhile.
                if (pump == null) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                }
            }
        }
    }

    /** `force` always reports the size: the native side has none until the first report. */
    private fun updateGeometry(force: Boolean) {
        val geometry = ScreenGeometry.current(this, settings.pointerSpeed)
        if (!force && geometry == router.geometry) return
        Log.i(TAG, "screen ${geometry.widthPx}x${geometry.heightPx} px, shared as ${geometry.logicalWidth}x${geometry.logicalHeight}")
        router.geometry = geometry
        try {
            NativeBridge.setGeometry(geometry.logicalWidth, geometry.logicalHeight)
        } catch (error: IllegalStateException) {
            Log.w(TAG, "invalid screen size: ${error.message}")
        }
    }

    private fun acquireWifiLock() {
        val wifi = applicationContext.getSystemService(WifiManager::class.java) ?: return
        @Suppress("DEPRECATION")
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        } else {
            WifiManager.WIFI_MODE_FULL_HIGH_PERF
        }
        // Wi-Fi power saving otherwise adds 100 ms or more to pointer latency on many phones.
        wifiLock = wifi.createWifiLock(mode, "Desklink").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun describe(state: JSONObject?): String {
        if (state == null) return "Starting…"
        if (!state.optBoolean("running")) return state.optString("error").takeIf { it.isNotEmpty() && it != "null" } ?: "Stopped"
        val peers = state.optJSONObject("status")?.optJSONArray("peers") ?: return "Waiting for a computer"
        val connected = (0 until peers.length()).map { peers.getJSONObject(it) }.filter { it.optBoolean("connected") }
        return when {
            connected.isEmpty() -> "Waiting for a paired computer"
            else -> "Connected to " + connected.joinToString { it.optString("name") }
        }
    }

    private fun createChannel() {
        val channel = NotificationChannel(CHANNEL, getString(R.string.channel_name), NotificationManager.IMPORTANCE_LOW)
        channel.description = getString(R.string.channel_description)
        channel.setShowBadge(false)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun notifyStatus() {
        if (stopping) return
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, buildNotification())
    }

    private fun buildNotification(): Notification {
        val immutable = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), immutable)
        val send = PendingIntent.getActivity(
            this, 1,
            Intent(this, SendClipboardActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            immutable,
        )
        val stop = PendingIntent.getService(this, 2, Intent(this, SharingService::class.java).setAction(ACTION_STOP), immutable)
        val text = if (router.receiving) "A computer is controlling this phone" else statusText
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat)
            .setContentTitle("Desklink sharing is on")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .addAction(Notification.Action.Builder(null, "Send clipboard", send).build())
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
    }

    override fun onDestroy() {
        if (!stopping) shutdown()
        router.close()
        super.onDestroy()
    }

    companion object {
        private const val TAG = "Desklink"
        private const val CHANNEL = "sharing"
        private const val NOTIFICATION_ID = 1
        const val ACTION_STOP = "com.desklink.android.STOP"
        const val ACTION_GEOMETRY = "com.desklink.android.GEOMETRY"
        const val ACTION_STOP_HELPER = "com.desklink.android.STOP_HELPER"

        @Volatile var running = false
            private set
        @Volatile var lastError: String? = null

        fun start(context: Context) {
            Settings(context).sharingWanted = true
            lastError = null
            context.startForegroundService(Intent(context, SharingService::class.java))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, SharingService::class.java).setAction(ACTION_STOP))
        }
    }
}
