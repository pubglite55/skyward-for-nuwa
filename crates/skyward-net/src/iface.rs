//! A network interface's own settings: up or down, its address, and the
//! default route through it, set the way ifconfig and route set them.

use anyhow::{Context, Result};
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// The kernel's `struct rtentry`, which the C library of a phone does not
/// describe.
#[repr(C)]
struct Route {
    pad1: libc::c_ulong,
    dst: libc::sockaddr,
    gateway: libc::sockaddr,
    genmask: libc::sockaddr,
    flags: libc::c_ushort,
    pad2: libc::c_short,
    pad3: libc::c_ulong,
    pad4: *mut libc::c_void,
    /// The route's metric plus one.
    metric: libc::c_short,
    dev: *mut libc::c_char,
    mtu: libc::c_ulong,
    window: libc::c_ulong,
    irtt: libc::c_ushort,
}

const RTF_UP: libc::c_ushort = 0x1;
const RTF_GATEWAY: libc::c_ushort = 0x2;

fn socket() -> Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("make a socket to set an interface with");
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn request(interface: &str) -> Result<libc::ifreq> {
    let mut r: libc::ifreq = unsafe { std::mem::zeroed() };
    anyhow::ensure!(interface.len() < r.ifr_name.len(), "{interface} is too long a name for an interface");
    for (to, from) in r.ifr_name.iter_mut().zip(interface.bytes()) {
        *to = from as libc::c_char;
    }
    Ok(r)
}

fn ioctl(fd: &OwnedFd, what: libc::c_int, arg: *mut libc::c_void, doing: &str) -> Result<()> {
    if unsafe { libc::ioctl(fd.as_raw_fd(), what, arg) } != 0 {
        return Err(std::io::Error::last_os_error()).context(doing.to_string());
    }
    Ok(())
}

fn sockaddr(address: Ipv4Addr) -> libc::sockaddr {
    let inet = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: 0,
        sin_addr: libc::in_addr { s_addr: u32::from_ne_bytes(address.octets()) },
        sin_zero: [0; 8],
    };
    unsafe { std::mem::transmute::<libc::sockaddr_in, libc::sockaddr>(inet) }
}

/// The interface's hardware address.
pub fn hardware_address(interface: &str) -> Result<[u8; 6]> {
    let fd = socket()?;
    let mut r = request(interface)?;
    ioctl(&fd, libc::SIOCGIFHWADDR as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &format!("read the hardware address of {interface}"))?;
    let data = unsafe { r.ifr_ifru.ifru_hwaddr.sa_data };
    let mut mac = [0u8; 6];
    // A C char is signed on some phones and not on others.
    for (to, from) in mac.iter_mut().zip(data) {
        *to = from.to_ne_bytes()[0];
    }
    Ok(mac)
}

/// The address the interface has, where it has one: a phone that has not
/// joined a network yet has none.
pub fn address(interface: &str) -> Result<Option<Ipv4Addr>> {
    let fd = socket()?;
    let mut r = request(interface)?;
    let doing = format!("read the address of {interface}");
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCGIFADDR as libc::c_int, (&mut r as *mut libc::ifreq).cast::<libc::c_void>()) } != 0 {
        let e = std::io::Error::last_os_error();
        // The kernel says this of an interface that is there and has no
        // address on it, which is where a phone that has joined nothing
        // stands, rather than of one that is not there at all.
        if e.raw_os_error() == Some(libc::EADDRNOTAVAIL) {
            return Ok(None);
        }
        return Err(e).with_context(|| doing);
    }
    let inet = unsafe { std::mem::transmute::<libc::sockaddr, libc::sockaddr_in>(r.ifr_ifru.ifru_addr) };
    // The bytes are the address's own, as they are on the wire and as
    // `sockaddr` above puts them there.
    Ok(Some(Ipv4Addr::from(inet.sin_addr.s_addr.to_ne_bytes())))
}

/// Bring the interface up, or take it down.
pub fn set_up(interface: &str, up: bool) -> Result<()> {
    let fd = socket()?;
    let mut r = request(interface)?;
    ioctl(&fd, libc::SIOCGIFFLAGS as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &format!("read how {interface} stands"))?;
    unsafe {
        if up {
            r.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
        } else {
            r.ifr_ifru.ifru_flags &= !(libc::IFF_UP as libc::c_short);
        }
    }
    let doing = if up { format!("bring {interface} up") } else { format!("take {interface} down") };
    ioctl(&fd, libc::SIOCSIFFLAGS as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &doing)
}

/// Give the interface `address` in a network of `mask`; what is sent to
/// any other goes through `router`.
pub fn configure(interface: &str, address: Ipv4Addr, mask: Ipv4Addr, router: Option<Ipv4Addr>) -> Result<()> {
    let fd = socket()?;
    let mut r = request(interface)?;
    r.ifr_ifru.ifru_addr = sockaddr(address);
    ioctl(&fd, libc::SIOCSIFADDR as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &format!("give {interface} the address {address}"))?;
    r.ifr_ifru.ifru_netmask = sockaddr(mask);
    ioctl(&fd, libc::SIOCSIFNETMASK as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &format!("give {interface} the mask {mask}"))?;
    let Some(router) = router else { return Ok(()) };
    let mut name: Vec<libc::c_char> = interface.bytes().map(|b| b as libc::c_char).chain([0]).collect();
    let mut route = Route {
        pad1: 0,
        dst: sockaddr(Ipv4Addr::UNSPECIFIED),
        gateway: sockaddr(router),
        genmask: sockaddr(Ipv4Addr::UNSPECIFIED),
        flags: RTF_UP | RTF_GATEWAY,
        pad2: 0,
        pad3: 0,
        pad4: std::ptr::null_mut(),
        metric: 1,
        dev: name.as_mut_ptr(),
        mtu: 0,
        window: 0,
        irtt: 0,
    };
    // A default route left from an earlier address goes first; the kernel
    // keeps only one.
    if unsafe { libc::ioctl(fd.as_raw_fd(), libc::SIOCDELRT as libc::c_int, (&mut route as *mut Route).cast::<libc::c_void>()) } != 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(e).with_context(|| format!("take away the default route through {interface}"));
        }
    }
    ioctl(&fd, libc::SIOCADDRT as libc::c_int, (&mut route as *mut Route).cast(), &format!("route through {router} on {interface}"))
}

/// Take the interface's address away, and the routes through it with it.
pub fn unconfigure(interface: &str) -> Result<()> {
    let fd = socket()?;
    let mut r = request(interface)?;
    r.ifr_ifru.ifru_addr = sockaddr(Ipv4Addr::UNSPECIFIED);
    ioctl(&fd, libc::SIOCSIFADDR as libc::c_int, (&mut r as *mut libc::ifreq).cast(), &format!("take the address of {interface} away"))
}
