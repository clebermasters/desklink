//! Asks the link to the host for low latency.
//!
//! Without it, the host polls the keyboard only every 25–70 ms, so pointer motion arrives in
//! visible clumps; real Bluetooth mice ask for this. The request is an HCI command, which takes
//! CAP_NET_RAW: the tiny `desklink-bt-qos` helper holds that one capability (installed by
//! `scripts/linux-bluetooth-hid.sh`) and does nothing else.
use crate::socket::parse_address;
use anyhow::{Context, Result, bail};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::Command,
};

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_HCI: libc::c_int = 1;
const HCI_CHANNEL_RAW: u16 = 0;
const HCI_COMMAND_PKT: u8 = 0x01;
const ACL_LINK: u8 = 0x01;
/// `_IOR('H', 213, int)`
const HCIGETCONNINFO: libc::c_ulong = 0x8004_48d5;
/// Link Policy commands, QoS Setup.
const QOS_SETUP: u16 = (0x02 << 10) | 0x0007;
/// What a host should guarantee: a poll every 2.5 ms, about what a wired mouse gets.
const LATENCY_US: u32 = 2_500;
const TOKEN_RATE: u32 = 800; // bytes per second: 100 reports of 8 bytes
const MAX_ADAPTERS: u16 = 8;

pub const HELPER: &str = "desklink-bt-qos";
/// Where the admin script installs the helper.
pub const INSTALLED_HELPER: &str = "/usr/local/libexec/desklink/desklink-bt-qos";

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

#[repr(C)]
#[derive(Default)]
struct ConnInfo {
    handle: u16,
    bdaddr: [u8; 6],
    kind: u8,
    out: u8,
    state: u16,
    link_mode: u32,
}

#[repr(C)]
#[derive(Default)]
struct ConnInfoRequest {
    bdaddr: [u8; 6],
    kind: u8,
    info: ConnInfo,
}

fn raw_socket(dev: u16) -> io::Result<OwnedFd> {
    // SAFETY: plain socket creation; the descriptor is owned immediately.
    let fd = unsafe {
        let fd = libc::socket(
            AF_BLUETOOTH,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            BTPROTO_HCI,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        OwnedFd::from_raw_fd(fd)
    };
    let address = SockaddrHci {
        family: AF_BLUETOOTH as libc::sa_family_t,
        dev,
        channel: HCI_CHANNEL_RAW,
    };
    // SAFETY: the address points to a live sockaddr_hci of the length given.
    let bound = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const SockaddrHci).cast(),
            size_of::<SockaddrHci>() as libc::socklen_t,
        )
    };
    if bound < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn command(opcode: u16, parameters: &[u8]) -> Vec<u8> {
    let mut packet = vec![HCI_COMMAND_PKT];
    packet.extend(opcode.to_le_bytes());
    packet.push(parameters.len() as u8);
    packet.extend_from_slice(parameters);
    packet
}

fn qos_setup(handle: u16) -> Vec<u8> {
    let mut parameters = Vec::with_capacity(20);
    parameters.extend(handle.to_le_bytes());
    parameters.push(0); // flags
    parameters.push(0x02); // guaranteed service
    parameters.extend(TOKEN_RATE.to_le_bytes());
    parameters.extend(0u32.to_le_bytes()); // peak bandwidth: unknown
    parameters.extend(LATENCY_US.to_le_bytes());
    parameters.extend(u32::MAX.to_le_bytes()); // delay variation: don't care
    command(QOS_SETUP, &parameters)
}

/// Sends the request on whichever adapter has a connection to `address`.
pub fn request_low_latency(address: &str) -> Result<()> {
    let mut bdaddr = parse_address(address)?;
    bdaddr.reverse();
    for dev in 0..MAX_ADAPTERS {
        let Ok(socket) = raw_socket(dev) else {
            continue;
        };
        let mut request = ConnInfoRequest {
            bdaddr,
            kind: ACL_LINK,
            ..Default::default()
        };
        // SAFETY: the request struct is live, writable and as large as the kernel expects.
        let found = unsafe {
            libc::ioctl(
                socket.as_raw_fd(),
                HCIGETCONNINFO as _,
                &mut request as *mut ConnInfoRequest,
            )
        };
        if found < 0 {
            continue;
        }
        let packet = qos_setup(request.info.handle);
        // SAFETY: the packet buffer is live for the call.
        let written =
            unsafe { libc::write(socket.as_raw_fd(), packet.as_ptr().cast(), packet.len()) };
        if written < 0 {
            return Err(io::Error::last_os_error())
                .context("cannot send the low-latency request (the helper needs CAP_NET_RAW)");
        }
        return Ok(());
    }
    bail!("no Bluetooth connection to {address}")
}

fn helper_path() -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(HELPER)));
    std::env::var_os("DESKLINK_BT_QOS")
        .map(PathBuf::from)
        .into_iter()
        .chain([PathBuf::from(INSTALLED_HELPER)])
        .chain(beside)
        .find(|path| Path::new(path).is_file())
}

/// Asks for low latency directly when this process may, otherwise through the helper. Failure
/// only costs smoothness, so it is logged rather than returned.
pub fn improve_latency(address: &str) {
    if request_low_latency(address).is_ok() {
        tracing::debug!("asked the host for low latency");
        return;
    }
    let Some(helper) = helper_path() else {
        tracing::warn!(
            "pointer motion may look choppy: install {HELPER} with \
             `sudo scripts/linux-bluetooth-hid.sh enable`"
        );
        return;
    };
    match Command::new(&helper).arg(address).output() {
        Ok(output) if output.status.success() => tracing::debug!("asked the host for low latency"),
        Ok(output) => tracing::warn!(
            error = %String::from_utf8_lossy(&output.stderr).trim(),
            "the low-latency request failed; pointer motion may look choppy"
        ),
        Err(error) => tracing::warn!(%error, helper = %helper.display(), "cannot run the helper"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qos_setup_is_a_guaranteed_low_latency_request() {
        let packet = qos_setup(0x0100);
        assert_eq!(packet[..4], [0x01, 0x07, 0x08, 20], "command header");
        assert_eq!(packet[4..6], [0x00, 0x01], "handle");
        assert_eq!(packet[7], 0x02, "guaranteed");
        assert_eq!(
            u32::from_le_bytes(packet[16..20].try_into().unwrap()),
            LATENCY_US
        );
        assert_eq!(packet.len(), 24);
    }

    #[test]
    fn kernel_structures_have_the_kernel_layout() {
        assert_eq!(size_of::<SockaddrHci>(), 6);
        assert_eq!(size_of::<ConnInfo>(), 16);
        assert_eq!(std::mem::offset_of!(ConnInfoRequest, info), 8);
    }
}
