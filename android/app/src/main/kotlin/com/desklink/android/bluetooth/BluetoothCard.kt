package com.desklink.android.bluetooth

import android.Manifest
import android.app.AlertDialog
import android.bluetooth.BluetoothAdapter
import android.content.Intent
import android.os.Build
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioGroup
import android.widget.Spinner
import android.widget.TextView
import com.desklink.android.MainActivity
import com.desklink.android.NativeBridge
import com.desklink.android.Network
import com.desklink.android.R
import com.desklink.android.Settings
import com.desklink.android.SharingService
import org.json.JSONObject

/**
 * The "No Desklink" approach in the Computers card: set up a computer that this phone controls
 * as a standard Bluetooth keyboard and mouse, and show it in the computer list.
 */
class BluetoothCard(private val activity: MainActivity, private val settings: Settings) {
    private val keyboard = KeyboardSettings(activity)
    private val hid = HidKeyboardMouse.get(activity)
    private val row: LinearLayout = activity.findViewById(R.id.keyboard_computer)
    private val setup: LinearLayout = activity.findViewById(R.id.bluetooth_setup)
    private val desklinkPairing: View = activity.findViewById(R.id.desklink_pairing)
    private val approach: RadioGroup = activity.findViewById(R.id.approach)
    private var info: JSONObject? = null
    private var controllers: List<Pair<String, String>> = emptyList() // (public key, name)
    private var statusLine: TextView? = null
    private var connected = false

    init {
        approach.setOnCheckedChangeListener { _, id -> showSetup(id == R.id.approach_bluetooth) }
    }

    /** `phoneInfo` lists the computers this phone trusts; one of them drives the keyboard. */
    fun update(phoneInfo: JSONObject?) {
        val peers = phoneInfo?.optJSONArray("peers")
        controllers = (0 until (peers?.length() ?: 0)).map { peers!!.getJSONObject(it) }
            .map { it.getString("public_key") to it.getString("name") }
        if (keyboard.enabled) {
            activity.background({
                NativeBridge.keyboardEnsure(keyboard.profileDir, KeyboardSettings.DEFAULT_NAME)
                JSONObject(NativeBridge.info(keyboard.profileDir, Network.wifiAddress(activity)))
            }) { loaded ->
                info = loaded
                renderRow()
                if (setup.visibility == View.VISIBLE) renderSetup()
            }
        } else {
            info = null
            renderRow()
            if (setup.visibility == View.VISIBLE) renderSetup()
        }
    }

    /** Called every second while the app is open. */
    fun refreshStatus() {
        if (!keyboard.enabled) return
        activity.background({ JSONObject(NativeBridge.keyboardStatus(keyboard.profileDir)) }) { state ->
            val peers = state.optJSONObject("status")?.optJSONArray("peers")
            connected = (0 until (peers?.length() ?: 0)).any { peers!!.getJSONObject(it).optBoolean("connected") }
            renderRow()
            statusLine?.text = describe()
        }
    }

    fun permissionsChanged() {
        if (setup.visibility == View.VISIBLE) renderSetup()
    }

    private fun showSetup(visible: Boolean) {
        desklinkPairing.visibility = if (visible) View.GONE else View.VISIBLE
        setup.visibility = if (visible) View.VISIBLE else View.GONE
        if (visible) renderSetup()
    }

    private fun describe(): String {
        val computer = keyboard.deviceAddress?.let { hid.nameOf(it) ?: it }
        val bluetooth = when (hid.state) {
            HidKeyboardMouse.State.CONNECTED -> "Bluetooth: connected to ${hid.host?.let { hid.nameOf(it.address) } ?: computer ?: "the computer"}"
            HidKeyboardMouse.State.CONNECTING -> "Bluetooth: connecting to ${computer ?: "the computer"}…"
            HidKeyboardMouse.State.READY -> if (computer == null) "Bluetooth: waiting for the computer to pair" else "Bluetooth: $computer is not connected"
            HidKeyboardMouse.State.STARTING -> "Bluetooth: starting…"
            HidKeyboardMouse.State.NO_PERMISSION -> "Bluetooth: allow Nearby devices for Desklink"
            HidKeyboardMouse.State.UNAVAILABLE -> if (SharingService.running) "Bluetooth: unavailable" else "Bluetooth: starts with sharing"
        }
        val controller = controllers.firstOrNull { it.first == controllerKey() }?.second ?: "the controlling computer"
        val desklink = if (connected) "Desklink: $controller connected" else "Desklink: waiting for $controller"
        return "$bluetooth\n$desklink"
    }

