use crate::parcel::{Payload, Reader};
use anyhow::{bail, Context, Result};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;

/// The driver every program on the phone reaches its neighbours through.
const DEVICE: &str = "/dev/binder";

/// The second driver, which the phone's hardware services talk over. Older
/// phones have the node itself; newer ones have the driver's own filesystem
/// and the node inside it.
const HARDWARE: [&str; 2] = ["/dev/hwbinder", "/dev/binderfs/hwbinder"];

/// The layout this crate speaks. The driver refuses a program that knows a
/// different one.
const PROTOCOL: i32 = 8;

/// Room the driver fills with what arrives for us, the size libbinder asks
/// for: a megabyte less the two pages the driver keeps for itself.
const MAP_BYTES: usize = 1024 * 1024 - 8192;

/// Build a driver request number the way the kernel's headers do.
const fn ioc(dir: u32, kind: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((kind as u32) << 8) | nr as u32
}

const WRITE: u32 = 1;
const READ: u32 = 2;

const BINDER_WRITE_READ: u32 = ioc(WRITE | READ, b'b', 1, size_of::<WriteRead>());
const BINDER_SET_MAX_THREADS: u32 = ioc(WRITE, b'b', 5, 4);
const BINDER_VERSION: u32 = ioc(WRITE | READ, b'b', 9, 4);

const BC_TRANSACTION: u32 = ioc(WRITE, b'c', 0, size_of::<TransactionData>());
const BC_TRANSACTION_SG: u32 = ioc(WRITE, b'c', 17, size_of::<TransactionData>() + 8);
const BC_FREE_BUFFER: u32 = ioc(WRITE, b'c', 3, 8);
const BC_ACQUIRE: u32 = ioc(WRITE, b'c', 5, 4);
const BC_RELEASE: u32 = ioc(WRITE, b'c', 6, 4);

const BR_ERROR: u32 = ioc(READ, b'r', 0, 4);
const BR_REPLY: u32 = ioc(READ, b'r', 3, size_of::<TransactionData>());
const BR_DEAD_REPLY: u32 = ioc(0, b'r', 5, 0);
const BR_TRANSACTION_COMPLETE: u32 = ioc(0, b'r', 6, 0);
const BR_INCREFS: u32 = ioc(READ, b'r', 7, 16);
const BR_ACQUIRE: u32 = ioc(READ, b'r', 8, 16);
const BR_RELEASE: u32 = ioc(READ, b'r', 9, 16);
const BR_DECREFS: u32 = ioc(READ, b'r', 10, 16);
const BR_NOOP: u32 = ioc(0, b'r', 12, 0);
const BR_SPAWN_LOOPER: u32 = ioc(0, b'r', 13, 0);
const BR_FAILED_REPLY: u32 = ioc(0, b'r', 17, 0);
const BR_FROZEN_REPLY: u32 = ioc(0, b'r', 18, 0);

/// The call takes no file descriptors but is willing to receive them.
const TF_ACCEPT_FDS: u32 = 0x10;
/// The reply carries a bare error number in place of a payload.
const TF_STATUS_CODE: u32 = 0x08;

#[repr(C)]
#[derive(Default)]
struct WriteRead {
    write_size: u64,
    write_consumed: u64,
    write_buffer: u64,
    read_size: u64,
    read_consumed: u64,
    read_buffer: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct TransactionData {
    target: u64,
    cookie: u64,
    code: u32,
    flags: u32,
    sender_pid: i32,
    sender_euid: u32,
    data_size: u64,
    offsets_size: u64,
    buffer: u64,
    offsets: u64,
}

fn bytes_of<T>(v: &T) -> &[u8] {
    // Every field is a plain integer, so the struct is its own bytes.
    unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) }
}

/// A call the far side turned away with a status of its own. The numbers
/// are libbinder's, and they say what the far side found: a parcel it was
/// not built to read, an argument it would not take, a service it does not
/// have. What the number means is read from it where the call is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused(pub i32);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the call failed in the driver with error {}", self.0)
    }
}

impl std::error::Error for Refused {}

/// This program's end of the driver.
pub struct Binder {
    file: File,
    map: *mut libc::c_void,
}

impl Binder {
    pub fn open() -> Result<Binder> {
        Binder::on(Path::new(DEVICE))
    }

