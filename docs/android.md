# Android

The Desklink app makes an Android phone or tablet one more screen in your arrangement. Move your computer's pointer across the shared edge and it appears on the phone. Click, drag, scroll and type there, then move back to return. Text and image clipboard copies travel in both directions.

The phone is a **receiver only**. Its touchscreen does not control the computer.

## How it works

```
 computer (desklink run)            phone
 ───────────────────────            ─────────────────────────────────────────────
 Engine ── Noise XX/TCP ─────────▶  Rust agent (crates/android, unmodified runtime)
                                      │  AndroidDesktop: Warp / Inject / clipboard
                                      ▼  bounded action queue (JNI)
                                    SharingService (foreground)
                                      ▼
                                    InputRouter ──▶ Accessibility backend (default)
                                      │         └─▶ ADB helper (developer mode)
                                      └─▶ pointer overlay, clipboard
```

The phone runs the same agent, protocol, trust model and handoff state machine as the desktop. `crates/android` supplies an Android `Desktop`. The receiving engine already tracks the pointer and emits absolute positions, so the app only performs actions. The app advertises its screen in density-independent pixels, so a computer's mouse moves at a comfortable speed. Rotating the phone updates the geometry.

## Requirements

- Android 8.0 (API 26) or newer. Android 13+ is recommended for keyboard input in accessibility mode.
- Phone and computer on the same network. The phone listens on TCP 24880.
- A computer running this Desklink version (protocol v2).

## Build and install

You need Rust 1.95+, JDK 17, the Android SDK (platform 35, build-tools 36) with NDK 29.0.14206865, and `cargo install cargo-ndk`.

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cd android
echo "sdk.dir=$HOME/Android/Sdk" > local.properties
./gradlew assembleDebug          # Gradle builds the Rust library through cargo-ndk
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

To build other ABIs, use `-Pdesklink.abis=arm64-v8a,armeabi-v7a,x86_64`. `assembleRelease` builds a minified APK. It is signed with the debug key until you configure your own signing in `android/app/build.gradle.kts`.

## Set up

1. **Pair both ways.** Every connection requires both devices to trust each other's public key. There is no trust-on-first-use.
   - **With adb:** run `scripts/android-pair.sh --edge right`. It trusts the computer on the phone and the phone on the computer. `--edge` says where the phone sits.
   - **Without adb:**
     1. On the computer, run `desklink pairing-code`, or use *Copy pairing code* in the editor.
     2. Get that code onto the phone and open it. Opening a `desklink://pair/…` link starts the app, or you can paste it under *Computers*.
     3. Confirm the computer's name and key.
     4. In the app, use *This phone → Share* and give that code to the computer: `desklink pair CODE --edge right`, or paste it in the editor.
   - Leave *Let this computer arrange the screens* checked. The phone then follows that computer's layout, including multi-computer traversal.
2. **Choose an input mode** (below).
3. **Start sharing** on the phone. Then start or restart sharing on the computer, or place the phone in the layout editor and save.

### Input modes

| | Accessibility (default) | ADB developer mode (opt-in) |
| --- | --- | --- |
| Setup | Settings → Accessibility → Desklink → On | USB or wireless debugging, then `scripts/android-input-helper.sh` from the computer after every phone restart |
| Privilege | Accessibility permission approved by the user | Runs as the adb `shell` user (not root) |
| Pointer | Drawn by Desklink | Drawn by Desklink (grant *Display over other apps*) |
| Left button | Tap, drag and long press with a held touch | Real mouse events, including hover |
| Right button | Long press | Secondary click |
| Wheel | Short swipes under the pointer | Real wheel events |
| Keyboard | Android 13+: key events to the focused field; Android 8–12: text-field editing only | Real key events everywhere, including shortcuts |
| Back button / Escape | Android Back | Back key / Escape |

Accessibility mode is the recommended default. It needs no developer settings and no elevated access. ADB developer mode gives the most faithful mouse and keyboard. It relies on developer options and should only be used on a phone you control. If the selected mode is unavailable, the app falls back to the other mode when that one is available.

### Clipboard

Copies made on a computer arrive on the phone automatically: text, and PNG images through a private content provider.

Android lets an app read the clipboard only while it is in front, so phone→computer works this way:

- Copies made in the Desklink app are sent at once, for example Ctrl+C typed from the computer into the *Try it* field.
- Elsewhere, copy, then tap *Send clipboard* in the sharing notification or open the app. Both send the latest copy.

Turn clipboard sharing off under *Options*.

## Security model

- **Same protocol as the desktop.**
  - Noise XX with explicit public-key trust on both sides.
  - The phone's identity lives in app-private storage and is excluded from backups and device transfer.
  - Pairing links always show the computer's name, address and key and require confirmation.
- **Shell-only provider.** `content://com.desklink.android.shell` serves pairing and helper registration. It accepts calls only from the adb shell user or root. Holding adb authorization already implies full control of the device.
- **The ADB helper:**
  - Listens on `127.0.0.1` on a random port.
  - Requires a random 32-byte token before accepting any command.
  - Is announced only to the app through the shell-only provider, so other apps can neither learn the token nor pose as the helper.
  - Releases all held keys and buttons when the app disconnects.
- **Accessibility service.**
  - On Android 13+ it does not read window content.
  - On Android 8–12 it reads only the focused text field, to insert typed text.
  - It does not observe accessibility events.
- **Logging.** The app never logs key codes, typed text or clipboard contents.

## Troubleshooting

- **The computer cannot connect.**
  - Both devices must be paired both ways and on the same network.
  - Some guest or office networks isolate clients.
  - The app shows the address computers should use.
- **Pointer appears but taps do nothing.** Enable the accessibility service, or start the ADB helper and select that mode. The router logs the backend for each session: `adb logcat -s Desklink`.
- **High latency.** The app holds a low-latency Wi-Fi lock while sharing. Battery savers or a weak signal can still add delay.
- **Return to the computer.** Move back across the edge you entered from, or the layout seam. Ctrl+Alt+Shift+Escape on the computer always restores local control.

## Testing

- `cargo test -p desklink-android` covers the action queue, key map, desktop translation and profile handling. It also runs the real agent in-process against a headless controller over loopback: input, clipboard both ways, return with held-key release, stop and restart.
- `tests/android_e2e.py` drives an emulator or test phone through `adb forward` using `crates/android/examples/controller.rs`. It never captures this computer's input. It covers:
  - shell pairing
  - accessibility click, typing and Ctrl+A/Ctrl+V
  - wheel scrolling
  - clipboard both ways
  - the ADB helper (shell user, authentication, typing, wrong-token rejection)
- The test changes accessibility settings and the app's pairing. Use a test device or emulator.

## Known limits

- The phone receives input but does not share its touchscreen.
- In accessibility mode, typing on Android 8–12 edits the focused field directly. Shortcuts there are limited to select all, copy, cut and paste.
- Phone→computer clipboard needs the app in front, or the notification action, because Android 10+ blocks background clipboard reads.
- Desklink does not unlock the phone or wake its screen. Behavior while the phone is locked or asleep has not been tested.
- The ADB helper must be started again after the phone restarts.
