//! Asks the Bluetooth link to one computer for low latency, and nothing else. Installed with
//! only CAP_NET_RAW by `scripts/linux-bluetooth-hid.sh`, so Desklink itself stays unprivileged.
fn main() {
    #[cfg(target_os = "linux")]
    {
        let mut args = std::env::args().skip(1);
        let (Some(address), None) = (args.next(), args.next()) else {
            eprintln!("usage: desklink-bt-qos AA:BB:CC:DD:EE:FF");
            std::process::exit(2);
        };
        if let Err(error) = desklink_bluez::qos::request_low_latency(&address) {
            eprintln!("desklink-bt-qos: {error:#}");
            std::process::exit(1);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("desklink-bt-qos only runs on Linux");
        std::process::exit(1);
    }
}
