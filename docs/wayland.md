# Wayland support

Desklink uses the desktop's InputCapture portal to receive physical input and its RemoteDesktop portal to inject input through EI. It uses the Clipboard portal for text and PNG sharing. It does not access raw input devices or install a privileged helper.

Both InputCapture and RemoteDesktop interfaces are currently required by the backend, even for a computer used primarily as a receiver. `desklink doctor` probes input interface availability without starting capture. Running the application creates portal sessions and may present multiple approval dialogs.

Clipboard sharing requires a backend that implements the Clipboard portal and grants clipboard access to the RemoteDesktop session. If that interface is unavailable, the application reports an error; `--no-clipboard` is an explicit input-only alternative. It does not silently claim that unsupported clipboard sharing works.

The capture session places pointer barriers on configured outer edges, then receives input over EI after the compositor activates a barrier. If no connected peer takes control, unused capture is released automatically. Capture deactivation resets the sharing state so a source computer is not left forwarding input into a revoked session.

Clipboard pipes are asynchronous and nonblocking, with byte limits and timeouts. A stalled clipboard owner therefore does not block the input worker. Selection ownership distinguishes locally copied data from clipboard data applied by Desklink.

Known limitations needing native acceptance testing:

- Compositor/portal versions differ. GNOME and KDE must be tested independently.
- A display-layout change currently closes the backend with an explicit restart/reauthorization message.
- Paused or disconnected emulation devices currently end the backend to restore local control; automatic session reconstruction after lock/sleep is not yet implemented.
- Screen-edge placement uses the outer bounds of available zones. Irregular monitor arrangements need additional geometry coverage.
- Portal permissions are requested at startup; restore tokens are not yet persisted.

Current automated coverage includes a real EI client/server socket exchange, device negotiation, keyboard/pointer/button/scroll frames, fractional pointer accumulation, and bounded/cancellable clipboard pipes. These tests do not substitute for testing the portal permission lifecycle on a real Wayland desktop.
