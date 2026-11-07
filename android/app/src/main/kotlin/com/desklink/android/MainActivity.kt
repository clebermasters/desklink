package com.desklink.android

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings as AndroidSettings
import android.view.Gravity
import android.view.View
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioGroup
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import com.desklink.android.bluetooth.BluetoothCard
import com.desklink.android.clipboard.ClipboardSync
import com.desklink.android.input.DesklinkAccessibilityService
import com.desklink.android.input.ShellBackend
import com.desklink.android.shell.ShellHelperChannel
import org.json.JSONObject
import java.util.concurrent.Executors

class MainActivity : Activity() {
    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var settings: Settings
    private var info: JSONObject? = null
    private var connectedKeys: Set<String> = emptySet()
    private var resumed = false
    private lateinit var bluetooth: BluetoothCard

    private val status: TextView by lazy { findViewById(R.id.status) }
    private val toggle: Button by lazy { findViewById(R.id.toggle) }
    private val peers: LinearLayout by lazy { findViewById(R.id.peers) }
    private val nameField: EditText by lazy { findViewById(R.id.name) }
    private val clipboardSwitch: Switch by lazy { findViewById(R.id.clipboard) }

    private val profileChanged: () -> Unit = { main.post { profileUpdated(restart = true) } }
    private val poll = object : Runnable {
        override fun run() {
            refreshStatus()
            if (resumed) main.postDelayed(this, 1_000)
        }
    }
    private val accessibilityChanged: () -> Unit = { main.post { refreshInputState() } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        settings = Settings(this)
        bindControls()
        bluetooth = BluetoothCard(this, settings)
        background({ NativeBridge.init(settings.profileDir, Build.MODEL ?: "Android phone") }) { refreshProfile() }
        handlePairingIntent(intent)
        ProfileEvents.add(profileChanged)
        DesklinkAccessibilityService.listeners += accessibilityChanged
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePairingIntent(intent)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        refreshInputState()
        main.post(poll)
    }

    override fun onPause() {
        resumed = false
        main.removeCallbacks(poll)
        super.onPause()
    }

    override fun onDestroy() {
        ProfileEvents.remove(profileChanged)
        DesklinkAccessibilityService.listeners -= accessibilityChanged
        worker.shutdown()
        super.onDestroy()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // Android lets the app read the clipboard only now, so forward a fresh phone copy.
        if (hasFocus && SharingService.running && info?.optBoolean("clipboard") == true) {
            ClipboardSync.sendCurrent(this)
        }
    }

