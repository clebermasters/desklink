# Changelog

## 0.1.0 — in development

- Modular Rust engine with native X11 and Quartz backends.
- Authenticated, encrypted peer connections and reconnection.
- Screen-edge handoff, reverse return, emergency release and disconnect recovery.
- Bidirectional text and PNG clipboard sharing, limits and loop prevention.
- Discard queued clipboard observations and completed portal reads after a newer remote copy.
- Preserve macOS drag-button identifiers, held input, and Caps Lock state at handoff; establish modifiers before held shortcuts on X11 and macOS.
- Portal/EI backend for supporting Wayland desktops, under validation.
- Isolated two-desktop integration tests and cross-platform build checks.
