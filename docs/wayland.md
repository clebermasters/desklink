# Wayland support

Desklink uses the desktop's InputCapture portal to receive physical input and its RemoteDesktop portal to inject input through EI. It uses the Clipboard portal for text and PNG sharing. It does not access raw input devices or install a privileged helper.

InputCapture is required. When RemoteDesktop is absent (for example on Hyprland), explicitly disable clipboard sharing with `clipboard=false` in the configuration or `desklink run --no-clipboard` to send keyboard and mouse input to another computer. This mode cannot receive input; incoming handoffs are rejected. Returning restores the local pointer near the edge where capture began. RemoteDesktop remains required for receiving input and portal clipboard sharing. Denied or failed emulation sessions never fall back to sending only. `desklink doctor` probes input interface availability without starting capture. Running the application creates portal sessions and may present multiple approval dialogs.

Clipboard sharing requires a backend that implements the Clipboard portal and grants clipboard access to the RemoteDesktop session. If that interface is unavailable, the application reports an error; `--no-clipboard` is an explicit input-only alternative. It does not silently claim that unsupported clipboard sharing works.

The capture session places pointer barriers on configured outer edges, then receives input over EI after the compositor activates a barrier. If no connected peer takes control, unused capture is released automatically. Capture deactivation resets the sharing state so a source computer is not left forwarding input into a revoked session.

The native GUI uses Wayland window handles directly. Layout seams determine the exact spans armed for capture. When a saved edit changes these spans, the GUI stops sharing cleanly and starts a fresh portal session. Approve the desktop dialogs again. Edits that keep the same capture spans apply without restarting.

This restart handles a limitation observed in GNOME/Mutter 48.7: disabling capture clears its EIS device references, while the portal rejects a second EIS connection on that session. Continuing the existing session can therefore arm a barrier without delivering input. See the [Mutter 48.7 implementation](https://github.com/GNOME/mutter/blob/48.7/src/backends/meta-input-capture-session.c) and the intended [portal lifecycle](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.InputCapture.html). Desklink uses a fresh consent session instead of treating that state as working capture.

Clipboard pipes are asynchronous and nonblocking, with byte limits and timeouts. A stalled clipboard owner therefore does not block the input worker. Selection ownership distinguishes locally copied data from clipboard data applied by Desklink.

Known limitations needing native acceptance testing:

- Compositor/portal versions differ. GNOME and KDE must be tested independently.
- A display-layout change currently closes the backend with an explicit restart/reauthorization message.
- Disabled capture, paused or disconnected emulation devices end the backend to restore local control; automatic session reconstruction after lock/sleep is not yet implemented.
- Screen-edge placement uses the outer bounds of available zones. Irregular monitor arrangements need additional geometry coverage.
- Portal permissions are requested at startup; restore tokens are not yet persisted.

Native acceptance now covers GNOME 48.7 in an isolated headless compositor: real permission dialogs, native UI gestures, changed-layout reauthorization, capture, keyboard/pointer/buttons/scroll, edge return, native GTK text/PNG clipboard sharing in both directions and the UI's Copy pairing code action. The selection decoder accepts both the documented MIME array and GNOME 48's one-field wrapped array; it consumes transfer notifications directly without silently dropping undecodable events.

The test also switches the controller through the GUI and verifies the receiver role: X11 input reaches a native GTK Wayland text field through EI, with edge return to the controller.

Portable coverage also includes a real EI client/server socket exchange, device negotiation, keyboard/pointer/button/scroll frames, fractional pointer accumulation, and bounded/cancellable clipboard pipes. GNOME results do not establish KDE or other compositor support. See [exact versions and remaining limits](STATUS.md).
