# Native layout editor

The current focus is a tested Linux editor, including X11 and Wayland. macOS uses the same UI and layout code; native Mac builds and acceptance are deferred until a Mac is available. Wayland remains a supported target, not a reason to block X11 development.

## User experience

One native Desklink window contains a computer list, a desk canvas, and a selected-computer inspector. Computers are draggable cards. Cards snap to adjacent edges and aligned corners. Visible seams show where a pointer can cross. Offline computers remain in the saved arrangement. Overlaps and disconnected arrangements prevent applying a layout and explain the correction.

The main actions are Add computer, Save layout, Start sharing and Stop sharing. Changes are drafts until saved. Clipboard sharing remains visible. The current computer is clearly marked. The inspector shows pairing identity, address, and connection status. Editing and starting input sharing are separate actions.

The same embedded fonts, controls, colors and canvas implementation are used on every OS. The native window uses egui/eframe with X11 and Wayland backends enabled. Standard OS graphics libraries remain required; no Electron, Qt, GTK or WebKit runtime is bundled. The headless CLI remains available as a static build without the GUI feature.

## Shared layout

A layout contains a schema version, monotonic revision, controller identity and logical rectangles for the computers. Rectangles describe the physical desk arrangement independently of actual desktop pixel resolutions. The controller is the authority for edits and owns the keyboard/mouse stream. Receivers explicitly select the trusted controller whose layout they accept. Layout synchronization never adds trusted computers or grants input permission.

Positive edge overlap creates a bidirectional seam. Several computers may share different parts of one edge. A crossing maps the position along the source seam to the corresponding part of the destination, preserving offsets and different display sizes. Screens must not overlap and the arrangement must be connected to its controller.

The controller maintains direct authenticated connections to the computers it controls. A receiver reaching another seam asks the controller to switch the input stream. The controller validates the requested crossing against its own layout, releases held input on the old receiver, and begins a new handoff token on the next receiver. This permits a row of three computers without forwarding input through intermediate machines.

## Runtime and persistence

Configuration and layout validation live in reusable crates. The UI saves atomically and applies changes through a private per-profile Unix socket. X11 positions apply live. On Wayland, an edit that changes capture spans restarts the agent to obtain a fresh portal session and desktop consent. Other layout edits still apply live. Status updates contain computer identities and sharing state, never clipboard contents or keystrokes. Only the explicit Start action launches the sharing agent. The agent is another process of the same executable, preserving main-thread requirements for native input APIs.

The GUI and CLI use the same configuration. Legacy left/right/top/bottom settings remain supported when no shared layout is configured. Shared layout updates are accepted only from the configured controller and only when their revision is newer.

## Verification

Portable tests cover seams, offsets, split edges, snapping, overlaps, disconnected layouts, stale revisions and controller trust. Linux integration tests cover saved layout edits, live synchronization, three-computer handoffs, keyboard/mouse/clipboard, and disconnected destinations. Native UI tests use isolated X11 and nested Wayland sessions, with screenshots inspected for layout and interaction quality. Native macOS results remain explicitly deferred.

Craft Anything observations are kept under the ignored analysis directory. Its static inspection identified draggable rectangles, overlap rejection, border/corner snapping and service-backed settings in Synergy. These are design references; Desklink's implementation is independent.
