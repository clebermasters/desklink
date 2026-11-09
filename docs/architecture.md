# Architecture

The workspace separates portable policy from I/O. Platform callbacks and clipboard owners never perform network handshakes or wait for a peer.

| Crate | Owns | Does not own |
| --- | --- | --- |
| `desklink-protocol` | Wire types, limits, framing payloads, coordinates, clipboard assembly | Network or OS access |
| `desklink-transport` | Identities, trust checks, Noise handshakes, socket framing, keepalives, prioritized writes | Input injection or routing |
| `desklink-core` | Handoff state machine, held input, edge routing, clipboard logical clock | Operating-system or socket code |
| `desklink-platform` | X11, Quartz, Wayland input; portal clipboard integration | Peer trust or network protocol |
| `desklink-clipboard` | Native text/PNG selection ownership and polling on X11/macOS | Remote connection management |
| `desklink-layout` | Logical rectangles, validation, seams, snapping and capture spans | OS pixels, networking or GUI |
| `desklink-settings` | Shared configuration, atomic saves, pairing codes and local control types | Rendering or capture |
| `desklink-ui` | Native screen editor, pairing and sharing controls, status | Input injection or networking |
| `desklink` | CLI, local agent control, connection supervision, action execution | Platform-specific input mechanics |
| `desklink-hid` | HID report descriptor, evdev→HID usages, a receiving `Desktop` that emits keyboard and pointer reports | Any transport (Bluetooth: the Android app, or `desklink-bluez`) |
| `desklink-bluez` | BlueZ D-Bus profiles and pairing agent, the HIDP control/interrupt channels, reconnection and the low-latency (HCI QoS) request | Linux |
| `desklink-android` | JNI library hosting the same runtime inside the Android app: action queue, Android `Desktop`, key map, phone profile | Performing input (done by the app's accessibility or ADB-helper backend) |

The `desklink` package is a library plus the executable. `runtime::run_with_host` runs the agent with any `Desktop`, a polling interval, optional signal handling and a shutdown token. The desktop executable and the Android app embed that same runtime, so handoff, trust and clipboard rules stay identical. See [Android](android.md).

## Input lifecycle

1. Local movement reaches a configured outer edge with a connected peer.
2. The engine captures local input and sends an `Enter` with a unique session token, destination edge and normalized position.
3. The receiver positions its pointer, enters receiving state and acknowledges the token.
4. Until acknowledgment, input is bounded and buffered. Once acknowledged, input is forwarded in order.
5. Motion updates receiver coordinates. In a shared arrangement, reaching a seam requests a switch from the controller; it releases the old receiver and sends a new handoff to the next computer. Reaching the controller returns local control. Legacy edge-only profiles return through their entry edge.
6. Disconnect, a rejected handoff, timeout, capture revocation or the emergency chord resets the engine and releases tracked input.

Sessions contain the controller's public identity plus a monotonically increasing sequence. Delayed acknowledgments, releases and input from an earlier handoff cannot affect a newer handoff. If two users initiate a handoff at the same time, public-key ordering determines which request wins.

Every sharing agent is one process with an async networking runtime. The native GUI launches the same executable as a separate agent process and communicates through a private per-profile Unix socket. Closing the GUI leaves an active agent running; Stop sharing ends it. X11 requests and Quartz callbacks belong to the agent's desktop thread. Native clipboard work runs on its own thread because selection owners may respond slowly. The Wayland worker owns its asynchronous portal sessions, EI connections and clipboard pipes on a separate runtime.

## Clipboard lifecycle

The native clipboard service observes text or image changes and normalizes images to PNG. A local change receives a logical-clock stamp `(counter, origin)`. Each remote transfer advertises its format, bounded size and digest, then streams chunks. The receiver verifies completeness and digest before applying it.

The clock imposes a total order on simultaneous changes. Remote changes with older stamps are discarded. Applying a remote change updates the service's last-observed digest so it does not send the same contents back as a new local edit. Portal clipboard ownership signals serve the equivalent role on Wayland.

Clipboard observations also carry a local revision. Applying a remote copy invalidates queued observations immediately, before the native clipboard thread runs. Wayland reads are similarly invalidated when ownership changes, including reads that finished but have not yet been consumed. This prevents a delayed read of old contents from becoming a new logical-clock update and overwriting a newer remote copy.

Control/input messages have priority between clipboard chunks. Slow connections have bounded control queues; an overflow closes the peer rather than silently dropping key releases.

## Extending backends

Implement `Desktop` and produce `LocalEvent` values. Core routing depends only on the portable `Geometry` and `Input` types. An input backend must report failure or revoked capture; pretending capture continues would strand the source pointer. A backend that owns clipboard integration supplies it through the same local event stream.

## Distribution

Linux and macOS use separate native builds. A macOS universal executable can contain Intel and Apple Silicon slices. The preferred Mac distribution is a signed/notarized application bundle containing one main executable and its metadata. Configuration and private identity files belong to the user profile, not to the application installation directory.
