# Desklink

Share a keyboard, mouse, and clipboard between computers over your local network. Arrange computers on a native desk canvas, then move your pointer across their touching edges. Each computer runs the same executable as its signed-in user. Linux is the current tested target; macOS UI builds and native testing are deferred until a Mac is available.

Desklink is an independent implementation. It does not use Synergy's code, account service, or network protocol.

## Features

- Bidirectional keyboard, pointer, buttons, and scrolling.
- Native screen-layout editor: drag, resize and snap computer cards to match your desk, with overlap/disconnection validation.
- Left, right, top and bottom links, partial edge overlaps, different display sizes and multi-computer traversal.
- Explicit controller selection, pairing codes, live layout synchronization, connection status and Start/Stop sharing.
- **Bidirectional Unicode text and PNG image clipboard sharing**, including transparency. Clipboard sharing is enabled by default.
- Encrypted Noise XX connections with explicit public-key pairing on both computers.
- Automatic reconnection, session identifiers to discard stale input, release of held keys/buttons on disconnect, and an emergency return chord.
- Modular Rust workspace, one executable per OS/CPU target, no privileged service or input driver.

## Platform status

| Platform | Implementation | Verification |
| --- | --- | --- |
| Linux X11 | Native UI, X11/XTEST input and clipboard | Real UI gestures/controls and two/three independent Xvfb desktops |
| Linux Wayland | Native UI, InputCapture/RemoteDesktop/Clipboard portals and EI | GNOME 48 test environment; exact results and remaining limits in [status](docs/STATUS.md) |
| macOS | Shared UI/layout code, Quartz input and native pasteboard backend | Earlier headless compile checks only; the 0.2 GUI is not built or tested on macOS |

macOS requires operating-system approval for input access. Some initial permission grants or device policies may require an administrator. Wayland requires the desktop's input portals and approval. Normal sharing runs in the signed-in user's session. Login screens and FileVault preboot are outside the supported session scope.

## Build

Install Rust 1.95 or newer, then:

```sh
cargo build --release --locked
./target/release/desklink doctor
./target/release/desklink
```

The native GUI uses system OpenGL/EGL graphics support and the desktop's X11 or Wayland session. It does not bundle Electron, Qt, GTK or WebKit. Build tools and window-system development headers are not needed to run the archive. GNU/Linux GUI binaries depend on the build system's glibc version; the optional headless musl build avoids that dependency:

```sh
cargo build -p desklink --release --locked --no-default-features --target x86_64-unknown-linux-musl
```

`scripts/package-linux.sh` packages the native GUI by default. Set `DESKLINK_HEADLESS=1 DESKLINK_TARGET=x86_64-unknown-linux-musl` to package the static CLI.

For a prebuilt Linux archive, extract it and use `./desklink` directly; Rust and a system-wide installation are not required. The commands below use `desklink` for a copy on your `PATH`; substitute `./desklink` when running from the extracted folder. The archive includes the linked documentation and example configuration. Mac bundles include the same documentation under `Contents/Resources/Documentation`.

## Arrange your desk

![Desklink native screen-layout editor](docs/images/layout-editor.png)

1. Open `desklink` (or `desklink ui`) on both computers. Sharing starts only when you press **Start sharing**.
2. Select **This computer** and copy its pairing code from the details panel. Paste it into **Add computer** on the other computer. Exchange codes in both directions through a channel you trust.
3. On the receiver, select the computer whose keyboard you will use under **Layout controller**. A pairing code can also select it using **Use its keyboard and screen layout**.
4. On the controller, drag the cards into their physical positions. Touching edges create crossings; a corner alone does not. Resize a card's corner to represent its screen proportions. **Save layout** applies the arrangement and sends it to connected receivers.
5. Start sharing on every computer and approve any desktop input permissions. Move across a shared edge to change computers. Offline computers keep their place, but cannot take control.