    /// The driver the phone's hardware services talk over, which is a
    /// second one of the same kind.
    pub fn hardware() -> Result<Binder> {
        let node = HARDWARE
            .iter()
            .map(Path::new)
            .find(|p| p.exists())
            .with_context(|| format!("none of {} is here, so no hardware service can be reached", HARDWARE.join(" or ")))?;
        Binder::on(node)
    }

    fn on(node: &Path) -> Result<Binder> {
        let file = File::options().read(true).write(true).open(node).with_context(|| format!("open {}", node.display()))?;
        let mut version: i32 = 0;
        unsafe { call(&file, BINDER_VERSION, &mut version as *mut i32 as *mut libc::c_void) }.context("ask the driver its protocol")?;
        if version != PROTOCOL {
            bail!("the driver speaks protocol {version}, Skyward speaks {PROTOCOL}");
        }
        // Skyward only ever calls out, so the driver never needs us to take
        // a call and never has to ask for a thread to take it on.
        let mut threads: u32 = 0;
        unsafe { call(&file, BINDER_SET_MAX_THREADS, &mut threads as *mut u32 as *mut libc::c_void) }.context("give up taking calls")?;
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), MAP_BYTES, libc::PROT_READ, libc::MAP_PRIVATE | libc::MAP_NORESERVE, file.as_raw_fd(), 0)
        };
        if map == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error()).context("map the driver's replies");
        }
        Ok(Binder { file, map })
    }

    /// The service directory, which every program reaches at handle 0. The
    /// driver carries a call to whichever process holds that place without
    /// being given a share of it first, so this takes none and gives none
    /// back: taking one and giving it back on every look-up would have the
    /// directory's own object counted up and down from outside while it is
    /// still starting.
    pub fn directory(&self) -> Result<Handle<'_>> {
        Ok(Handle { binder: self, handle: 0, held: false })
    }

    /// Call method `code` on `handle` and wait for what it answers.
    pub fn transact(&self, handle: u32, code: u32, data: &impl Payload) -> Result<Reply<'_>> {
        let payload = data.values();
        let addresses = data.addresses();
        let td = TransactionData {
            target: handle as u64,
            code,
            flags: TF_ACCEPT_FDS,
            data_size: payload.len() as u64,
            offsets_size: (addresses.len() * 8) as u64,
            buffer: payload.as_ptr() as u64,
            offsets: addresses.as_ptr() as u64,
            ..TransactionData::default()
        };
        // A call that hands over addresses is sent the way that tells the
        // driver how much room to set aside for what they point at.
        let mut write = match addresses.is_empty() {
            true => BC_TRANSACTION.to_ne_bytes().to_vec(),
            false => BC_TRANSACTION_SG.to_ne_bytes().to_vec(),
        };
        write.extend_from_slice(bytes_of(&td));
        if !addresses.is_empty() {
            write.extend_from_slice(&data.pointed().to_ne_bytes());
        }
        // The driver answers a call with a handful of short notices; what
        // the other side sent lands in the mapped room, not here.
        let mut read = [0u8; 256];
        let mut sent = false;
        loop {
            let mut bwr = WriteRead {
                write_size: if sent { 0 } else { write.len() as u64 },
                write_buffer: write.as_ptr() as u64,
                read_size: read.len() as u64,
                read_buffer: read.as_mut_ptr() as u64,
                ..WriteRead::default()
            };
            unsafe { call(&self.file, BINDER_WRITE_READ, &mut bwr as *mut WriteRead as *mut libc::c_void) }.context("send a call")?;
            sent = true;
            if let Some(reply) = self.answer(&read[..bwr.read_consumed as usize])? {
                return Ok(reply);
            }
        }
    }

    /// What the driver said back, and the reply among it if it has come.
    fn answer(&self, mut said: &[u8]) -> Result<Option<Reply<'_>>> {
        while said.len() >= 4 {
            let note = u32::from_ne_bytes(said[..4].try_into().expect("four bytes"));
            said = &said[4..];
            let mut skip = |n: usize| {
                let (head, rest) = said.split_at(n.min(said.len()));
                said = rest;
                head
            };
            match note {
                BR_NOOP | BR_TRANSACTION_COMPLETE | BR_SPAWN_LOOPER => {}
                BR_INCREFS | BR_ACQUIRE | BR_RELEASE | BR_DECREFS => {
                    skip(16);
                }
                BR_REPLY => {
                    let raw = skip(size_of::<TransactionData>());
                    let td: TransactionData = unsafe { std::ptr::read_unaligned(raw.as_ptr().cast()) };
                    if td.flags & TF_STATUS_CODE != 0 {
                        let code = unsafe { std::ptr::read_unaligned(td.buffer as *const i32) };
                        self.free(td.buffer)?;
                        return Err(Refused(code).into());
                    }
                    let data = unsafe { std::slice::from_raw_parts(td.buffer as *const u8, td.data_size as usize) };
                    return Ok(Some(Reply { binder: self, buffer: td.buffer, data }));
                }
                BR_DEAD_REPLY => bail!("the other side is gone"),
                BR_FAILED_REPLY => bail!("the driver refused the call"),
                BR_FROZEN_REPLY => bail!("the other side is frozen"),
                BR_ERROR => {
                    let raw = skip(4);
                    let code = i32::from_ne_bytes(raw.try_into().unwrap_or([0; 4]));
                    bail!("the driver reported error {code}");
                }
                other => bail!("the driver said {other:#x}, which Skyward does not know"),
            }
        }
        Ok(None)
    }

    /// Tell the driver something that needs no answer.
    fn tell(&self, note: u32, payload: &[u8]) -> Result<()> {
        let mut write = note.to_ne_bytes().to_vec();
        write.extend_from_slice(payload);
        let mut bwr = WriteRead {
            write_size: write.len() as u64,
            write_buffer: write.as_ptr() as u64,
            ..WriteRead::default()
        };
        unsafe { call(&self.file, BINDER_WRITE_READ, &mut bwr as *mut WriteRead as *mut libc::c_void) }.context("tell the driver")
    }

    fn free(&self, buffer: u64) -> Result<()> {
        self.tell(BC_FREE_BUFFER, &buffer.to_ne_bytes())
    }
}

