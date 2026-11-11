// The numbering serves the phone's own driver and the host tests.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

#[cfg(target_os = "android")]
use crate::parcel::Parcel;
#[cfg(target_os = "android")]
use crate::status;
use anyhow::{Context, Result};

/// The directory every program registers with and looks things up in.
const DIRECTORY: &str = "android.os.IServiceManager";

/// What the directory numbers the methods it is asked by here. It numbers
/// its methods in the order its interface declares them, and Android 16
/// declared two lookups of its own ahead of the rest; the directory is the
/// recovery's own program, so the version the recovery was built as says
/// which numbering it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Methods {
    /// Ask for a service without waiting for one that has not registered
    /// yet.
    check_service: u32,
    /// List the names registered at the dump priorities given.
    list_services: u32,
    /// Ask whether the phone says it has a service, whether or not it is
    /// running.
    is_declared: u32,
}

/// The first version of Android whose directory declares the two lookups.
const LOOKUPS_DECLARED: u32 = 36;

/// How the directory of a recovery built as Android version `sdk` numbers
/// its methods.
fn methods_of(sdk: u32) -> Methods {
    match sdk >= LOOKUPS_DECLARED {
        true => Methods { check_service: 3, list_services: 6, is_declared: 9 },
        false => Methods { check_service: 2, list_services: 4, is_declared: 7 },
    }
}

impl Methods {
    /// The numbering the other build of the directory keeps: the one that
    /// names `listServices` where this one names something else.
    fn other(self) -> Self {
        match self.list_services {
            4 => Methods { check_service: 3, list_services: 6, is_declared: 9 },
            _ => Methods { check_service: 2, list_services: 4, is_declared: 7 },
        }
    }
}

/// The numbering the directory answered with, once it has.
static KEPT: std::sync::OnceLock<Methods> = std::sync::OnceLock::new();

/// The status libbinder answers a call with whose parcel was not opened the
/// way it opens its own: `BAD_TYPE`. The recovery's own copies of libbinder
/// and those built with the rest of Android write a different mark into the
/// header, and each reads only its own.
const BAD_TYPE: i32 = i32::MIN + 1;

/// Whether the far side turned the call away as a parcel of the wrong kind.
#[cfg(target_os = "android")]
fn wrong_kind(e: &anyhow::Error) -> bool {
    e.downcast_ref::<crate::driver::Refused>().is_some_and(|r| r.0 == BAD_TYPE)
}

/// Run a call with the mark in use. A far side built with the other kind of
/// libbinder reads only its own mark, so a parcel it turns away is sent
/// again with the other, which is then kept for every call after it.
#[cfg(target_os = "android")]
fn with_mark<T>(call: impl Fn(i32) -> Result<T>) -> Result<T> {
    match call(crate::parcel::header()) {
        Err(e) if wrong_kind(&e) => {
            let other = crate::parcel::other_header();
            let out = call(other)?;
            crate::parcel::keep_header(other);
            let _ = skyward_sys::kmsg("the far side reads a parcel opened the other way; calls are opened that way from now on");
            Ok(out)
        }
        other => other,
    }
}

/// The recovery's own version, which its property files give.
const SDK: &str = "ro.build.version.sdk";

fn methods() -> Result<Methods> {
    if let Some(kept) = KEPT.get() {
        return Ok(*kept);
    }
    let sdk = skyward_sys::prop(SDK).with_context(|| format!("the recovery says no {SDK}"))?;
    Ok(methods_of(sdk.parse().with_context(|| format!("{SDK} is {sdk:?}"))?))
}

/// Every dump priority a service can register at.
const DUMP_PRIORITY_ALL: i32 = 0b1111;

/// Ask any object which process serves it. libbinder answers this itself,
/// for every object, before any interface sees the call.
const DEBUG_PID: u32 = u32::from_be_bytes(*b"_PID");

/// Ask any object which interface it serves, which libbinder answers the
/// same way.
const INTERFACE: u32 = u32::from_be_bytes(*b"_NTF");

/// Ask a service which version of its interface it serves. Every service
/// of the newer kind answers this, numbered from the top of the range its
/// own methods are numbered from the bottom of.
const INTERFACE_VERSION: u32 = 0x00ff_ffff;

/// Which version of `interface` a service serves. It answers this only
/// once it has taken the call as one for that interface, so a service that
/// will not answer it is one that turns the call away before reading
/// anything of what it carries.
#[cfg(target_os = "android")]
pub fn version(service: &crate::driver::Handle, interface: &str) -> Result<i32> {
    let what = format!("ask which version of {interface} is served");
    with_mark(|mark| {
        let reply = service
            .transact(INTERFACE_VERSION, &Parcel::call_marked(interface, mark))
            .with_context(|| what.clone())?;
        let mut read = reply.read();
        status::ok(&mut read).with_context(|| what.clone())?;
        read.i32().with_context(|| what.clone())
    })
}

