# Changelog

## 0.2.0 — in development

- Bluetooth keyboard and mouse: the Android app can control a computer that runs no Desklink software and needs no admin rights there. The computer pairs the phone as a standard Bluetooth keyboard and mouse; a paired Desklink computer arranges it as its own screen and drives it through the phone. Absolute pointer positioning, optional Control↔Command swap for Macs, `scripts/android-pair.sh --keyboard`. Choose per computer in the app: "It runs Desklink" or "No Desklink".
- New `desklink-hid` crate: HID report descriptor, evdev→HID key map and a receiving `Desktop` that emits keyboard/pointer reports, shared by the phone today and a direct Linux Bluetooth path later.
- Android receiver app: the unmodified agent runs through JNI. Input goes through an accessibility service (default) or an opt-in ADB helper that runs as the shell user, authenticated with a token. Includes a pointer overlay, rotation-aware geometry, text/PNG clipboard both ways, pairing links and a foreground service.
- `desklink pairing-code` and `desklink pair CODE [--edge]` CLI commands; `scripts/android-pair.sh` and `scripts/android-input-helper.sh`.
- The runtime can be embedded (`runtime::run_with_host`) with a custom desktop, tick interval and shutdown token.
- Settings and profile locks use flock(2) on Android, where std file locking is unsupported.
- Connections made after a display change (rotation, monitor change) advertise the current geometry instead of the startup size.
- Native screen-layout UI with shared fonts and controls for Linux and future macOS validation.
- Drag, resize, snap, validate and save computer arrangements; explicit controller selection and public pairing codes.
- Live layout synchronization and three-computer routing with held-input transfer and disconnected-target recovery.
- Start/Stop sharing, connected-computer status, visible text/image clipboard setting and private local agent control.
- Native X11 and Wayland window backends; precise Wayland capture spans and a fresh permission session when those spans change.
- GNOME 48 clipboard notification compatibility, verified with native GTK text and transparent PNG sharing in both directions.
- Protocol v2; update all paired computers together. Native macOS UI builds and tests are deferred.
- Linux GUI packaging uses a native GNU build; static musl remains available for the headless CLI.

## 0.1.0 — in development

- Modular Rust engine with native X11 and Quartz backends.
- Authenticated, encrypted peer connections and reconnection.
- Screen-edge handoff, reverse return, emergency release and disconnect recovery.
- Bidirectional text and PNG clipboard sharing, limits and loop prevention.
- Discard queued clipboard observations and completed portal reads after a newer remote copy.
- Preserve macOS drag-button identifiers, held input, and Caps Lock state at handoff; establish modifiers before held shortcuts on X11 and macOS.
- Portal/EI backend for supporting Wayland desktops, under validation.
- Isolated two-desktop integration tests and cross-platform build checks.
