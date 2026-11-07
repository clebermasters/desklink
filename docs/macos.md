# macOS distribution and permissions

The 0.2 native UI uses the same Rust layout/editor code as Linux. Its macOS build, bundle and permission tests are deliberately deferred until a Mac is available. The instructions below describe the intended packaging and acceptance work; they are not a claim of a tested Mac GUI release.

The Quartz backend uses a session event tap rather than a root-only HID event tap. It posts input through Core Graphics and uses the native pasteboard for text and images. It does not install a daemon or kernel extension.

Screen handoff snapshots keys and mouse buttons already held in the current login session, with modifiers replayed before shortcut action keys. Caps Lock is treated as latch state rather than a held modifier. Other-button drag events preserve their Quartz button identifier. The conversion and ordering policy has portable regression tests; framework interaction still needs native Mac acceptance testing. These paths use Apple's documented [session key-state query](https://developer.apple.com/documentation/coregraphics/cgeventsource/keystate(_:key:)) and [mouse-event button argument](https://developer.apple.com/documentation/coregraphics/cgevent/init(mouseeventsource:mousetype:mousecursorposition:mousebutton:)).

Build on a Mac with Rust and Xcode command-line tools. The release script can combine the Apple Silicon and Intel builds with `lipo` into one executable inside `Desklink.app`. The bundle's metadata gives the executable a stable application identity for privacy prompts.

Launching the executable without arguments now opens the layout editor. Pairing, controller selection and sharing controls live in that window. The same executable's `init`, `trust`, `run`, `status` and `stop` commands remain available. Starting sharing launches another process of the same executable so the Quartz backend can retain its main-thread event loop; validate the packaged process's privacy identity during Mac acceptance.

Input access remains controlled by macOS. Use `desklink permissions --request`, then review Accessibility and Input Monitoring in System Settings → Privacy & Security. A standard user or managed Mac may need administrator/IT involvement to authorize access. Signing and notarization improve distribution trust but do not grant input privileges.

For normal daily operation, run the program in the signed-in user's session. Login items or a user LaunchAgent can start it after login. System launch daemons, login-screen control and FileVault preboot are outside this design.

The developer must supply their own Developer ID identity and notarization credentials for a public signed release. Development bundles can be built without those credentials, but must not be presented as signed/notarized releases. Validate the actual packaged application's permission behavior on supported macOS versions.
