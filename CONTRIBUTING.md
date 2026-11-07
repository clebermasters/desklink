# Contributing

Keep the sharing policy independent of the OS and transport. New behavior belongs in the smallest responsible crate, with tests for externally visible behavior or a meaningful failure boundary.

Before committing, run formatting, Clippy with warnings denied and workspace tests. Input changes also require the isolated X11 integration suite; macOS and Wayland changes require native desktop results where available. Record a compile check as a compile check, not as a successful input test.

Never add real private identities, clipboard contents, packet captures with user data, or workstation configuration to the repository. Use temporary profiles and synthetic text/images for tests. Do not run automated input tests against the active desktop.

Describe the concrete behavior change, the affected platforms and the checks you ran. Keep protocol changes versioned and document their compatibility implications.
