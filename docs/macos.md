# macOS distribution and permissions

The Quartz backend uses a session event tap rather than a root-only HID event tap. It posts input through Core Graphics and uses the native pasteboard for text and images. It does not install a daemon or kernel extension.

Screen handoff snapshots keys and mouse buttons already held in the current login session, with modifiers replayed before shortcut action keys. Caps Lock is treated as latch state rather than a held modifier. Other-button drag events preserve their Quartz button identifier. The conversion and ordering policy has portable regression tests; framework interaction still needs native Mac acceptance testing. These paths use Apple's documented [session key-state query](https://developer.apple.com/documentation/coregraphics/cgeventsource/keystate(_:key:)) and [mouse-event button argument](https://developer.apple.com/documentation/coregraphics/cgevent/init(mouseeventsource:mousetype:mousecursorposition:mousebutton:)).

Build on a Mac with Rust and Xcode command-line tools. The release script can combine the Apple Silicon and Intel builds with `lipo` into one executable inside `Desklink.app`. The bundle's metadata gives the executable a stable application identity for privacy prompts.

Initialize and pair the app from Terminal using `Desklink.app/Contents/MacOS/desklink init` and the `trust` commands in the README. After setup, launching the bundle with no arguments starts sharing. This release provides a CLI rather than a settings window or menu-bar controller; use a user LaunchAgent for managed startup/stop, or the command-line executable in a terminal.

Input access remains controlled by macOS. Use `desklink permissions --request`, then review Accessibility and Input Monitoring in System Settings → Privacy & Security. A standard user or managed Mac may need administrator/IT involvement to authorize access. Signing and notarization improve distribution trust but do not grant input privileges.

For normal daily operation, run the program in the signed-in user's session. Login items or a user LaunchAgent can start it after login. System launch daemons, login-screen control and FileVault preboot are outside this design.

The developer must supply their own Developer ID identity and notarization credentials for a public signed release. Development bundles can be built without those credentials, but must not be presented as signed/notarized releases. Validate the actual packaged application's permission behavior on supported macOS versions.
