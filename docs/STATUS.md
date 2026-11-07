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

## Platform limits and follow-up

- On Wayland, changed capture spans restart sharing and request fresh consent. GNOME 48 cannot retain working EIS capture references through the needed disable/update cycle. Layout edits with unchanged spans apply live. X11 applies position edits live.
- KDE and other compositor/portal versions need independent acceptance. Irregular multi-monitor zones, permission revocation and lock/sleep recovery need broader native coverage. Disabled/paused sessions end sharing with an explicit error so the user can restart safely.
- macOS 0.2 UI compilation, standard-user permission setup, native input/clipboard, signing and bundle testing remain deferred. Earlier headless Apple Silicon/Intel compile checks do not validate the new GUI.
- Legacy edge-only profiles remain supported, but protocol v2 requires updating all paired computers together.

All automated desktop input and clipboard data are synthetic and confined to private Xvfb or GNOME sessions. The installed Synergy application and the user's normal desktop were not modified. Craft Anything evidence stays in the ignored analysis directory; Desklink contains an independent implementation.

The Wayland acceptance image used gnome-shell `48.7-0+deb13u2`, libmutter `48.7-0+deb13u1`, xdg-desktop-portal `1.20.3+ds-1`, xdg-desktop-portal-gnome `48.0-2` and GTK `4.18.6+ds-2`. The GUI window and GTK clipboard client both report native Wayland display types. Nested GNOME is used only for editor gestures; the native headless compositor is used for portal input and clipboard acceptance.
