//! Erasing a partition, and making afresh the filesystem a phone's table
//! names for one, the way a phone's own recovery formats it.

use crate::tools;
use anyhow::{bail, ensure, Context, Result};
use std::path::Path;

/// The programs a recovery ramdisk carries for making the two filesystems.
const MAKE_F2FS: &str = "make_f2fs";
const MAKE_EXT4: &str = "mke2fs";

/// How much of a partition's start is written over with zeros once it is
/// discarded: well past where any filesystem a phone uses keeps its
/// superblocks.
#[cfg(target_os = "android")]
const ZEROED: u64 = 1 << 20;

/// Discard all a partition holds, as a phone's own recovery erases one
/// that the system opens through metadata encryption, then write zeros
/// over its start. A partition that starts with zeros is one the system
/// formats afresh, under a new key, as it mounts it; what discarded blocks
/// read back as is the storage's own choice, so the zeros are written.
#[cfg(target_os = "android")]
pub fn discard(device: &Path) -> Result<()> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::FileExt;
    use std::os::unix::io::AsRawFd;
    // _IO(0x12, 125) and _IO(0x12, 119); each takes the start and length.
    const BLKSECDISCARD: libc::c_int = 0x127d;
    const BLKDISCARD: libc::c_int = 0x1277;
    let size = crate::size_bytes(device)?;
    let f = OpenOptions::new().write(true).open(device).with_context(|| format!("open {}", device.display()))?;
    let range = [0u64, size];
    // The secure kind leaves nothing behind in spare blocks either; not
    // all storage offers it.
    if unsafe { libc::ioctl(f.as_raw_fd(), BLKSECDISCARD, range.as_ptr()) } != 0 {
        let secure = std::io::Error::last_os_error();
        if unsafe { libc::ioctl(f.as_raw_fd(), BLKDISCARD, range.as_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("discard {}, after a secure discard failed with {secure}", device.display()));
        }
    }
    let zeros = vec![0u8; ZEROED.min(size) as usize];
    f.write_all_at(&zeros, 0).with_context(|| format!("write zeros over the start of {}", device.display()))?;
    f.sync_all().with_context(|| format!("sync {}", device.display()))
}

#[cfg(not(target_os = "android"))]
pub fn discard(device: &Path) -> Result<()> {
    bail!("this host has no block device {} to discard", device.display())
}

/// The size of ext4's blocks and of f2fs's where make_f2fs is told no
/// other, and the unit both programs count a length in.
const BLOCK: u64 = 4096;

/// Make the filesystem `filesystem` on `device`, as a phone's own recovery
/// makes it: `extras` are what the phone's table says of it beyond how to
/// mount it, and `casefold` has names in it looked up without regard to
/// case, as Android has a data partition made whose shared storage works
/// so.
pub fn make(device: &Path, filesystem: &str, extras: &[String], casefold: bool) -> Result<()> {
    let (program, mut args) = command(device, filesystem, extras, casefold, page_size())?;
    args.push(tools::path(device)?);
    if let Some(length) = length(extras, || crate::size_bytes(device))? {
        // mke2fs counts it in its blocks, Android's make_f2fs in sectors it
        // takes as just as large.
        args.push((length / BLOCK).to_string());
    }
    let tool = tools::find(program)?;
    let ran = tool.run(&args)?;
    ensure!(ran.status == 0, "{} ended with {}: {}", tool.describe(&args), ran.status, ran.said.trim());
    Ok(())
}

