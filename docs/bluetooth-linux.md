# Linux as a Bluetooth keyboard and mouse

Some computers can't run Desklink. A company-managed Mac is the usual example: macOS asks for admin rights before an app may capture or inject input. A Linux computer running Desklink can still control such a computer without installing anything on it. Linux presents itself to that computer as an **ordinary Bluetooth keyboard and mouse**. Pairing a keyboard needs no admin rights on the Mac.

```
your keyboard and mouse → Linux (desklink run) → Bluetooth → Mac
```

The Mac is one more screen in your layout. Move the pointer past its edge and you are typing and clicking on the Mac, with exact pointer placement, clicks, drags and scrolling. Move back to return. There is no phone and no Wi-Fi hop. The [phone path](android.md#computers-without-desklink-bluetooth-keyboard-and-mouse) does the same job when the Linux computer isn't within Bluetooth range.

There is no shared clipboard: the Mac sees only a keyboard and mouse.

## One-time setup on Linux (admin)

BlueZ's classic `input` plugin owns the keyboard-and-mouse channels (L2CAP PSM 17 and 19). It also disconnects any computer that isn't itself a keyboard or mouse. A script turns that plugin off and installs a small helper:

```bash
cargo build --release -p desklink-bluez            # builds the desklink-bt-qos helper
sudo scripts/linux-bluetooth-hid.sh enable
scripts/linux-bluetooth-hid.sh status
```

What it changes:

- **A systemd drop-in.** It adds `/etc/systemd/system/bluetooth.service.d/desklink-hid.conf`, which starts `bluetoothd -P input`. Bluetooth restarts once, so audio and other Bluetooth connections drop briefly.
  - Bluetooth LE keyboards and mice (BlueZ's separate `hog` plugin), audio and file transfer keep working.
  - Only older **classic** Bluetooth keyboards and mice stop working on this Linux computer while the plugin is off.
- **The `desklink-bt-qos` helper.** It goes to `/usr/local/libexec/desklink/` with exactly one capability, `CAP_NET_RAW`.
  - Its only job is to ask the Bluetooth link for low latency when the Mac connects. That request is an HCI command, which is why it needs the capability.
  - Without it, the Mac polls the keyboard only every 25–70 ms and the pointer moves in visible clumps. See the [measurements](#smoothness).

Desklink itself runs as your user with no privileges. bluetoothd accepts the channels and passes them to Desklink over D-Bus (`org.bluez.Profile1`).

`sudo scripts/linux-bluetooth-hid.sh disable` undoes both changes.

## Add, pair and run

```bash
desklink bluetooth setup --name "Work MacBook" --size 1512x982 --edge left
desklink bluetooth pair      # visible for 2 minutes; then keeps running
```

On the Mac, open **System Settings → Bluetooth** and click **Connect** next to this Linux computer. It is listed under its Bluetooth name, for example `pop-os`. No code is needed.

Then restart `desklink run`, or press Start in the app, so the main profile connects to the new screen. From then on:

```bash
desklink run              # captures this computer's keyboard and mouse, as always
desklink bluetooth run    # the keyboard and mouse the Mac sees
```

- `--size` is the Mac's screen in points: the "looks like" resolution in *Displays*, for example 1512×982 on a 14-inch MacBook Pro.
- `--edge` places the Mac in the layout, beside this computer's screen. You can rearrange it later in the layout editor.
- `--replace NAME` gives the new screen the place of an already paired computer, for example the same Mac reached through the phone (`--replace "Computer (Bluetooth)"`). That computer stays paired but leaves the layout. Without a saved layout, peer edges decide.
- Control and Command are swapped by default, so Ctrl+C on Linux is ⌘C on the Mac. `--no-swap` keeps them as they are.
- `--relative` moves the pointer with deltas instead of exact positions, for hosts without absolute pointer support.
- If the Mac sleeps or walks out of range, Desklink reconnects by itself. It retries at once when you move the pointer onto the Mac's screen.
- `desklink bluetooth status` shows the setup, the paired Mac and what's missing.
- `desklink bluetooth remove` deletes the receiver profile. Also remove the computer in the Mac's Bluetooth settings.

#### Start it with your session

`packaging/linux/desklink-bluetooth.service` is a systemd user unit, the companion of `desklink.service`:

```bash
cp packaging/linux/desklink-bluetooth.service ~/.config/systemd/user/
systemctl --user enable --now desklink-bluetooth.service
```

### How it fits together

`desklink bluetooth setup` creates a second profile in `~/.config/desklink/bluetooth/`.

- That profile has its own identity and listens on **127.0.0.1:24881 only**.
- It trusts only your main profile.
- Your main profile trusts it at `127.0.0.1:24881`.

The layout, edge crossing, hotkeys and emergency release therefore work as with any other computer. The receiver runs the same `desklink-hid` code as the phone path and hands its reports to BlueZ (`crates/bluez`).

If the same Mac is already paired through the phone, `setup` refuses to put a second copy at the same edge. Remove one first (`desklink untrust KEY`) or choose another edge.

## How the Mac sees it

The descriptor is the one the phone uses:

- a keyboard;
- an absolute pointer;
- a relative pointer;
- a pen.

macOS quirks, found by testing, and how Desklink handles them:

| macOS behaviour | What Desklink does |
|---|---|
| Ignores buttons on an absolute mouse and reads every absolute report as "all buttons up". | Clicks travel in relative-pointer reports with no motion. |
| Scrolls only with the absolute pointer's wheel. | The wheel rides in absolute reports. |
| Accepts an absolute **pen** with a pressed tip as a left-button drag. | Left drags are pen strokes, exact to the pixel, and the pointer stays where you let go. |
| — | Right and middle drags move relatively. |

## Smoothness

These figures come from a steady sweep on a MacBook, measuring the gaps between pointer updates on the Mac. "Before" is a link without the low-latency request; "after" has it.

| Report rate | Median gap, before → after | Gaps over 20 ms, before → after |
|---|---|---|
| every 8 ms | 1.3 ms in clumps, p90 31.6 ms → **8.9 ms**, p90 11.4 ms | 63 → **4** |
| every 4 ms | p90 31.5 ms → **5.8 ms**, p90 6.5 ms | 68 → **3** |

Without the request, the Mac (the link's central) polls rarely, so updates arrive in bursts. With it, the Mac holds the link at about 2.5 ms latency and the link stays awake. Linux follows the main agent's 4 ms tick, about 250 updates a second. The receiver keeps only the newest pointer position while the link is busy, so delay never builds up.

## Troubleshooting

- **Hyprland: RemoteDesktop portal was not found.** Use `desklink run --no-clipboard`, or set `clipboard=false` in the main profile for the app. With InputCapture available, Linux can send input to the Bluetooth Mac without RemoteDesktop. Linux cannot receive input in this mode; returning restores its pointer near the entry edge.

- **"bluetoothd's input plugin owns the keyboard channels."** Run `sudo scripts/linux-bluetooth-hid.sh enable`.
- **Pointer motion looks choppy.** `scripts/linux-bluetooth-hid.sh status` should show the helper with `cap_net_raw=ep`. With `RUST_LOG=desklink_bluez=debug`, the log reads `asked the host for low latency` on every connection.
- **The Mac paired this computer earlier, before setup.** It cached a record without the keyboard service. Forget the computer on the Mac and pair again.
- **Pairing finds nothing.** The computer is visible only while `desklink bluetooth pair` runs (2 minutes).
- **Bluetooth was restarted** (for example by `linux-bluetooth-hid.sh enable`): `desklink bluetooth run` notices bluetoothd come back, registers the keyboard again and reconnects to the Mac. The log reads `the keyboard is registered again`. If it reads `cannot register the keyboard with the restarted bluetoothd` instead, restart `desklink bluetooth run`. When that follows `linux-bluetooth-hid.sh disable`, it is expected. Checked live: the Mac was back about 6 seconds after `systemctl restart bluetooth`.

## Option 2: a wired USB keyboard-and-mouse adapter (not built)

For the feel of a wired mouse, plug a small USB keyboard-and-mouse adapter into the Mac and drive it from Linux. **Desklink does not support this yet;** this section records the design.

```
Linux → USB serial → adapter (e.g. CH9329 board or Raspberry Pi Pico) → USB HID → Mac
```

- **Hardware.** A CH9329 serial-to-HID board costs about $5–10, and a Raspberry Pi Pico running TinyUSB firmware about $5. Both present themselves to the Mac as a standard USB keyboard and mouse, so the Mac needs no software and no admin rights.
- **What to expect** (expected, not measured):
  - USB HID is polled up to 1000 times a second with no radio, so motion should be as smooth as a local mouse;
  - no pairing, and it works while Bluetooth is off.
  - Whether macOS's absolute-pointer button quirk also applies over USB hasn't been tested. The same descriptor (relative buttons, pen drags) would cover it either way.
- **Fit with Desklink.**
  - `desklink-hid` already produces the reports. A serial `ReportSink` would frame them for the adapter: CH9329's command protocol, or a tiny framing of our own for Pico firmware.
  - The receiver profile, layout and edge crossing would stay exactly as for Bluetooth.
- **Before plugging anything into a managed Mac, check your company's USB policy with IT.** Some managed Macs block or log unknown USB devices, even keyboards.
