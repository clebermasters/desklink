# Implementation status

Desklink 0.2 adds a native screen-layout editor to the existing sharing engine. Linux is the active implementation and test target. macOS GUI builds and native acceptance are explicitly deferred by the user until a Mac is available.

## Delivered

- Nine Rust crates separating layout geometry, validated settings, native GUI, protocol, encrypted transport, sharing state, clipboard, OS input and CLI/agent supervision.
- Draggable/resizable computer cards, edge/corner snapping, overlap and disconnected-layout validation, draft/save/reload behavior and a compact-window details dialog.
- Public pairing codes, explicit controller selection, connected-computer status, Start/Stop sharing and a visible text/image clipboard setting.
- Trusted layout synchronization, partial and split edge seams, A→B→C routing through direct controller connections, held-input transfer and offline-target recovery.
- One executable containing GUI and sharing agent. A private per-profile Unix socket controls the background agent; closing the editor keeps active sharing running.
- Linux GNU GUI and optional static musl CLI packaging, examples, protocol v2 documentation and isolated test fixtures. No remote repository or release publication is configured.

## Verified

- 47 workspace tests pass: layout geometry and validation, controller trust/revision checks, atomic/conflict-aware settings, encrypted sockets, EI wire exchanges, clipboard notification compatibility, limits/convergence and input recovery.
- Native X11 UI tests pass for real drag/snap/save/reload, invalid-save rejection, pairing, Start/Stop, connection status and changes applied to running agents.
- Three real X11 agents pass controller layout sync, A→B→C→B→A handoffs with held Control, live repositioning and an offline destination.
- The original two-X11-desktop suite passes Unicode text and transparent PNG transfers in both directions, concurrent copies, input/scrolling, emergency release, crash cleanup, reconnects and clipboard opt-out.
- The native Wayland editor passes the same rendered-window gestures. The test checks an actual Wayland display handle rather than inferring the backend from environment variables.
- GNOME Shell/Mutter 48.7, xdg-desktop-portal-gnome 48 and a regular UID 1000 in a disposable Debian desktop pass real portal approvals, agent startup, layout saving with a fresh permission session, barrier activation, EI keyboard/pointer/buttons/scroll and edge return.
- Native GTK Wayland ↔ X11 clipboard tests pass Unicode text and transparent PNG images in both directions. The native Wayland UI also copies a pairing code that reaches the other computer's clipboard. The Linux GNU release binary passes the UI/sharing test on X11 and GNOME Wayland.
- Switching controllers through the GUI also passes: the Wayland receiver accepts its selected controller's layout, receives EI keyboard input in a native GTK widget and returns control to the X11 controller at the shared edge.
- Formatting and Clippy pass locally. Hosted CI remains unrun; a workflow definition is not evidence of hosted results.

## Android receiver

Delivered:
- An Android 8+ app (`android/`) that embeds the unmodified agent through JNI (`crates/android`).
- Accessibility input by default, and an opt-in ADB developer-mode helper. The helper runs as the shell user and authenticates the app with a token.
- A pointer overlay, rotation-aware geometry and text/PNG clipboard.
- Confirmed pairing links, a foreground service, and CLI/script pairing.

Verified on an Android 15 (API 35) x86_64 emulator, using the debug APK and the minified release APK:
- `tests/android_e2e.py --x11-agent` passes 18 checks:
  - shell-only pairing
  - accessibility click focus, capitalized typing and Ctrl+A/Ctrl+V
  - wheel scrolling
  - text clipboard in both directions
  - ADB helper: shell user, authenticated connection, typing, wrong-token rejection
  - a real `desklink run` on a private Xvfb display: hands its pointer across its right edge, types with captured X11 keys, and exchanges the X11 clipboard both ways
- The pairing link always asks for confirmation.
- Rotation reaches both connected and newly connected peers.
- `cargo test -p desklink-android` runs the agent in-process against a headless controller.

Verified on a physical Samsung Galaxy Z Fold7 (Android 17, cover screen 1080×2520) over home Wi-Fi, driven by a real `desklink run` on an X11 desktop:
- shell pairing in both directions and connection over the LAN
- edge handoff to the phone and back, with the accessibility backend
- click focus, typing, Ctrl+A/Ctrl+C/Ctrl+V, wheel scrolling
- Unicode text clipboard in both directions

Bluetooth keyboard and mouse, verified with the same Galaxy Z Fold7 and a company-managed MacBook Pro (macOS 26.3, standard user, no Desklink installed): Linux → Wi-Fi → phone → Bluetooth → Mac.
- The Mac pairs the phone from its own Bluetooth settings without an admin password, and lists it as a HID device.
- The Linux controller sees the Mac's 1512×982 screen on its left edge. Crossing places the Mac pointer at the matching edge position, absolute positions land exactly (25%/75% checks), and returning restores Linux control at the matching height.
- Text typed on Linux arrives unchanged in a Mac Terminal. The phone keeps working as its own screen at the same time.

Not yet verified:
- Fold/unfold geometry changes on a foldable.
- Bluetooth: Control↔Command shortcuts (unit-tested only); reconnection after the Mac sleeps; Windows and Linux hosts; the in-app setup flow end to end (the live test used `android-pair.sh --keyboard`).

Bluetooth on macOS, found by testing: macOS ignores buttons on an absolute pointer and reads each absolute report as "all buttons up", and scrolls only with the absolute pointer's wheel. Buttons therefore travel in relative reports, and a drag moves relatively. Left, right, middle and back clicks, scrolling and drags work. **Known issue:** a relative drag is subject to macOS pointer acceleration, so on release the pointer jumps back to the exact layout position. A fix is in progress.
- Layout-editor mode: `LayoutUpdate` sync and `Traverse` through the phone.
- PNG clipboard on Android.
- Drag, long-press/right-click and the Back button.
- ADB-mode hover and right click.
- Android 8–12 legacy typing.
- The notification's *Send clipboard* action.
- Behavior while locked or with the screen off.
- The shell provider's UID rejection, which is reviewed in code only: Android blocks the `content` tool for app UIDs before the call reaches the provider.
- The hosted Android CI job has not run.

## Platform limits and follow-up

- On Wayland, changed capture spans restart sharing and request fresh consent. GNOME 48 cannot retain working EIS capture references through the needed disable/update cycle. Layout edits with unchanged spans apply live. X11 applies position edits live.
- KDE and other compositor/portal versions need independent acceptance. Irregular multi-monitor zones, permission revocation and lock/sleep recovery need broader native coverage. Disabled/paused sessions end sharing with an explicit error so the user can restart safely.
- macOS 0.2 UI compilation, standard-user permission setup, native input/clipboard, signing and bundle testing remain deferred. Earlier headless Apple Silicon/Intel compile checks do not validate the new GUI.
- Legacy edge-only profiles remain supported, but protocol v2 requires updating all paired computers together.

All automated desktop input and clipboard data are synthetic and confined to private Xvfb or GNOME sessions. The installed Synergy application and the user's normal desktop were not modified. Craft Anything evidence stays in the ignored analysis directory; Desklink contains an independent implementation.

The Wayland acceptance image used gnome-shell `48.7-0+deb13u2`, libmutter `48.7-0+deb13u1`, xdg-desktop-portal `1.20.3+ds-1`, xdg-desktop-portal-gnome `48.0-2` and GTK `4.18.6+ds-2`. The GUI window and GTK clipboard client both report native Wayland display types. Nested GNOME is used only for editor gestures; the native headless compositor is used for portal input and clipboard acceptance.