/// The program that makes a filesystem and its options, as a phone's own
/// recovery runs it: f2fs with Android's defaults and quota by project,
/// case folded where asked, compression where the table asks for it, and
/// blocks as large as the kernel's pages; ext4 with 4 KiB blocks, inodes
/// wide enough for project quota, checksums over its metadata where the
/// table asks for them, and laid along the storage's own blocks where the
/// table gives their sizes.
fn command(device: &Path, filesystem: &str, extras: &[String], casefold: bool, page: u64) -> Result<(&'static str, Vec<String>)> {
    let has = |flag: &str| extras.iter().any(|e| e == flag);
    let mut args = Vec::new();
    match filesystem {
        "f2fs" => {
            // `-f` makes it over what is there: a partition being given a
            // filesystem usually holds one already, and make_f2fs refuses
            // that with "In use by the system!" without it.
            args.extend(["-f", "-g", "android", "-O", "project_quota,extra_attr"].map(String::from));
            if casefold {
                args.extend(["-O", "casefold", "-C", "utf8"].map(String::from));
            }
            if has("fscompress") {
                args.extend(["-O", "compression", "-O", "extra_attr"].map(String::from));
            }
            // f2fs's blocks are the size of the kernel's pages. The older
            // make_f2fs some recoveries carry is told no size, and makes
            // 4 KiB blocks.
            if page != BLOCK {
                args.extend(["-b".to_string(), page.to_string()]);
            }
            Ok((MAKE_F2FS, args))
        }
        "ext4" => {
            args.extend(["-F", "-t", "ext4", "-b", "4096", "-I", "512"].map(String::from));
            if has("metadata_csum") {
                args.extend(["-O", "metadata_csum", "-O", "64bit", "-O", "extent"].map(String::from));
            }
            if let Some(layout) = layout(extras)? {
                args.extend(["-E".to_string(), layout]);
            }
            Ok((MAKE_EXT4, args))
        }
        other => bail!("{} is to hold {other}, which the recovery cannot make", device.display()),
    }
}

/// The length the phone's table gives the filesystem on a partition, in
/// bytes: a positive one as it stands, a negative one as what is left once
/// that much of the partition's end is kept back. None where the table
/// gives none, and the filesystem takes the whole partition; `size` gives
/// the partition's own.
fn length(extras: &[String], size: impl FnOnce() -> Result<u64>) -> Result<Option<u64>> {
    let Some(given) = extras.iter().find_map(|e| e.strip_prefix("length=")) else { return Ok(None) };
    let given: i64 = given.parse().with_context(|| format!("the table's length={given} is no number of bytes"))?;
    if given >= 0 {
        return Ok((given > 0).then_some(given as u64));
    }
    let (size, kept) = (size()?, given.unsigned_abs());
    ensure!(kept < size, "the table keeps back {kept} bytes of a partition of {size}");
    Ok(Some(size - kept))
}

/// How ext4 is laid along the storage's own blocks, where the table gives
/// both the size the storage is written in and the size it is erased in,
/// each a power of two of at least 4 KiB: a stride of the first or of
/// 8 KiB, whichever is larger, and a stripe as wide as the second, both
/// counted in the filesystem's blocks.
fn layout(extras: &[String]) -> Result<Option<String>> {
    let size = |name: &str| -> Result<Option<u64>> {
        let prefix = format!("{name}=");
        let Some(given) = extras.iter().find_map(|e| e.strip_prefix(prefix.as_str())) else { return Ok(None) };
        let n: u64 = given.parse().with_context(|| format!("the table's {name}={given} is no number of bytes"))?;
        ensure!(n >= BLOCK && n.is_power_of_two(), "the table's {name}={given} is no power of two of at least {BLOCK}");
        Ok(Some(n))
    };
    Ok(match (size("logicalblk")?, size("eraseblk")?) {
        (Some(written), Some(erased)) => Some(format!("stride={},stripe-width={}", written.max(8192) / BLOCK, erased / BLOCK)),
        _ => None,
    })
}