/// The service registered under `name`, or `None` while nothing has
/// registered it.
#[cfg(target_os = "android")]
pub fn find<'a>(binder: &'a crate::driver::Binder, name: &str) -> Result<Option<crate::driver::Handle<'a>>> {
    with_mark(|mark| {
        let directory = binder.directory()?;
        let mut call = Parcel::call_marked(DIRECTORY, mark);
        call.string(name);
        let reply = directory.transact(methods()?.check_service, &call).with_context(|| format!("look up {name}"))?;
        let mut read = reply.read();
        status::ok(&mut read).with_context(|| format!("look up {name}"))?;
        match read.object().with_context(|| format!("read what the directory answered for {name}"))? {
            Some(handle) => Ok(Some(crate::driver::Handle::hold(binder, handle)?)),
            None => Ok(None),
        }
    })
}

/// Every name registered with the directory, the directory's own among
/// them. Which numbering and which mark of the header the directory keeps
/// is not known before it answers, so the answer that comes back is what
/// decides both, for this and every call after it.
#[cfg(target_os = "android")]
pub fn names(binder: &crate::driver::Binder) -> Result<Vec<String>> {
    let declared = methods()?;
    let mut last = None;
    for m in [declared, declared.other()] {
        match with_mark(|mark| list(binder, m, mark)) {
            Ok(names) => {
                KEPT.set(m).ok();
                let _ = skyward_sys::kmsg(&format!("the service directory answers by method {} of its own", m.list_services));
                return Ok(names);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("a numbering was tried"))
}

#[cfg(target_os = "android")]
fn list(binder: &crate::driver::Binder, m: Methods, mark: i32) -> Result<Vec<String>> {
    let directory = binder.directory()?;
    let mut call = Parcel::call_marked(DIRECTORY, mark);
    call.i32(DUMP_PRIORITY_ALL);
    let reply = directory.transact(m.list_services, &call).context("list what has registered")?;
    let mut read = reply.read();
    status::ok(&mut read).context("list what has registered")?;
    let count = read.i32().context("read how many have registered")?;
    anyhow::ensure!(count >= 0, "the directory says {count} have registered");
    (0..count).map(|_| read.string().context("read a registered name")).collect()
}

/// The process that serves an object.
#[cfg(target_os = "android")]
pub fn pid(service: &crate::driver::Handle) -> Result<u32> {
    let reply = service.transact(DEBUG_PID, &Parcel::default()).context("ask which process serves an object")?;
    let pid = reply.read().i32().context("read which process serves an object")?;
    u32::try_from(pid).with_context(|| format!("an object says process {pid} serves it"))
}

/// The interface an object serves, by its full name.
#[cfg(target_os = "android")]
pub fn interface(service: &crate::driver::Handle) -> Result<String> {
    let reply = service.transact(INTERFACE, &Parcel::default()).context("ask which interface an object serves")?;
    reply.read().string().context("read which interface an object serves")
}

/// Whether the phone's own description says it has this service. The
/// directory turns away a hardware service the description does not name,
/// so a service that will not register is often one the directory cannot
/// see described.
#[cfg(target_os = "android")]
pub fn declared(binder: &crate::driver::Binder, name: &str) -> Result<bool> {
    with_mark(|mark| {
        let directory = binder.directory()?;
        let mut call = Parcel::call_marked(DIRECTORY, mark);
        call.string(name);
        let reply = directory.transact(methods()?.is_declared, &call).with_context(|| format!("ask whether {name} is described"))?;
        let mut read = reply.read();
        status::ok(&mut read).with_context(|| format!("ask whether {name} is described"))?;
        read.bool().with_context(|| format!("read whether {name} is described"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The positions of getService, checkService, addService, listServices,
    /// registerForNotifications, unregisterForNotifications and isDeclared
    /// in IServiceManager.aidl, and of the two lookups Android 16 put after
    /// the first and the second of them.
    #[test]
    fn the_directory_of_android_16_numbers_its_methods_two_and_four_further_on() {
        assert_eq!(methods_of(33), Methods { check_service: 2, list_services: 4, is_declared: 7 }, "the 13 Ultra's recovery");
        assert_eq!(methods_of(35), methods_of(33));
        assert_eq!(methods_of(36), Methods { check_service: 3, list_services: 6, is_declared: 9 }, "the 17 Ultra's recovery");
        assert_eq!(methods_of(37), methods_of(36));
    }

    /// The two numberings a phone's directory may keep, neither of which the
    /// version a base reports need name: an update package's own userspace
    /// says one version and carries the directory of another.
    #[test]
    fn either_numbering_leads_to_the_other() {
        assert_eq!(methods_of(33).other(), methods_of(36));
        assert_eq!(methods_of(36).other(), methods_of(33));
        assert_ne!(methods_of(33).list_services, methods_of(33).other().list_services);
    }
}
