# Wire protocol v2

Desklink v2 is independent of the Synergy/Deskflow protocols and incompatible with Desklink 0.1. Update every paired computer together.

## Establishment

TCP uses `TCP_NODELAY`. The connection begins with three Noise XX handshake messages using `Noise_XX_25519_ChaChaPoly_BLAKE2s` and prologue `desklink/2`. Each packet uses a network-order 32-bit length prefix. Handshake frames are limited to 4096 bytes and the exchange has a deadline.

Both peers maintain explicit trust lists of 32-byte Curve25519 public keys. Unknown keys abort the handshake. There is no trust-on-first-use acceptance and no unauthenticated input path. Identity files are created atomically, kept private to their owner, and validated against the derived public key on load.

After the handshake, every length-prefixed packet contains one authenticated Noise transport message. Plaintext is encoded with Postcard and is limited to 32 KiB, plus the 16-byte authentication tag on the wire. Directional Noise counters provide ordered authentication; modified or replayed packets fail decryption.

The first encrypted message is `Hello`, containing the protocol version, a bounded computer name, validated geometry, and clipboard capability. Control begins only after this exchange.

## Control

`Enter`, `Entered`, `Input`, `Return`, and `Release` carry a handoff token `(controller public key, sequence)`. Keys use physical Linux evdev identifiers; X11 converts to X keycodes and macOS uses an explicit hardware mapping. Text layout interpretation belongs to the destination OS. The protocol does not synthesize Unicode text in place of keyboard shortcuts.

`Ping`/`Pong` keep idle connections alive. Reads have a deadline and stalled writes terminate the session. Geometry changes reset active sharing and advertise the new desktop bounds.

`LayoutUpdate` carries validated logical rectangles, an owner identity and monotonic revision. Receivers accept it only from their explicitly selected, paired controller. It cannot change a trust list. `LayoutRequest` asks that controller for its current arrangement. `Traverse` identifies the active handoff token, exit edge and normalized position; the controller validates the crossing and selects the next directly connected receiver. `TraverseDenied` leaves control on the current computer. A successful switch releases old injected input and creates a new token, preserving the physical source's capture and held keys/buttons.

## Clipboard

`ClipboardStart` contains a logical-clock stamp, format (`Text` or `Png`), total size, and SHA-256 digest over the format discriminator plus bytes. `ClipboardChunk` contains at most 24 KiB. `ClipboardEnd` commits only a complete, valid transfer. Overlapping, oversized, truncated, or digest-mismatched transfers fail the session.

One pending clipboard transfer per connection is assembled in bounded memory. Transfer duration is limited. Input is prioritized between chunks; clipboard versions waiting to be sent can be coalesced, while an already-started transfer is completed before a newer one begins.

These limits are protocol requirements, not assertions that every local OS clipboard representation can be transferred. Current formats exclude files and HTML.