    /** This phone's profile: name, pairing code and trusted computers. */
    internal fun phoneInfo(): JSONObject? = info

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQUEST_BLUETOOTH) bluetooth.permissionsChanged()
    }

    private fun bindControls() {
        toggle.setOnClickListener {
            if (SharingService.running) {
                settings.sharingWanted = false
                SharingService.stop(this)
            } else {
                requestNotificationPermission()
                SharingService.start(this)
            }
            main.postDelayed({ refreshStatus() }, 300)
        }

        val mode = findViewById<RadioGroup>(R.id.mode)
        mode.check(if (settings.inputMode == InputMode.SHELL) R.id.mode_shell else R.id.mode_accessibility)
        mode.setOnCheckedChangeListener { _, id ->
            settings.inputMode = if (id == R.id.mode_shell) InputMode.SHELL else InputMode.ACCESSIBILITY
            refreshInputState()
        }
        findViewById<Button>(R.id.open_accessibility).setOnClickListener {
            startActivity(Intent(AndroidSettings.ACTION_ACCESSIBILITY_SETTINGS))
        }
        findViewById<TextView>(R.id.shell_command).text = ShellBackend.launchCommand(this)
        findViewById<Button>(R.id.copy_command).setOnClickListener {
            copy("Desklink helper command", ShellBackend.launchCommand(this))
        }
        findViewById<Button>(R.id.stop_helper).setOnClickListener {
            if (SharingService.running) {
                startService(Intent(this, SharingService::class.java).setAction(SharingService.ACTION_STOP_HELPER))
            } else {
                ShellBackend(this).apply { stopHelper(); close() }
            }
            toast("Asked the helper to stop")
        }
        findViewById<Button>(R.id.overlay_permission).setOnClickListener {
            startActivity(
                Intent(AndroidSettings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:$packageName")),
            )
        }

        findViewById<Button>(R.id.pair).setOnClickListener {
            confirmPairing(findViewById<EditText>(R.id.pair_code).text.toString())
        }
        findViewById<Button>(R.id.copy_code).setOnClickListener {
            info?.optString("pairing_code")?.let { copy("Desklink pairing code", it) }
        }
        findViewById<Button>(R.id.share_code).setOnClickListener {
            val code = info?.optString("pairing_code") ?: return@setOnClickListener
            startActivity(
                Intent.createChooser(
                    Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, code),
                    "Send pairing code",
                ),
            )
        }

        nameField.setOnEditorActionListener { view, action, _ ->
            if (action == EditorInfo.IME_ACTION_DONE) view.clearFocus()
            false
        }
        nameField.setOnFocusChangeListener { _, focused ->
            val name = nameField.text.toString().trim()
            if (!focused && name.isNotEmpty() && name != info?.optString("name")) {
                background({ NativeBridge.setName(settings.profileDir, name) }) { profileUpdated(restart = true) }
            }
        }
        clipboardSwitch.setOnCheckedChangeListener { button, checked ->
            if (!button.isPressed) return@setOnCheckedChangeListener
            background({ NativeBridge.setClipboard(settings.profileDir, checked) }) { profileUpdated(restart = true) }
        }
        findViewById<Button>(R.id.send_clipboard).setOnClickListener {
            when {
                !SharingService.running -> toast("Start sharing first")
                ClipboardSync.sendCurrent(this) -> toast("Clipboard sent")
                else -> toast("The clipboard is empty")
            }
        }

        val speed = findViewById<RadioGroup>(R.id.speed)
        speed.check(
            when {
                settings.pointerSpeed < 0.9f -> R.id.speed_slow
                settings.pointerSpeed > 1.2f -> R.id.speed_fast
                else -> R.id.speed_normal
            },
        )
        speed.setOnCheckedChangeListener { _, id ->
            settings.pointerSpeed = when (id) {
                R.id.speed_slow -> 0.6f
                R.id.speed_fast -> 1.6f
                else -> 1.0f
            }
            if (SharingService.running) {
                startService(Intent(this, SharingService::class.java).setAction(SharingService.ACTION_GEOMETRY))
            }
        }
    }

    private fun handlePairingIntent(intent: Intent?) {
        val code = when (intent?.action) {
            Intent.ACTION_VIEW -> intent.dataString
            Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)
                ?.let { Regex("desklink://pair/[A-Za-z0-9_-]+").find(it)?.value }
            else -> null
        } ?: return
        setIntent(Intent(this, MainActivity::class.java))
        confirmPairing(code)
    }

    /** A link or pasted code is only trusted after the user confirms who it belongs to. */
    private fun confirmPairing(code: String) {
        val trimmed = code.trim()
        if (trimmed.isEmpty()) return toast("Paste a pairing code first")
        val controller = findViewById<CheckBox>(R.id.pair_controller).isChecked
        background({ JSONObject(NativeBridge.preview(trimmed)) }) { preview ->
            val key = preview.getString("public_key")
            AlertDialog.Builder(this)
                .setTitle("Pair with ${preview.getString("name")}?")
                .setMessage(
                    "Address: ${preview.getString("address")}\n" +
                        "Key: ${key.take(16).chunked(4).joinToString(" ")}…\n\n" +
                        "Only pair with computers you control. A paired computer can use this phone's " +
                        "pointer, keyboard and clipboard while sharing is on.",
                )
                .setPositiveButton("Pair") { _, _ ->
                    background({ NativeBridge.pair(settings.profileDir, trimmed, controller) }) { name ->
                        findViewById<EditText>(R.id.pair_code).text.clear()
                        toast("Paired with $name. Pair the phone on the computer too.")
                        profileUpdated(restart = true)
                    }
                }
                .setNegativeButton("Cancel", null)
                .show()
        }
    }

    private fun profileUpdated(restart: Boolean) {
        refreshProfile()
        if (restart && (SharingService.running || settings.sharingWanted)) {
            SharingService.stop(this)
            main.postDelayed({ SharingService.start(this) }, 300)
        }
    }

    private fun refreshProfile() {
        val dir = settings.profileDir
        background({ JSONObject(NativeBridge.info(dir, Network.wifiAddress(this))) }) { loaded ->
            info = loaded
            if (!nameField.hasFocus()) nameField.setText(loaded.getString("name"))
            clipboardSwitch.isChecked = loaded.getBoolean("clipboard")
            findViewById<TextView>(R.id.own_code).text = loaded.getString("pairing_code")
            findViewById<TextView>(R.id.own_address).text =
                "Reachable at ${loaded.getString("address")} on this network."
            renderPeers()
            bluetooth.update(loaded)
        }
    }

    private fun renderPeers() {
        peers.removeAllViews()
        val list = info?.optJSONArray("peers") ?: return
        if (list.length() == 0) {
            peers.addView(TextView(this).apply {
                text = "No computers paired yet."
                setTextColor(getColor(R.color.muted))
            })
            return
        }
        for (i in 0 until list.length()) {
            val peer = list.getJSONObject(i)
            val key = peer.getString("public_key")
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                setPadding(0, 8, 0, 8)
            }
            val label = TextView(this).apply {
                val connected = key in connectedKeys
                val role = if (peer.optBoolean("controller")) " · arranges screens" else ""
                text = "${if (connected) "●" else "○"} ${peer.getString("name")}$role\n${peer.optString("address", "")}"
                setTextColor(getColor(if (connected) R.color.good else R.color.text))
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            }
            val remove = Button(this).apply {
                text = "Remove"
                setOnClickListener {
                    AlertDialog.Builder(this@MainActivity)
                        .setTitle("Remove ${peer.getString("name")}?")
                        .setMessage("It will no longer be able to control this phone.")
                        .setPositiveButton("Remove") { _, _ ->
                            background({ NativeBridge.unpair(settings.profileDir, key) }) { profileUpdated(restart = true) }
                        }
                        .setNegativeButton("Cancel", null)
                        .show()
                }
            }
            row.addView(label)
            row.addView(remove)
            peers.addView(row)
        }
    }

    private fun refreshStatus() {
        bluetooth.refreshStatus()
        val dir = settings.profileDir
        background({ JSONObject(NativeBridge.status(dir)) }) { state ->
            val running = state.optBoolean("running")
            toggle.text = if (running) "Stop sharing" else "Start sharing"
            val peerStates = state.optJSONObject("status")?.optJSONArray("peers")
            val connected = mutableSetOf<String>()
            val names = mutableListOf<String>()
            if (peerStates != null) {
                for (i in 0 until peerStates.length()) {
                    val peer = peerStates.getJSONObject(i)
                    if (peer.optBoolean("connected")) {
                        connected += peer.getString("public_key")
                        names += peer.getString("name")
                    }
                }
            }
            val error = SharingService.lastError ?: state.optString("error").takeIf { it.isNotEmpty() && it != "null" }
            status.text = when {
                running && names.isNotEmpty() -> "On · connected to ${names.joinToString()}"
                running -> "On · waiting for a paired computer on ${info?.optString("address") ?: "this network"}"
                error != null -> "Off · $error"
                else -> "Off"
            }
            status.setTextColor(getColor(if (running) R.color.good else if (error != null) R.color.warn else R.color.muted))
            if (connected != connectedKeys) {
                connectedKeys = connected
                renderPeers()
            }
        }
    }

    private fun refreshInputState() {
        val accessibilityOn = DesklinkAccessibilityService.instance != null
        findViewById<TextView>(R.id.accessibility_state).text =
            if (accessibilityOn) "Accessibility control is on."
            else "Accessibility control is off. Open the settings, choose Desklink, and turn it on."
        findViewById<Button>(R.id.open_accessibility).visibility = if (accessibilityOn) View.GONE else View.VISIBLE
        val shellMode = settings.inputMode == InputMode.SHELL
        findViewById<View>(R.id.shell_section).visibility = if (shellMode) View.VISIBLE else View.GONE
        if (shellMode) {
            val endpoint = ShellHelperChannel.endpoint(this)
            findViewById<TextView>(R.id.shell_state).text =
                (if (endpoint == null) "The helper has not started yet. " else "The helper has registered. ") +
                    "Enable USB or wireless debugging, connect with adb from your computer, then run " +
                    "scripts/android-input-helper.sh or this command:"
            findViewById<Button>(R.id.overlay_permission).visibility =
                if (AndroidSettings.canDrawOverlays(this) || accessibilityOn) View.GONE else View.VISIBLE
        }
    }

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    internal fun copy(label: String, text: String) {
        getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText(label, text))
        toast("Copied")
    }

    internal fun toast(message: String) = Toast.makeText(this, message, Toast.LENGTH_LONG).show()

    /** Runs native calls off the main thread; errors are shown to the user. */
    internal fun <T> background(work: () -> T, done: (T) -> Unit) {
        worker.execute {
            val result = try {
                Result.success(work())
            } catch (error: Exception) {
                Result.failure(error)
            }
            main.post {
                if (isDestroyed) return@post
                result.onSuccess(done).onFailure { toast(it.message ?: "Something went wrong") }
            }
        }
    }

    companion object {
        const val REQUEST_BLUETOOTH = 2
    }
}
