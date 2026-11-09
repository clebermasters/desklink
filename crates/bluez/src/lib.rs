//! Presents this Linux computer to another computer as a Bluetooth keyboard and mouse.
//!
//! The reports come from [`desklink_hid::HidDesktop`]; this crate only carries them over
//! BlueZ, so the controlled computer needs no Desklink software and no permissions.
pub mod hidp;
pub mod record;

#[cfg(target_os = "linux")]
pub mod device;
#[cfg(target_os = "linux")]
pub mod qos;
#[cfg(target_os = "linux")]
pub mod socket;

#[cfg(target_os = "linux")]
pub use device::{BluezSink, HidDevice, Options};

/// Whether bluetoothd runs with its classic input plugin, which claims the keyboard-and-mouse
/// channels for itself. `None` when bluetoothd is not running or cannot be inspected.
pub fn input_plugin_active() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let path = entry.path();
            let comm = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
            if comm.trim() != "bluetoothd" {
                continue;
            }
            let cmdline = std::fs::read(path.join("cmdline")).ok()?;
            let args: Vec<_> = cmdline
                .split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            return Some(!disables_input(&args));
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// `-P input`, `-Pinput`, `--noplugin=input` or a comma list containing `input`.
fn disables_input(args: &[String]) -> bool {
    let lists = args.iter().enumerate().filter_map(|(i, arg)| {
        if arg == "-P" || arg == "--noplugin" {
            args.get(i + 1).map(String::as_str)
        } else {
            arg.strip_prefix("--noplugin=")
                .or_else(|| arg.strip_prefix("-P"))
        }
    });
    lists
        .flat_map(|list| list.split(','))
        .any(|plugin| plugin.trim() == "input")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split(' ').map(String::from).collect()
    }

    #[test]
    fn recognises_every_way_to_disable_the_input_plugin() {
        assert!(disables_input(&args(
            "/usr/lib/bluetooth/bluetoothd -P input"
        )));
        assert!(disables_input(&args("bluetoothd -Pinput")));
        assert!(disables_input(&args("bluetoothd --noplugin=sap,input")));
        assert!(disables_input(&args("bluetoothd --noplugin input")));
        assert!(!disables_input(&args("bluetoothd")));
        assert!(!disables_input(&args("bluetoothd -P sap")));
        assert!(!disables_input(&args("bluetoothd -P inputs")));
    }
}