/// The size of the kernel's memory pages.
fn page_size() -> u64 {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as u64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extras(list: &[&str]) -> Vec<String> {
        list.iter().map(|e| e.to_string()).collect()
    }

    #[test]
    fn f2fs_is_made_with_androids_defaults_and_project_quota() {
        let c = command(Path::new("/dev/block/by-name/metadata"), "f2fs", &[], false, 4096).unwrap();
        assert_eq!(c, ("make_f2fs", extras(&["-g", "android", "-O", "project_quota,extra_attr"])));
    }

    #[test]
    fn f2fs_folds_case_where_asked_and_compresses_where_the_table_asks() {
        let data = command(Path::new("/dev/x"), "f2fs", &extras(&["check", "fscompress"]), true, 4096).unwrap();
        let wanted = ["-O", "casefold", "-C", "utf8", "-O", "compression", "-O", "extra_attr"];
        assert_eq!(&data.1[4..], extras(&wanted));
    }

    #[test]
    fn f2fs_blocks_are_as_large_as_the_kernels_pages() {
        let big = command(Path::new("/dev/x"), "f2fs", &[], false, 16384).unwrap();
        assert_eq!(&big.1[4..], extras(&["-b", "16384"]));
    }

    #[test]
    fn ext4_takes_metadata_checksums_where_the_table_asks() {
        let plain = command(Path::new("/dev/x"), "ext4", &extras(&["check"]), true, 4096).unwrap();
        assert_eq!(plain, ("mke2fs", extras(&["-F", "-t", "ext4", "-b", "4096", "-I", "512"])), "case folding is f2fs's");
        let summed = command(Path::new("/dev/x"), "ext4", &extras(&["metadata_csum"]), false, 4096).unwrap();
        assert_eq!(&summed.1[7..], extras(&["-O", "metadata_csum", "-O", "64bit", "-O", "extent"]));
    }

    #[test]
    fn ext4_is_laid_along_the_storages_blocks_where_the_table_gives_both_sizes() {
        let laid = command(Path::new("/dev/x"), "ext4", &extras(&["logicalblk=4096", "eraseblk=524288"]), false, 4096).unwrap();
        assert_eq!(&laid.1[7..], extras(&["-E", "stride=2,stripe-width=128"]), "a stride of 8 KiB at least");
        let wide = layout(&extras(&["logicalblk=16384", "eraseblk=4194304"])).unwrap();
        assert_eq!(wide.as_deref(), Some("stride=4,stripe-width=1024"));
        assert_eq!(layout(&extras(&["eraseblk=524288"])).unwrap(), None, "one size alone lays out nothing");
        assert!(layout(&extras(&["logicalblk=4096", "eraseblk=6000"])).is_err(), "no power of two");
        assert!(layout(&extras(&["logicalblk=2048", "eraseblk=524288"])).is_err(), "under 4 KiB");
    }

    #[test]
    fn the_tables_length_is_taken_as_it_stands_or_kept_back_from_the_partitions_end() {
        let size = || Ok(1 << 30);
        assert_eq!(length(&extras(&["check"]), size).unwrap(), None);
        assert_eq!(length(&extras(&["length=0"]), size).unwrap(), None);
        assert_eq!(length(&extras(&["length=268435456"]), size).unwrap(), Some(1 << 28));
        assert_eq!(length(&extras(&["length=-16384"]), size).unwrap(), Some((1 << 30) - 16384));
        assert!(length(&extras(&["length=-1073741824"]), size).is_err(), "nothing would be left");
        assert!(length(&extras(&["length=16M"]), size).is_err(), "bytes are written as a number alone");
    }

    #[test]
    fn a_filesystem_the_recovery_cannot_make_is_refused() {
        assert!(command(Path::new("/dev/x"), "erofs", &[], false, 4096).is_err());
    }

    /// A file of `mib` MiB in the temporary folder, as a block device the
    /// test writes to, and the file. The device lets the file go with
    /// [`let_go`].
    #[cfg(target_os = "android")]
    fn partition(name: &str, mib: u64) -> (std::path::PathBuf, String) {
        let image = std::env::temp_dir().join(format!("skyward-erase-{name}-{}.img", std::process::id()));
        std::fs::File::create(&image).unwrap().set_len(mib << 20).unwrap();
        (image.clone(), attach(&image))
    }

    #[cfg(target_os = "android")]
    fn attach(image: &Path) -> String {
        let out = std::process::Command::new("/system/bin/losetup").args(["-f", "--show"]).arg(image).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    #[cfg(target_os = "android")]
    fn let_go(device: &str) {
        assert!(std::process::Command::new("/system/bin/losetup").args(["-d", device]).status().unwrap().success(), "let {device} go");
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_f2fs_is_made_and_checks_out_clean() {
        use crate::fsck::{self, Fixed};
        let (image, device) = partition("f2fs", 128);
        let dev = Path::new(&device);
        make(dev, "f2fs", &[], false).unwrap();
        assert_eq!(crate::probe::filesystem(dev).unwrap().as_deref(), Some("f2fs"));
        let (fixed, ran) = fsck::repair(dev, "f2fs").unwrap();
        assert_eq!(fixed, Fixed::Clean, "{}", ran.said);
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_ext4_is_made_checked_and_has_its_blocks_unshared() {
        use crate::fsck::{self, Fixed};
        let (image, device) = partition("ext4", 64);
        let dev = Path::new(&device);
        make(dev, "ext4", &["metadata_csum".into()], false).unwrap();
        assert_eq!(crate::probe::filesystem(dev).unwrap().as_deref(), Some("ext4"));
        let (fixed, ran) = fsck::repair(dev, "ext4").unwrap();
        assert_eq!(fixed, Fixed::Clean, "{}", ran.said);
        let e2fsck = crate::tools::find(fsck::E2FSCK).unwrap();
        let (fixed, ran) = fsck::unshare_blocks(&e2fsck, dev).unwrap();
        assert_eq!(fixed, Fixed::Clean, "no blocks are shared: {}", ran.said);
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_f2fs_folds_case_where_asked_and_takes_the_tables_length() {
        let (image, device) = partition("casefold", 128);
        make(Path::new(&device), "f2fs", &["length=-16777216".into()], true).unwrap();
        let args = ["--dry-run".to_string(), "--no-kernel-check".into(), device.clone()];
        let said = tools::find("fsck.f2fs").unwrap().run(&args).unwrap().said;
        let features = said.lines().find(|l| l.contains("superblock features")).unwrap_or_default();
        assert!(features.contains("casefold"), "{said}");
        assert!(said.contains("total FS sectors = 229376 (112 MB)"), "16 MiB kept back at the end: {said}");
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_ext4_takes_the_tables_length() {
        let (image, device) = partition("length", 64);
        make(Path::new(&device), "ext4", &["length=-8388608".into()], false).unwrap();
        let args = ["-f".to_string(), "-n".into(), device.clone()];
        let said = tools::find(crate::fsck::E2FSCK).unwrap().run(&args).unwrap().said;
        assert!(said.contains("/14336 blocks"), "8 MiB kept back at the end: {said}");
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    /// Where make_f2fs puts the node address table of a 128 MiB filesystem,
    /// in 4 KiB blocks, and the second copy of it a segment on.
    #[cfg(target_os = "android")]
    const NAT: [u64; 2] = [2560, 2560 + 512];

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_damaged_f2fs_is_told_as_fixed_once_repaired() {
        use crate::fsck::{self, Fixed};
        use std::os::unix::fs::FileExt;
        let (image, device) = partition("damaged", 128);
        make(Path::new(&device), "f2fs", &[], false).unwrap();
        let at = std::env::temp_dir().join(format!("skyward-erase-damaged-{}", std::process::id()));
        std::fs::create_dir_all(&at).unwrap();
        crate::mount::mount(Path::new(&device), &at, "f2fs", &[]).unwrap();
        for k in 0..8 {
            std::fs::write(at.join(format!("f{k}")), vec![k as u8; 1 << 20]).unwrap();
        }
        crate::mount::umount(&at).unwrap();
        std::fs::remove_dir(&at).unwrap();
        // Both copies of the table's first block written over, as storage
        // that went bad would leave them.
        let_go(&device);
        let file = std::fs::OpenOptions::new().write(true).open(&image).unwrap();
        for block in NAT {
            file.write_all_at(&[0x5a; 4096], block * 4096).unwrap();
        }
        drop(file);
        let device = attach(&image);
        let (fixed, ran) = fsck::repair(Path::new(&device), "f2fs").unwrap();
        assert_eq!(fixed, Fixed::Fixed, "{}", ran.said);
        assert!(ran.said.contains("[Fail]"), "it says which checks failed: {}", ran.said);
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_a_partition_discarded_holds_no_filesystem() {
        let (image, device) = partition("blank", 16);
        assert_eq!(crate::probe::filesystem(Path::new(&device)).unwrap(), None);
        let_go(&device);
        std::fs::remove_file(&image).unwrap();
    }

    #[cfg(target_os = "android")]
    #[test]
    fn on_the_phone_erofs_is_told_though_blkid_does_not_know_it() {
        let tmp = std::env::temp_dir();
        let (image, source) = (tmp.join(format!("skyward-erase-erofs-{}.img", std::process::id())), tmp.join(format!("skyward-erase-erofs-{}", std::process::id())));
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("build.prop"), b"ro.product.device=ishtar\n").unwrap();
        // The ramdisk's mkfs.erofs makes an image file, not a filesystem on
        // a device.
        let made = std::process::Command::new("/system/bin/mkfs.erofs").arg(&image).arg(&source).output().unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        let device = attach(&image);
        assert_eq!(crate::probe::filesystem(Path::new(&device)).unwrap().as_deref(), Some("erofs"));
        let_go(&device);
        std::fs::remove_dir_all(&source).unwrap();
        std::fs::remove_file(&image).unwrap();
    }
}