    private fun controllerKey(): String? = info?.optJSONArray("peers")?.optJSONObject(0)?.optString("public_key")

    private fun renderRow() {
        row.removeAllViews()
        val loaded = info
        if (!keyboard.enabled || loaded == null) return
        val line = LinearLayout(activity).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(0, 8, 0, 8)
        }
        val live = connected && hid.state == HidKeyboardMouse.State.CONNECTED
        line.addView(TextView(activity).apply {
            text = "${if (live) "●" else "○"} ${loaded.getString("name")} · Bluetooth keyboard and mouse\n" +
                (keyboard.deviceAddress?.let { hid.nameOf(it) } ?: "Not connected over Bluetooth yet")
            setTextColor(activity.getColor(if (live) R.color.good else R.color.text))
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        })
        line.addView(Button(activity).apply {
            text = "Edit"
            setOnClickListener { approach.check(R.id.approach_bluetooth) }
        })
        row.addView(line)
    }

    private fun renderSetup() {
        setup.removeAllViews()
        fun body(text: String) = TextView(activity).apply {
            this.text = text
            setTextColor(activity.getColor(R.color.muted))
            setPadding(0, 10, 0, 0)
        }
        setup.addView(body(
            "The computer needs no Desklink software or admin rights: it sees this phone as a standard " +
                "Bluetooth keyboard and mouse. The computer you choose below controls it through the phone. " +
                "Clipboard sharing is not available this way. On a managed work computer, make sure your " +
                "IT policy allows Bluetooth accessories.",
        ))
        if (!hid.supported) {
            setup.addView(body("This needs Android 9 or newer with Bluetooth."))
            return
        }
        if (!hid.permission()) {
            setup.addView(Button(activity).apply {
                text = "Allow Bluetooth (Nearby devices)"
                setOnClickListener { requestPermissions() }
            })
            return
        }
        if (controllers.isEmpty()) {
            setup.addView(body("First pair a computer that runs Desklink. It will control the Bluetooth computer."))
            return
        }

        setup.addView(body("Controlled by"))
        val controller = Spinner(activity).apply {
            adapter = ArrayAdapter(activity, android.R.layout.simple_spinner_dropdown_item, controllers.map { it.second })
            val current = controllers.indexOfFirst { it.first == controllerKey() }
            if (current >= 0) setSelection(current)
        }
        setup.addView(controller)
        // A computer that already paired with the phone can be chosen; otherwise the first
        // computer that pairs and connects is remembered.
        val bonded = hid.bondedComputers()
        val computers = listOf<Pair<String?, String>>(null to "The computer that pairs next") +
            bonded.map { (address, name) -> address to name }
        setup.addView(body("Bluetooth computer"))
        val computer = Spinner(activity).apply {
            adapter = ArrayAdapter(activity, android.R.layout.simple_spinner_dropdown_item, computers.map { it.second })
            setSelection(computers.indexOfFirst { it.first == keyboard.deviceAddress }.coerceAtLeast(0))
        }
        setup.addView(computer)
        val name = EditText(activity).apply {
            hint = "Computer name, e.g. Work MacBook"
            setText(info?.optString("name")?.takeIf { it != KeyboardSettings.DEFAULT_NAME } ?: "")
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_WORDS
        }
        setup.addView(name)
        setup.addView(body("Screen size the pointer uses. On a Mac: System Settings → Displays, the \"looks like\" size."))
        val size = LinearLayout(activity).apply { orientation = LinearLayout.HORIZONTAL }
        fun number(value: Int, label: String) = EditText(activity).apply {
            setText(value.toString())
            hint = label
            inputType = InputType.TYPE_CLASS_NUMBER
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        }
        val width = number(keyboard.width, "Width")
        val height = number(keyboard.height, "Height")
        size.addView(width)
        size.addView(TextView(activity).apply { text = " × " })
        size.addView(height)
        setup.addView(size)
        val swap = CheckBox(activity).apply {
            text = "It's a Mac: Control acts as ⌘ (copy, paste and other shortcuts)"
            isChecked = keyboard.swapControlAndCommand
        }
        val relative = CheckBox(activity).apply {
            text = "Relative pointer (only if the pointer does not follow)"
            isChecked = keyboard.relativePointer
        }
        setup.addView(swap)
        setup.addView(relative)
        setup.addView(Button(activity).apply {
            text = if (keyboard.enabled) "Save" else "Save and make this phone visible"
            setOnClickListener {
                val w = width.text.toString().toIntOrNull()
                val h = height.text.toString().toIntOrNull()
                if (w == null || h == null || w !in 200..16384 || h !in 200..16384) {
                    return@setOnClickListener activity.toast("Enter the screen size, e.g. 1512 × 982")
                }
                keyboard.deviceAddress = computers[computer.selectedItemPosition].first
                save(controllers[controller.selectedItemPosition].first, name.text.toString().trim(), w, h, swap.isChecked, relative.isChecked)
            }
        })

        if (keyboard.enabled) {
            statusLine = body(describe()).also(setup::addView)
            setup.addView(Button(activity).apply {
                text = "Make this phone visible to pair"
                setOnClickListener { makeVisible() }
            })
            val code = info?.optString("pairing_code").orEmpty()
            val controllerName = controllers.firstOrNull { it.first == controllerKey() }?.second ?: "the controlling computer"
            setup.addView(body("Give this code to $controllerName so it adds the Bluetooth computer to its layout (desklink pair CODE --edge left, or Add computer in the editor):"))
            setup.addView(TextView(activity).apply {
                text = code
                setTextIsSelectable(true)
                typeface = android.graphics.Typeface.MONOSPACE
                textSize = 11f
                setTextColor(activity.getColor(R.color.text))
                setBackgroundResource(R.drawable.card)
                setPadding(20, 20, 20, 20)
            })
            val buttons = LinearLayout(activity).apply { orientation = LinearLayout.HORIZONTAL }
            fun half(label: String, action: () -> Unit) = Button(activity).apply {
                text = label
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
                setOnClickListener { action() }
            }
            buttons.addView(half("Copy") { activity.copy("Desklink pairing code", code) })
            buttons.addView(half("Share") {
                activity.startActivity(
                    Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, code), "Send pairing code"),
                )
            })
            setup.addView(buttons)
            setup.addView(Button(activity).apply {
                text = "Remove Bluetooth computer"
                setOnClickListener { confirmRemove() }
            })
        } else {
            statusLine = null
        }
    }

    private fun save(controllerKey: String, name: String, width: Int, height: Int, swap: Boolean, relative: Boolean) {
        val first = !keyboard.enabled
        keyboard.width = width
        keyboard.height = height
        keyboard.swapControlAndCommand = swap
        keyboard.relativePointer = relative
        activity.background({
            NativeBridge.keyboardEnsure(keyboard.profileDir, KeyboardSettings.DEFAULT_NAME)
            if (name.isNotEmpty()) NativeBridge.setName(keyboard.profileDir, name)
            NativeBridge.keyboardTrust(settings.profileDir, keyboard.profileDir, controllerKey)
        }) { controller ->
            keyboard.enabled = true
            if (SharingService.running) SharingService.keyboardChanged(activity) else SharingService.start(activity)
            activity.toast("Saved. Give the code below to $controller.")
            update(activity.phoneInfo())
            if (first) activity.window.decorView.postDelayed({ makeVisible() }, 1_500)
        }
    }

    /** Lets the computer find the phone. Pairing itself happens in the computer's Bluetooth settings. */
    private fun makeVisible() {
        if (!SharingService.running) SharingService.start(activity)
        val intent = Intent(BluetoothAdapter.ACTION_REQUEST_DISCOVERABLE)
            .putExtra(BluetoothAdapter.EXTRA_DISCOVERABLE_DURATION, 300)
        runCatching { activity.startActivity(intent) }
        AlertDialog.Builder(activity)
            .setTitle("Pair from the computer")
            .setMessage(
                "On the computer, open its Bluetooth settings and connect to this phone " +
                    "(${hid.phoneName() ?: "this phone"}). Confirm the code on both.\n\n" +
                    "If the computer paired with this phone before, remove it there first, so it " +
                    "learns about the keyboard and mouse. A Mac may then ask you to identify the " +
                    "keyboard: press the keys it asks for from the controlling computer, or close the assistant.",
            )
            .setPositiveButton("OK", null)
            .show()
    }

    private fun confirmRemove() {
        AlertDialog.Builder(activity)
            .setTitle("Remove the Bluetooth computer?")
            .setMessage("The phone stops acting as its keyboard and mouse. Remove the phone from the computer's Bluetooth settings too.")
            .setPositiveButton("Remove") { _, _ ->
                keyboard.clear()
                SharingService.keyboardChanged(activity)
                approach.check(R.id.approach_desklink)
                update(activity.phoneInfo())
            }
            .setNegativeButton("Cancel", null)
            .show()
    }

    private fun requestPermissions() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            activity.requestPermissions(
                arrayOf(Manifest.permission.BLUETOOTH_CONNECT, Manifest.permission.BLUETOOTH_ADVERTISE),
                MainActivity.REQUEST_BLUETOOTH,
            )
        }
    }
}
