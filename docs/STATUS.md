# Implementation status

The objective remains a functional, modular macOS/Linux sharing application with clipboard support and a professional Git project. The current implementation is a working CLI application; native platform acceptance remains part of the unfinished work.

## Delivered in the working tree

- Six Rust crates separating protocol, secure transport, sharing state, clipboard, OS backends, and CLI/configuration.
- Native X11 input, Quartz input, and Wayland portal/EI implementations.
- Bidirectional text/PNG clipboard sharing with limits, chunking, digest verification, convergence and echo prevention.
- Explicit mutual public-key pairing and encrypted Noise connections.
- Handoff tokens, concurrent-handoff arbitration, escape chord, input release on failure, reconnects and single-profile instance locking.
- Configuration validation, diagnostics, permission onboarding commands, examples and developer documentation.
- CI definitions, a Linux user-service example, static Linux packaging and macOS universal-bundle/signing/notarization scripts.

## Authoritative verification so far

- 37 workspace tests pass, including actual encrypted socket exchanges, actual EI wire exchanges, stale clipboard-read rejection and portable macOS input-policy regressions.
- Isolated X11 end-to-end tests pass for both input directions, different resolutions, text and image clipboard contents/alpha, concurrent-copy convergence, held shortcut ordering, scrolling, emergency release, crash cleanup, reconnects and clipboard opt-out.
- The release Linux x86-64 executable is a single statically linked file and is checked with the same real two-desktop test.
- Apple Silicon and Intel macOS targets pass compile checks; ARM64 Linux passes a compile check.
- Formatting, Clippy, YAML/shell syntax and the macOS property list are checked locally. GitHub CI definitions have not been run on a hosted repository.

## Still required before claiming the full objective complete

1. Run the application on a Mac and verify standard-user permission setup, capture/injection, modifiers, scrolling, text/images, disconnects and packaging behavior. A target Mac/version or SSH host has been requested from the user; none has yet been supplied.
2. Run the portal backend on supporting GNOME/KDE Wayland desktops and exercise approval, capture, injection and clipboard. Current wire-level tests alone cannot prove those desktop integrations.
3. Resolve any platform failures found by those native acceptance tests and update the compatibility matrix with actual versions/results.

The local desktop is GNOME Shell 42.9 in an X11 session. It is not an available modern Wayland portal acceptance environment. No Mac target has been supplied. Cross-compilation and portable policy/wire tests remain explicitly separate from native desktop verification.

The existing Synergy installation and normal desktop input were not modified for testing. The earlier binary investigation remains outside Git in the ignored `analysis/` directory. No private identity or real clipboard contents belong in this repository.