The same controller layout appears on receivers as a read-only arrangement. Layout updates never add trusted computers. A controller connects directly to each receiver, so a row of three computers can be crossed in either direction without routing input through the middle one.

**Ctrl+Alt+Shift+Escape** returns control locally. Closing the editor leaves an active sharing agent running; use **Stop sharing** or `desklink stop` to end it. Text and image clipboard sharing has a visible checkbox. Saving computer or clipboard settings restarts the agent. Position changes apply live on X11. Wayland changes that alter capture edges restart sharing and request fresh desktop consent.

## CLI pairing (legacy edge mode)

First initialize each computer:

```sh
desklink init --name "Linux desktop"
desklink identity
```

```sh
desklink init --name "Mac laptop"
desklink identity
```

Exchange the **public** keys printed by `identity` using a channel you trust. Do not copy `identity.json`, which holds the private key. Assuming the Mac is to the right of Linux:

```sh
# On Linux: paste the Mac's 64-character public key.
desklink trust MAC_PUBLIC_KEY --name Mac --address 192.168.1.20:24880 --edge right

# On the Mac: paste Linux's public key.
desklink trust LINUX_PUBLIC_KEY --name Linux --address 192.168.1.10:24880 --edge left
```

Then run on both computers:

```sh
desklink run
```

Move across the shared edge to switch computers. **Ctrl+Alt+Shift+Escape** restores local control. On a Mac keyboard this is Control+Option+Shift+Escape. Ctrl+C stops the program and releases input.

The default port is **TCP 24880**, avoiding the ports of an existing Synergy installation. Both peers listen and may connect; duplicate connections are resolved automatically. Pairing alone does not configure a firewall. At least one peer must be reachable at its configured address.

On macOS, run `desklink permissions --request` and approve the executable/application in System Settings → Privacy & Security. Use `desklink doctor` to inspect access. The permission identity of a terminal executable can differ from a signed application bundle; see [macOS packaging](docs/macos.md).

## Clipboard

Text and PNG images travel in both directions, independently of which computer currently owns the keyboard. Input has priority over clipboard chunks. Simultaneous changes converge using a logical clock, and applying a remote clipboard does not echo it back.

- Text limit: 1 MiB of UTF-8.
- PNG transfer limit: 16 MiB; image dimensions are bounded to 16 million pixels.
- File-manager copy/paste and rich-text/HTML formats are not implemented.
- Clipboard contents are never written to application logs.

To disable sharing explicitly:

```sh
desklink run --no-clipboard
```

Or set `clipboard = false` in the configuration. This also stops reading the clipboard for sharing.

## Configuration and operations

`desklink config-path` prints the configuration location. The default is the platform's user configuration directory; `--config /path/to/config.toml` selects a separate profile with its own adjacent identity file. See [the example](examples/config.toml).

```sh
desklink doctor
desklink untrust PEER_PUBLIC_KEY
RUST_LOG=desklink=debug desklink run
```

`desklink status` reports the local sharing agent. `desklink stop` releases input and stops it. Restart a CLI agent after changing configuration or trust; the GUI handles those restarts when saving. Shared layout edits apply live through the editor. Identities are created with owner-only file permissions; initialization does not overwrite an existing identity.

Only one process may run a given profile. Independent `--config` profiles can run separately, with different identities and listening ports.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --locked
python3 tests/x11_e2e.py
python3 tests/layout_e2e.py
python3 tests/ui_e2e.py --sharing
```

The integration scripts require Xvfb, xdotool, xclip, xev, Python Pillow and ImageMagick. They create isolated displays and never send test input to your active desktop. See [testing](docs/testing.md), [layout design](docs/layout-editor-design.md), [architecture](docs/architecture.md), [wire protocol](docs/protocol.md), and [contributing](CONTRIBUTING.md). Protocol v2 requires updating all computers from 0.1 together.

## License

MIT for Desklink's original code. Cargo dependencies retain their respective licenses.
