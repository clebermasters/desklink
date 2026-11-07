# Desklink

Share a keyboard, mouse, and clipboard between macOS and Linux computers over your local network. Move across a configured screen edge to control another computer, then move back to return. Each computer runs one executable as its own user.

Desklink is an independent implementation. It does not use Synergy's code, account service, or network protocol.

## Features

- Bidirectional keyboard, pointer, buttons, and scrolling.
- Left, right, top, and bottom screen links, with normalized entry positions across different display sizes.
- **Bidirectional Unicode text and PNG image clipboard sharing**, including transparency. Clipboard sharing is enabled by default.
- Encrypted Noise XX connections with explicit public-key pairing on both computers.
- Automatic reconnection, session identifiers to discard stale input, release of held keys/buttons on disconnect, and an emergency return chord.
- Modular Rust workspace, one executable per OS/CPU target, no privileged service or input driver.

## Platform status

| Platform | Implementation | Verification |
| --- | --- | --- |
| Linux X11 | X11/XTEST input and native clipboard | Two independent Xvfb desktops: real input, text/images, return, crash recovery and reconnect |
| macOS | Quartz session event taps and native pasteboard | Apple Silicon and Intel compile checks; native permission/input testing is still required |
| Linux Wayland | InputCapture/RemoteDesktop/Clipboard portals and EI | Compile, EI wire-protocol and clipboard-pipe tests; compositor acceptance testing is still required |

macOS requires operating-system approval for input access. Some initial permission grants or device policies may require an administrator. Wayland requires the desktop's input portals and approval. Normal sharing runs in the signed-in user's session. Login screens and FileVault preboot are outside the supported session scope.

## Build

Install Rust 1.89 or newer, then:

```sh
cargo build --release --locked
./target/release/desklink doctor
```

The application does not require X11/Qt/OpenSSL development packages on Linux. The X11 and cryptographic client implementations are compiled into the executable. Linux system desktop services are still required; a static musl release can avoid a glibc version dependency.

## Pair two computers

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

Restart a running instance after changing configuration or trust. Removing a trusted key takes effect when the running process reloads on restart. Identities are created with owner-only file permissions; initialization does not overwrite an existing identity.

Only one process may run a given profile. Independent `--config` profiles can run separately, with different identities and listening ports.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --locked
python3 tests/x11_e2e.py
```

The integration script requires Xvfb, xdotool, xclip, xev, and Python Pillow. It starts its own isolated displays and never sends test input to your active desktop. See [testing](docs/testing.md), [architecture](docs/architecture.md), [wire protocol](docs/protocol.md), and [contributing](CONTRIBUTING.md).

## License

MIT for Desklink's original code. Cargo dependencies retain their respective licenses.
