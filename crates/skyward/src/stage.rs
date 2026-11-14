//! Where a package goes when the data partition cannot be shown.
//!
//! The partition the phone's own storage lives in is closed until it is
//! opened, and a partition a factory reset emptied is not there to open at
//! all: either way a package cannot be put where the phone usually keeps
//! one, and the pages that bring one in — the wireless transfer, a
//! computer's file manager, a file picked from internal storage — have
//! nothing to show. A piece of the phone's memory is shown there instead, so
//! that a package can still be brought in, kept until the recovery restarts,
//! and flashed.
//!
//! What is shown is memory, and a restart empties it: the page that turns it
//! on says so, and it is taken away again before the partition that should
//! be shown there is opened.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use skyward_block::mount;

/// Where the memory is shown: the name the pages, the wireless transfer and
/// a computer's file manager all use for the phone's own storage.
pub const AT: &str = skyward_crypto::data::SDCARD;

/// The folders the pages expect to find in it.
const FOLDERS: [&str; 2] = ["Download", "Pictures/Screenshots"];

/// How much memory it may take: three quarters of what the phone has to
/// spare, and never less than this, a package being a few gigabytes. The
/// number is a ceiling, not an asking: pages are taken as they are written.
const LEAST: u64 = 8 << 30;

/// Whether it is the memory that is shown at [`AT`] now.
static STAGED: AtomicBool = AtomicBool::new(false);

/// The memory shown at [`AT`], where it was not before: `true` where it was
/// shown now, `false` where the partition that should be there is open, or
/// something else is already shown there.
pub fn make() -> Result<bool> {
    if STAGED.load(Ordering::Relaxed) {
        return Ok(false);
    }
    if mount::is_mount_point(Path::new(skyward_crypto::data::MOUNT))? {
        return Ok(false);
    }
    fs::create_dir_all(AT).with_context(|| format!("make {AT}"))?;
    if mount::is_mount_point(Path::new(AT))? {
        return Ok(false);
    }
    let room = room();
    let options = ["mode=0777".to_string(), "nosuid".to_string(), "nodev".to_string(), format!("size={room}")];
    mount::mount(Path::new("tmpfs"), Path::new(AT), "tmpfs", &options)
        .with_context(|| format!("show {} of the phone's memory at {AT}", spoken(room)))?;
    for folder in FOLDERS {
        fs::create_dir_all(Path::new(AT).join(folder)).with_context(|| format!("make {}/{folder}", AT))?;
    }
    STAGED.store(true, Ordering::Relaxed);
    Ok(true)
}

/// Take the memory away again, before the partition that should be shown
/// there is opened: a partition cannot be mounted over memory the page
/// still holds a name for.
pub fn take() -> Result<()> {
    if !STAGED.swap(false, Ordering::Relaxed) {
        return Ok(());
    }
    if mount::is_mount_point(Path::new(AT))? {
        // Taken out of the tree at once: a file the pages still hold open
        // would keep a plain unmount from finishing, and the memory is of no
        // use once the partition it stood in for is opened.
        mount::umount_detach(Path::new(AT)).with_context(|| format!("take the memory away from {AT}"))?;
    }
    Ok(())
}

/// How much of the phone's memory can be spared: three quarters of what is
/// free, and never less than [`LEAST`].
fn room() -> u64 {
    let free = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let rest = line.strip_prefix("MemAvailable:")?;
                let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
                Some(kib * 1024)
            })
        })
        .unwrap_or(LEAST * 2);
    (free / 4 * 3).max(LEAST)
}

/// `bytes` as the pages say sizes: whole gigabytes where it makes one.
fn spoken(bytes: u64) -> String {
    if bytes.is_multiple_of(1 << 30) {
        format!("{} GB", bytes >> 30)
    } else {
        format!("{} MB", bytes >> 20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_given_in_the_whole_unit_it_makes() {
        assert_eq!(spoken(4 << 30), "4 GB");
        assert_eq!(spoken(512 << 20), "512 MB");
    }

    #[test]
    fn the_room_spared_is_never_less_than_a_package() {
        assert!(room() >= LEAST, "{} is less than a package", room());
    }
}