impl Drop for Binder {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.map, MAP_BYTES) };
    }
}

unsafe fn call(file: &File, request: u32, arg: *mut libc::c_void) -> Result<()> {
    // The driver answers a signal with EINTR and expects the call again.
    loop {
        if libc::ioctl(file.as_raw_fd(), request as libc::Ioctl, arg) >= 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err.into());
        }
    }
}

/// An object in another process, held open until this goes out of scope.
pub struct Handle<'a> {
    binder: &'a Binder,
    handle: u32,
    /// Whether a share of the object was taken, and so has to be given
    /// back.
    held: bool,
}

impl<'a> Handle<'a> {
    /// Take a share of an object the driver has just named, so it outlives
    /// the reply that carried it.
    pub fn hold(binder: &'a Binder, handle: u32) -> Result<Handle<'a>> {
        binder.tell(BC_ACQUIRE, &handle.to_ne_bytes()).context("hold an object open")?;
        Ok(Handle { binder, handle, held: true })
    }

    pub fn transact(&self, code: u32, data: &impl Payload) -> Result<Reply<'a>> {
        self.binder.transact(self.handle, code, data)
    }
}

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        if self.held {
            let _ = self.binder.tell(BC_RELEASE, &self.handle.to_ne_bytes());
        }
    }
}

/// What the other side answered, in the driver's own room until it is read.
pub struct Reply<'a> {
    binder: &'a Binder,
    buffer: u64,
    data: &'a [u8],
}

impl<'a> Reply<'a> {
    pub fn read(&self) -> Reader<'_> {
        Reader::new(self.data)
    }

    /// The reply as it lies in the driver's room, for a layout the plain
    /// reader does not know.
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    pub fn binder(&self) -> &'a Binder {
        self.binder
    }
}

impl Drop for Reply<'_> {
    fn drop(&mut self) {
        let _ = self.binder.free(self.buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_numbers_match_the_kernels_headers() {
        assert_eq!(BINDER_WRITE_READ, 0xc030_6201);
        assert_eq!(BINDER_VERSION, 0xc004_6209);
        assert_eq!(BINDER_SET_MAX_THREADS, 0x4004_6205);
        assert_eq!(BC_TRANSACTION, 0x4040_6300);
        assert_eq!(BC_TRANSACTION_SG, 0x4048_6311);
        assert_eq!(BC_FREE_BUFFER, 0x4008_6303);
        assert_eq!(BR_REPLY, 0x8040_7203);
        assert_eq!(BR_NOOP, 0x0000_720c);
        assert_eq!(BR_DEAD_REPLY, 0x0000_7205);
    }

    #[test]
    fn the_structures_are_the_size_the_driver_reads() {
        assert_eq!(size_of::<WriteRead>(), 48);
        assert_eq!(size_of::<TransactionData>(), 64);
    }
}
