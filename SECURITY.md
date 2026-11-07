# Security model

Desklink trusts computers that the user has explicitly paired by public key. A paired computer may supply input and, when enabled, clipboard data within the signed-in desktop session. The trust decision therefore applies to the whole paired computer, not just its current user-facing application.

Connections use the established Noise XX construction implemented by Snow. Peer static keys are verified against a local allowlist before a connection can deliver application messages. Framing, control queues, clipboard sizes and transfer duration are bounded. No command-execution or remote-file-management protocol is exposed.

Clipboard contents are sensitive. They stay in memory, are sent only to paired peers when sharing is enabled, and are excluded from application logs. `--no-clipboard` disables clipboard observation and transfer. X11 layout edits apply live; Wayland capture-span changes require a fresh portal session. Computer, trust and clipboard changes require an agent restart, which the GUI performs when saving those changes. CLI users must restart the agent after trust changes.

Shared layout authority is explicit: a receiver selects one trusted controller. Received layouts cannot add trusted keys. GUI/agent control uses a private Unix socket and configuration writes use a file lock plus an optimistic digest check. A public pairing code contains a name, address and public key only; exchange and verify it through a channel you trust on both computers.

Keep `identity.json` private. Possession of its secret key allows impersonating that computer until the key is removed from every peer. Replacing a lost/compromised identity requires generating and exchanging a new public key.

The application runs with the OS permissions granted to its user. It does not bypass macOS privacy controls, Wayland portal consent, firewall policy, screen locks or preboot authentication.

This is a new implementation and has not undergone an independent security audit. Report a reproducible issue privately to the project maintainer before publishing sensitive details. Use synthetic identities and clipboard data in any reproduction.
