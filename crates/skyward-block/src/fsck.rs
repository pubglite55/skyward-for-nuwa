//! Checking a filesystem: before it is mounted writable, the way a phone
//! checks its own as it starts, and the whole of it when a page asks for it
//! to be repaired.

use crate::tools::{self, Ran, Tool};
use anyhow::{bail, Result};
use std::path::Path;

/// The programs that check each filesystem: the recovery's own, or the
/// installed system's where the recovery carries none.
const FSCK_F2FS: &str = "fsck.f2fs";
pub const E2FSCK: &str = "e2fsck";

/// What fs_mgr passes to check f2fs before it mounts it: fix what the
/// kernel marked as needing it, and keep as many hash entries in the cache
/// as fs_mgr asks for. The whole check that the phone's own starts make
/// once a month, timed by a clock the filesystem keeps, is left to them:
/// made here, it would move their mark.
const CHECK_F2FS: [&str; 5] = ["-a", "--no-kernel-check", "-c", "10000", "--debug-cache"];

/// What repairs f2fs: every part of the partition checked, and all that is
/// found fixed.
const REPAIR_F2FS: [&str; 6] = ["-f", "-y", "--no-kernel-check", "-c", "10000", "--debug-cache"];

/// What repairs ext4: the whole filesystem checked, and every fix made
/// without asking.
const REPAIR_EXT: [&str; 2] = ["-f", "-y"];

/// What gives each file in an ext4 filesystem blocks of its own again.
const UNSHARE_EXT: [&str; 3] = ["-p", "-E", "unshare_blocks"];

/// The exit statuses that mean the filesystem is sound now, which e2fsck and
/// fsck.f2fs both end with as fsck does: it was already, or what was wrong
/// has been fixed, with or without a restart to follow.
const CLEAN: i32 = 0;
const FIXED: [i32; 2] = [1, 2];

/// What checks ext4 before it is mounted: the preen check, which fixes what
/// it can without asking, as fs_mgr runs it.
const CHECK_EXT: [&str; 1] = ["-p"];

/// How a check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The program's exit status: 0 when it found nothing to fix.
    pub status: i32,
    /// What it printed.
    pub said: String,
}

/// Check `device`, which holds `filesystem`, as the phone's fs_mgr does
/// before it mounts one, and fix what the kernel marked as needing it.
pub fn check(device: &Path, filesystem: &str) -> Result<Checked> {
    let (program, options): (&str, &[&str]) = match filesystem {
        "f2fs" => (FSCK_F2FS, &CHECK_F2FS[..]),
        "ext4" | "ext3" | "ext2" => (E2FSCK, &CHECK_EXT[..]),
        other => bail!("{} holds {other}, which the recovery does not check before writing to it", device.display()),
    };
    let ran = tools::find(program)?.run(&arguments(options, device)?)?;
    Ok(Checked { status: ran.status, said: ran.said })
}

/// How a repair left the filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixed {
    /// Nothing was wrong with it.
    Clean,
    /// What was wrong has been fixed.
    Fixed,
    /// It is still not sound.
    Failed,
}

/// Check the whole of `device`, which holds `filesystem`, and fix what is
/// wrong, the way the programs of each filesystem repair one; nothing is to
/// be mounted from it. Gives how it was left and what the program said.
pub fn repair(device: &Path, filesystem: &str) -> Result<(Fixed, Ran)> {
    let ran = match filesystem {
        "f2fs" => tools::find(FSCK_F2FS)?.run(&arguments(&REPAIR_F2FS, device)?)?,
        "ext4" | "ext3" | "ext2" => tools::find(E2FSCK)?.run(&arguments(&REPAIR_EXT, device)?)?,
        other => bail!("{} holds {other}, which the recovery does not repair", device.display()),
    };
    Ok((left_by(ran.status), ran))
}

/// Undo the sharing of blocks in the ext4 filesystem on `device` with
/// `e2fsck`, so every file holds blocks of its own again and the filesystem
/// can be written to.
pub fn unshare_blocks(e2fsck: &Tool, device: &Path) -> Result<(Fixed, Ran)> {
    let ran = e2fsck.run(&arguments(&UNSHARE_EXT, device)?)?;
    Ok((left_by(ran.status), ran))
}

/// What a check's exit status says of the filesystem it left.
fn left_by(status: i32) -> Fixed {
    match status {
        CLEAN => Fixed::Clean,
        s if FIXED.contains(&s) => Fixed::Fixed,
        _ => Fixed::Failed,
    }
}

/// The options with the device after them, as every one of the programs
/// takes it.
fn arguments(options: &[&str], device: &Path) -> Result<Vec<String>> {
    let mut args: Vec<String> = options.iter().map(|o| o.to_string()).collect();
    args.push(tools::path(device)?);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f2fs_is_checked_as_the_phone_checks_it_less_the_monthly_whole_check() {
        let args = arguments(&CHECK_F2FS, Path::new("/dev/block/dm-8")).unwrap();
        assert_eq!(args, ["-a", "--no-kernel-check", "-c", "10000", "--debug-cache", "/dev/block/dm-8"]);
    }

    #[test]
    fn a_repair_goes_through_the_whole_partition_and_fixes_without_asking() {
        let f2fs = arguments(&REPAIR_F2FS, Path::new("/dev/block/dm-8")).unwrap();
        assert_eq!(f2fs, ["-f", "-y", "--no-kernel-check", "-c", "10000", "--debug-cache", "/dev/block/dm-8"]);
        let ext = arguments(&REPAIR_EXT, Path::new("/dev/block/dm-9")).unwrap();
        assert_eq!(ext, ["-f", "-y", "/dev/block/dm-9"]);
        let unshare = arguments(&UNSHARE_EXT, Path::new("/dev/block/dm-3")).unwrap();
        assert_eq!(unshare, ["-p", "-E", "unshare_blocks", "/dev/block/dm-3"]);
    }

    #[test]
    fn ext4_is_checked_before_mounting_as_the_phone_checks_it() {
        let args = arguments(&CHECK_EXT, Path::new("/dev/block/dm-8")).unwrap();
        assert_eq!(args, ["-p", "/dev/block/dm-8"]);
    }

    #[test]
    fn a_filesystem_the_recovery_does_not_check_or_repair_is_refused() {
        assert!(check(Path::new("/dev/x"), "erofs").is_err(), "no checker for it is carried");
        assert!(repair(Path::new("/dev/x"), "erofs").is_err());
    }

    #[test]
    fn what_a_check_leaves_is_read_from_its_status() {
        assert_eq!(left_by(0), Fixed::Clean);
        assert_eq!(left_by(1), Fixed::Fixed);
        assert_eq!(left_by(2), Fixed::Fixed, "fixed, with a restart to follow");
        assert_eq!(left_by(4), Fixed::Failed, "errors left uncorrected");
        assert_eq!(left_by(8), Fixed::Failed);
    }
}
