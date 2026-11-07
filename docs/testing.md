# Verification

## Automated portable checks

Run formatting, Clippy with warnings denied, and the workspace tests. Tests cover trust rejection, encrypted bidirectional transfers, bounded framing, clipboard corruption, PNG pixel preservation, concurrent clipboard convergence, handoff acknowledgment, stale sessions, invalid coordinates, emergency return and held-input release.

## X11 end-to-end test

Build `desklink`, then run `python3 tests/x11_e2e.py`. The test launches two independent Xvfb desktops at different resolutions, creates temporary identities, pairs both peers and runs the real application processes.

It verifies real Unicode text and PNG clipboard transfers in both directions, concurrent-copy convergence, real pointer movement, keyboard input and clicks, scrolling, screen-edge return, reverse input sharing, emergency return, crash cleanup and reconnection with the same identity. It also checks clipboard opt-out. Input commands always have an explicit private `DISPLAY`. The real desktop is not a test target.

The script needs Xvfb, xdotool, xclip, xev, `stdbuf`, and Pillow. CI provisions these as test dependencies; running the application does not require them.

## macOS acceptance

Compile checks do not prove desktop behavior. On a test Mac:

1. Test with a standard user and record any administrator authorization needed during permission setup.
2. Grant only the input permissions reported by `desklink doctor`/`permissions`.
3. Pair with the Linux test computer; exercise both controller and receiver roles.
4. Verify pointer movement, dragging, all modifier combinations, Command shortcuts, repeat, Caps Lock, scrolling, non-US layouts and multi-monitor coordinates. Include keys/buttons held before crossing a screen edge, and verify the original application retains no stuck input after returning.
5. Copy multiline Unicode text and transparent screenshots in both directions.
6. Test the emergency chord, network loss while modifiers/buttons are held, sleep/wake, permission revocation and restart.
7. Repeat using the packaged/signed app identity, including upgrade behavior. Do not infer it from permissions granted to Terminal.

## Wayland acceptance

Use a desktop with InputCapture, RemoteDesktop and EIS support. Clipboard integration also requires the Clipboard portal. Verify portal interface availability, explicit user grants, both controller/receiver roles, each configured edge, text/images, rejected grants, capture deactivation and clean shutdown. Test GNOME and KDE separately; a generic Wayland version is not a sufficient compatibility claim.

Keep exact desktop, compositor and portal package versions with any result. A private/nested desktop or test account is preferred over automating an active personal desktop.
