//! L2CAP sockets: the HID channels BlueZ hands over, and the ones this device opens itself to
//! reconnect to its host. Outgoing connections need no privileges.
use anyhow::{Context, Result, bail};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_L2CAP: libc::c_int = 0;
const SOL_BLUETOOTH: libc::c_int = 274;
const BT_SECURITY: libc::c_int = 4;
/// Encrypted with a bonded key: the same as BlueZ's "RequireAuthentication".
const BT_SECURITY_MEDIUM: u8 = 2;

#[repr(C)]
struct SockaddrL2 {
    l2_family: libc::sa_family_t,
    l2_psm: u16,
    l2_bdaddr: [u8; 6],
    l2_cid: u16,
    l2_bdaddr_type: u8,
}

#[repr(C)]
struct BtSecurity {
    level: u8,
    key_size: u8,
}

/// Parses "AA:BB:CC:DD:EE:FF".
pub fn parse_address(text: &str) -> Result<[u8; 6]> {
    let parts: Vec<_> = text.split(':').collect();
    if parts.len() != 6 {
        bail!("invalid Bluetooth address {text:?}");
    }
    let mut address = [0; 6];
    for (byte, part) in address.iter_mut().zip(parts) {
        *byte = u8::from_str_radix(part, 16)
            .ok()
            .filter(|_| part.len() == 2)
            .with_context(|| format!("invalid Bluetooth address {text:?}"))?;
    }
    Ok(address)
}

/// BlueZ names devices `/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF`.
pub fn address_of_device_path(path: &str) -> Option<String> {
    let address = path
        .rsplit('/')
        .next()?
        .strip_prefix("dev_")?
        .replace('_', ":");
    parse_address(&address).ok().map(|_| address)
}

fn check(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

/// Opens an encrypted L2CAP channel to `address`; blocks for up to the page timeout (~5 s).
pub fn connect(address: &str, psm: u16) -> Result<OwnedFd> {
    let mut bdaddr = parse_address(address)?;
    bdaddr.reverse(); // the kernel stores addresses least significant byte first
    // SAFETY: plain socket creation; the descriptor is owned immediately.
    let fd = unsafe {
        OwnedFd::from_raw_fd(check(libc::socket(
            AF_BLUETOOTH,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            BTPROTO_L2CAP,
        ))?)
    };
    let security = BtSecurity {
        level: BT_SECURITY_MEDIUM,
        key_size: 0,
    };
    // SAFETY: the option value points to a live struct of the length given.
    check(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            SOL_BLUETOOTH,
            BT_SECURITY,
            (&security as *const BtSecurity).cast(),
            size_of::<BtSecurity>() as libc::socklen_t,
        )
    })?;
    let target = SockaddrL2 {
        l2_family: AF_BLUETOOTH as libc::sa_family_t,
        l2_psm: psm.to_le(),
        l2_bdaddr: bdaddr,
        l2_cid: 0,
        l2_bdaddr_type: 0, // BR/EDR
    };
    // SAFETY: the address points to a live sockaddr_l2 of the length given.
    check(unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&target as *const SockaddrL2).cast(),
            size_of::<SockaddrL2>() as libc::socklen_t,
        )
    })
    .with_context(|| format!("cannot reach {address} on PSM {psm}"))?;
    Ok(fd)
}

/// A small send buffer makes a slow link push back after a few reports, so the queue (which
/// keeps only the newest pointer position) absorbs the backlog instead of the kernel.
pub fn limit_send_buffer(fd: &OwnedFd) {
    let size: libc::c_int = 1;
    // SAFETY: the option value points to a live int; the kernel raises it to its minimum.
    let result = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&size as *const libc::c_int).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if result < 0 {
        tracing::debug!(error = %io::Error::last_os_error(), "cannot shrink the send buffer");
    }
}

pub fn send(fd: &OwnedFd, frame: &[u8]) -> io::Result<()> {
    // SAFETY: the buffer is live for the call; MSG_NOSIGNAL turns a dead link into EPIPE.
    let sent = unsafe {
        libc::send(
            fd.as_raw_fd(),
            frame.as_ptr().cast(),
            frame.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    if sent < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Reads one message; `Ok(0)` means the host closed the channel.
pub fn recv(fd: &OwnedFd, buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: the buffer is live and writable for its whole length.
        let read =
            unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        if read >= 0 {
            return Ok(read as usize);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Wakes any thread blocked on the socket; the descriptor closes when its last owner drops it.
pub fn shutdown(fd: &OwnedFd) {
    // SAFETY: shutting down a descriptor we own has no memory-safety requirements.
    unsafe {
        libc::shutdown(fd.as_raw_fd(), libc::SHUT_RDWR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_from_text_and_device_paths() {
        assert_eq!(
            parse_address("5C:DC:49:5D:14:49").unwrap(),
            [0x5c, 0xdc, 0x49, 0x5d, 0x14, 0x49]
        );
        assert!(parse_address("5C:DC:49:5D:14").is_err());
        assert!(parse_address("5C:DC:49:5D:14:4").is_err());
        assert!(parse_address("5C:DC:49:5D:14:4G").is_err());
        assert_eq!(
            address_of_device_path("/org/bluez/hci0/dev_5C_DC_49_5D_14_49").as_deref(),
            Some("5C:DC:49:5D:14:49")
        );
        assert_eq!(address_of_device_path("/org/bluez/hci0"), None);
    }

    #[test]
    fn the_socket_address_matches_the_kernel_layout() {
        assert_eq!(size_of::<SockaddrL2>(), 14);
        assert_eq!(std::mem::offset_of!(SockaddrL2, l2_bdaddr), 4);
        assert_eq!(std::mem::offset_of!(SockaddrL2, l2_bdaddr_type), 12);
    }
}
