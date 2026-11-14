//! Opening the data partition. A phone encrypts the whole of it with a key
//! its storage hardware holds, and encrypts each user's files again with a
//! key only that user's credential leads to. This is the first of the two:
//! what it opens holds every file, with the names and contents of the ones
//! that are a user's own still encrypted.

use crate::gatekeeper::Gatekeeper;
use crate::keymint::KeyMint;
use crate::keystore::{StoredKey, Unwrap};
use crate::token::Token;
use crate::user::Bound;
use crate::weaver::Weaver;
use anyhow::{bail, ensure, Context, Result};
use skyward_binder::driver::Binder;
use skyward_block::dm::Target;
use skyward_block::fsck::Checked;
use skyward_block::fstab::Entry;
use skyward_block::{dm, mount};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Where the data partition is mounted, and the name its decrypted form is
/// given among the device-mapper's devices.
pub const MOUNT: &str = "/data";
pub const MAPPED: &str = "userdata";

/// Where the phone keeps the files its users share between apps, a folder
/// for each user, and where the recovery shows the owner's, the way the
/// phone shows them to apps.
pub const SHARED_STORAGE: &str = "/data/media";
pub const SDCARD: &str = "/sdcard";

/// The phone's owner, the first of its users.
const OWNER: u32 = 0;

/// The second space HyperOS keeps: a user of its own, whose files are kept
/// behind the owner's credential. Opening them is a second unlock, behind the
/// one the page asked for, and one that can leave the page waiting; the
/// recovery leaves them closed.
const SECOND_SPACE: u32 = 999;

/// How far an opening has come, for the page that waits on it: the number of
/// the steps below it has taken, the last of which is set as it returns.
static STEP: AtomicU8 = AtomicU8::new(0);

/// The steps an opening goes through: the partition, the files that are no
/// one user's, then each user's own.
const STEPS: u8 = 5;

/// Zero the steps for a job about to start.
pub fn restart() {
    STEP.store(0, Ordering::Relaxed);
}

/// How far the opening has come, from 0 to 1, 1 being its end.
pub fn progress() -> f32 {
    f32::from(STEP.load(Ordering::Relaxed)) / f32::from(STEPS)
}

fn at(step: u8) {
    STEP.store(step, Ordering::Relaxed);
}

/// The key is in a directory of this name under the one the phone names.
const KEY_DIR: &str = "key";

/// How the kernel knows the cipher a phone names.
const CIPHERS: [(&str, &str); 2] = [("aes-256-xts", "aes-xts-plain64"), ("adiantum", "xchacha12,aes-adiantum-plain64")];

/// The hardware reads and writes in 4096-byte units, so the partition is
/// mapped in whole multiples of eight 512-byte sectors.
const SECTORS_PER_UNIT: u64 = 8;

/// How a phone says its data partition is encrypted: the cipher, and
/// whether the key is one only the storage hardware can unwrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encryption {
    pub cipher: String,
    pub wrapped: bool,
}

impl Encryption {
    /// Read what an entry of the phone's own table says.
    pub fn of(entry: &Entry) -> Result<Encryption> {
        let said = entry.extra("metadata_encryption").context("the phone says its data partition is not encrypted whole")?;
        let (name, rest) = said.split_once(':').unwrap_or((said, ""));
        let cipher = CIPHERS
            .iter()
            .find_map(|(theirs, kernels)| (*theirs == name).then_some(*kernels))
            .with_context(|| format!("no cipher here is the {name} the phone names"))?;
        // What follows the cipher says who holds the key: nothing, where the
        // key is the one that was written down, and a form of the hardware's
        // wrapping otherwise, the first version of it and the ones after.
        let wrapped = match rest {
            "" => false,
            rest if rest.starts_with("wrappedkey_v") => true,
            other => bail!("the phone names {other} after the cipher, which is no way of holding a key here"),
        };
        Ok(Encryption { cipher: cipher.to_string(), wrapped })
    }
}

/// Where a table that names no directory keeps the key: the one fs_mgr reads
/// a table's silence as.
const KEY_DIRECTORY: &str = "/metadata/vold/metadata_encryption";

/// Where the phone keeps the key for the filesystem an entry describes.
pub fn key_dir(entry: &Entry) -> PathBuf {
    Path::new(entry.extra("keydirectory").unwrap_or(KEY_DIRECTORY)).join(KEY_DIR)
}

/// Unwrap the key the phone keeps for its data partition and give the
/// storage hardware the form of it that lasts until the phone is started
/// again.
pub fn storage_key(service: &KeyMint, entry: &Entry, encryption: &Encryption) -> Result<Vec<u8>> {
    hardware_key(service, &key_dir(entry), encryption.wrapped)
}

/// The key a directory holds, in the form the storage hardware takes for
/// this start of the phone. A key the hardware holds never appears as
/// itself: what comes back is wrapped again, for this start only.
fn hardware_key(service: &KeyMint, dir: &Path, wrapped: bool) -> Result<Vec<u8>> {
    let stored = StoredKey::load(dir).with_context(|| format!("read the key in {}", dir.display()))?;
    let key = stored.unwrap(Unwrap::Hardware(service)).with_context(|| format!("unwrap the key in {}", dir.display()))?;
    match wrapped {
        true => service.ephemeral_storage_key(&key).with_context(|| format!("make the key in {} usable now", dir.display())),
        false => Ok(key),
    }
}

/// The data partition as [`open`] leaves it.
pub struct Opened {
    /// The device the partition is read and written through: the decrypted
    /// one where the phone encrypts it whole, and the partition itself where
    /// it does not.
    pub mapped: PathBuf,
    pub filesystem: String,
    /// How the check before mounting ended, where the phone's table asks
    /// for one.
    pub checked: Option<Checked>,
}

/// Open the phone's data partition: unwrap the key it is encrypted with,
/// give that to the storage hardware, check the filesystem where the
/// phone's table asks for it, and mount it writable, as the phone mounts
/// it. The names and contents of each user's own files stay encrypted
/// until that user's key is let in as well. A partition the phone does not
/// encrypt whole is mounted as it lies. A partition that does not mount
/// leaves no device behind.
fn open(service: &KeyMint) -> Result<Opened> {
    let entries = skyward_block::fstab::read(MOUNT)?;
    let keyed = keyed_entry(&entries)?;
    // A partition a factory reset emptied holds nothing to open, and no key
    // is kept for it until the system builds it again: saying that is better
    // than the key it cannot find.
    if holds_nothing(Path::new(&keyed.device)) {
        bail!(
            "the data partition is empty: it was erased, and the system builds it again when it next starts. Start the system once, then come back."
        );
    }
    let mapped = map(service, keyed)?;
    match mount_data(&entries, keyed, &mapped) {
        Ok((checked, filesystem)) => Ok(Opened { mapped, filesystem, checked }),
        Err(e) => match dm::remove(MAPPED) {
            Ok(()) => Err(e),
            Err(r) => Err(e.context(format!("removing {MAPPED} afterwards failed too: {r:#}"))),
        },
    }
}

/// Whether `device` holds nothing at all: the first block of it is zeroes,
/// as a partition a factory reset erased holds, and as one the system has
/// not built yet holds. A partition the storage hardware encrypts whole
/// reads as ciphertext, which is never all zeroes, so this tells an empty
/// partition from one whose key is missing.
fn holds_nothing(device: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(device) else { return false };
    let mut head = [0u8; 4096];
    match file.read_exact(&mut head) {
        Ok(()) => head.iter().all(|byte| *byte == 0),
        Err(_) => false,
    }
}

/// The filesystems the probe can name: blkid's, and erofs, which the
/// ramdisk reads with dump.erofs. A partition holding another is mounted as
/// the phone's table says, without the probe to tell whether the key was the
/// right one.
const PROBED: [&str; 5] = ["f2fs", "ext4", "ext3", "ext2", "erofs"];

/// Mount the partition at /data as the entry of the filesystem it holds
/// says, after the check that entry asks for, and give how the check ended
/// and the filesystem. `keyed` is the entry the key was found by, which is
/// what a probe that names no filesystem falls back to.
fn mount_data(entries: &[Entry], keyed: &Entry, device: &Path) -> Result<(Option<Checked>, String)> {
    let holds = skyward_block::probe::filesystem(device)?;
    // A mapping through which no filesystem reads was opened under another
    // key than the one the partition was closed with. A filesystem the probe
    // cannot name is mounted as the table says, and what mount answers is the
    // answer.
    ensure!(
        holds.is_some() || !PROBED.contains(&keyed.filesystem.as_str()),
        "no filesystem can be read through {}: the key it was opened with is not the one it was closed with",
        device.display()
    );
    // A phone that names the partition once per filesystem it might hold
    // mounts it with the entry of the one it holds.
    let entry = match &holds {
        Some(held) => entries.iter().find(|e| &e.filesystem == held).unwrap_or(keyed),
        None => keyed,
    };
    let checked = match entry.extras.iter().any(|e| e == "check") {
        true => Some(skyward_block::fsck::check(device, &entry.filesystem)?),
        false => None,
    };
    std::fs::create_dir_all(MOUNT).with_context(|| format!("make {MOUNT}"))?;
    mount::mount(device, Path::new(MOUNT), &entry.filesystem, &entry.options)?;
    Ok((checked, entry.filesystem.clone()))
}

/// Map the data partition decrypted, under the key the phone keeps for it,
/// and give the device the filesystem is read and written through. A
/// filesystem that reads off the partition as it lies says the partition is
/// in the clear, whatever the table says of encrypting it: a table may keep
/// the words after the system made the partition afresh without them.
fn map(service: &KeyMint, entry: &Entry) -> Result<PathBuf> {
    let device = skyward_block::fstab::device(entry)?;
    let path = PathBuf::from(&device);
    let encrypts = entry.extra("metadata_encryption").is_some();
    if !encrypts || skyward_block::probe::filesystem(&path)?.is_some() {
        return Ok(path);
    }
    let encryption = Encryption::of(entry)?;
    let key = storage_key(service, entry, &encryption)?;
    let sectors = skyward_block::size_bytes(&device)? / 512;
    dm::create(MAPPED, &[target(&device, sectors, &key, &encryption)], false)
}

/// The entry the key for the partition is found by: the one that says how
/// the partition is encrypted whole. A phone may name the partition once per
/// filesystem it might hold, and only the one it holds says so; the others
/// are there for the filesystem that is not. The first is given where no
/// entry says it: the partition may be in the clear, which [`map`] tells by
/// itself.
fn keyed_entry(entries: &[Entry]) -> Result<&Entry> {
    match skyward_block::fstab::encrypted(entries) {
        Some(entry) => Ok(entry),
        None => entries.first().with_context(|| format!("the phone's own table says nothing about {MOUNT}")),
    }
}

/// Map the data partition decrypted without mounting it, for its
/// filesystem to be repaired, and give the device it is reached through;
/// [`unmap`] takes it away again. The partition is to be closed. A
/// partition the phone does not encrypt whole is given as it lies.
pub fn map_unmounted(service: &KeyMint) -> Result<PathBuf> {
    ensure!(dm::status(MAPPED)?.is_none(), "the data partition is open");
    let entries = skyward_block::fstab::read(MOUNT)?;
    map(service, keyed_entry(&entries)?)
}

/// Take away the mapping [`map_unmounted`] made, where it made one.
pub fn unmap() -> Result<()> {
    if dm::status(MAPPED)?.is_some() {
        dm::remove(MAPPED)?;
    }
    Ok(())
}

/// Show the owner's shared storage at /sdcard; shown already, it stays.
fn show_owner_storage() -> Result<()> {
    let at = Path::new(SDCARD);
    if mount::is_mount_point(at)? {
        return Ok(());
    }
    mount::bind(&Path::new(SHARED_STORAGE).join(OWNER.to_string()), at)
}

/// Close what opening the partition opened, where the recovery opened it:
/// /sdcard, the partition's mount, and the device it is read through.
pub fn close() -> Result<()> {
    for at in [SDCARD, MOUNT] {
        if mount::is_mount_point(Path::new(at))? {
            mount::umount(Path::new(at))?;
        }
    }
    if dm::status(MAPPED)?.is_some() {
        dm::remove(MAPPED)?;
    }
    Ok(())
}

/// A user whose own files are still closed: who, the kind of screen lock
/// they set in the phone's numbering, and whether the phone made their
/// credential up, in which case they open behind the owner and nobody is
/// asked for it. `why` says why such a user did not open behind the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closed {
    pub user: u32,
    pub lock: i32,
    pub made_up: bool,
    pub why: Option<String>,
}

/// The data partition as far as it opens with nothing a user knows.
pub struct Opening {
    /// How it was mounted, where this mounted it rather than an earlier
    /// run of the recovery.
    pub mounted: Option<Opened>,
    /// The users whose own files are still closed, the owner first.
    pub closed: Vec<Closed>,
}

/// Open the data partition as far as it opens with nothing a user knows:
/// mount it where it is not mounted, let in the keys of the files that
/// belong to no one user and of each user's files readable before they
/// sign in, and open the own files of every user who set no screen lock
/// and, behind the owner's, of those whose credential the phone made up.
pub fn open_without_credential(service: &KeyMint) -> Result<Opening> {
    at(1);
    let mounted = match mount::is_mount_point(Path::new(MOUNT))? {
        true => None,
        false => Some(open(service)?),
    };
    at(2);
    open_shared(service)?;
    at(3);
    let mut closed = Vec::new();
    let mut owner = Note::default();
    for user in open_users(service)? {
        // The second space's own files are kept behind the owner's credential,
        // so opening them is a second unlock the page is not waiting on.
        if user == SECOND_SPACE {
            continue;
        }
        let protector = protector(user)?;
        match protector.lock {
            None => {
                let note = open_user_files(service, user, None, None)?;
                if user == OWNER {
                    owner = note;
                }
            }
            Some(lock) => closed.push(Closed { user, lock: lock.kind, made_up: made_up(user, &protector.id), why: None }),
        }
    }
    at(4);
    let closed = open_made_up(service, closed, &owner);
    at(5);
    Ok(Opening { mounted, closed })
}

/// Open the own files of `user` with what they typed at their screen lock
/// and, where `user` is the owner, of each of `closed` whose credential
/// the phone made up behind theirs; give those still closed.
pub fn unlock(service: &KeyMint, closed: &[Closed], user: u32, credential: &[u8]) -> Result<Vec<Closed>> {
    at(1);
    let note = open_user_files(service, user, Some(credential), None)?;
    at(3);
    let rest = closed.iter().filter(|c| c.user != user).cloned().collect();
    let left = match user == OWNER {
        true => open_made_up(service, rest, &note),
        false => rest,
    };
    at(5);
    Ok(left)
}

/// Open the own files of each of `closed` whose credential the phone made
/// up, behind `owner`, the note the owner's own secret gave; give those
/// still closed, a made-up one with why it did not open. Where the secure
/// world wrote no note, none of them is tried and each is left with why.
fn open_made_up(service: &KeyMint, closed: Vec<Closed>, owner: &Note) -> Vec<Closed> {
    let Some(token) = &owner.token else {
        return closed
            .into_iter()
            .map(|c| match c.made_up {
                true => Closed { why: owner.refused.clone(), ..c },
                false => c,
            })
            .collect();
    };
    closed
        .into_iter()
        .filter_map(|c| match c.made_up {
            false => Some(c),
            true => match open_user_files(service, c.user, None, Some(token)) {
                Ok(_) => None,
                Err(e) => Some(Closed { why: Some(format!("{e:#}")), ..c }),
            },
        })
        .collect()
}

/// Whether the phone made up the credential of `user`, whose protector is
/// `id`, keeping it behind the owner's.
fn made_up(user: u32, id: &str) -> bool {
    made_up_record(user, id).is_some()
}

/// Where the phone keeps the key for the files that are readable before
/// anyone signs in, and what it says about how those files are encrypted.
const SHARED_KEY: &str = "/data/unencrypted/key";
const SHARED_NAME: &str = "/data/unencrypted/ref";
const FILE_MODE: &str = "/data/unencrypted/mode";

/// Where the phone keeps each user's own keys.
const USER_KEYS: &str = "/data/misc/vold/user_keys";

/// Let in the key for the files that belong to no one user. Until it is
/// in, most of what the data partition holds reads as scrambled names. A
/// phone that encrypts no files with a key of their own keeps none there,
/// and there is nothing to let in.
fn open_shared(service: &KeyMint) -> Result<()> {
    let mode = match std::fs::read_to_string(FILE_MODE) {
        Ok(mode) => mode,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("read {FILE_MODE}")),
    };
    let wrapped = crate::fscrypt::hardware_wrapped(mode.trim());
    let raw = hardware_key(service, Path::new(SHARED_KEY), wrapped)?;
    let name = crate::fscrypt::add_key(Path::new(MOUNT), &raw, wrapped)?;
    let recorded = crate::fscrypt::recorded_name(Path::new(SHARED_NAME))?;
    if name != recorded {
        anyhow::bail!("the key in {SHARED_KEY} is not the one {SHARED_NAME} names");
    }
    Ok(())
}

/// Which users the phone keeps keys for, in order.
fn users() -> Result<Vec<u32>> {
    let dir = Path::new(USER_KEYS).join("de");
    let mut users = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(users),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    for entry in entries {
        let path = entry.with_context(|| format!("read {}", dir.display()))?.path();
        let Some(user) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<u32>().ok()) else { continue };
        users.push(user);
    }
    users.sort_unstable();
    Ok(users)
}

/// Let in the keys for the files of each user that are readable before
/// that user signs in, and give the users they belong to.
fn open_users(service: &KeyMint) -> Result<Vec<u32>> {
    let mode = match std::fs::read_to_string(FILE_MODE) {
        Ok(mode) => mode,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {FILE_MODE}")),
    };
    let wrapped = crate::fscrypt::hardware_wrapped(mode.trim());
    let users = users()?;
    for user in &users {
        let dir = Path::new(USER_KEYS).join("de").join(user.to_string());
        let raw = hardware_key(service, &dir, wrapped)?;
        crate::fscrypt::add_key(Path::new(MOUNT), &raw, wrapped)?;
    }
    Ok(users)
}

/// Where a phone keeps what it needs to recover a user's own secret.
const PROTECTORS: &str = "/data/system_de";

/// Read what a user's own credential keeps their secret behind. A phone
/// may keep more than one blob for a user: Android keeps one for each
/// token some part of the phone was given to reach the secret with, and
/// HyperOS, once the owner changes one screen lock for another with a
/// fingerprint enrolled, keeps a second one for a while, so that the
/// credential before can still be used. Which of them the user's current
/// credential opens is the phone's own business, and it writes that down,
/// so this reads it rather than guess.
fn protector(user: u32) -> Result<crate::user::Protector> {
    let dir = blobs(user);
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry.with_context(|| format!("read {}", dir.display()))?.path();
        if path.extension().is_some_and(|e| e == "spblob") {
            if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
                ids.push(id.to_string());
            }
        }
    }
    let id = match crate::locksettings::current_protector(Path::new(crate::locksettings::DATABASE), user)? {
        Some(recorded) => {
            anyhow::ensure!(
                ids.contains(&recorded),
                "the phone keeps user {user}'s secret behind the blob {recorded}, which is not among the {} in {}",
                ids.len(),
                dir.display()
            );
            recorded
        }
        // Where the phone records none, only a blob that is alone is
        // beyond doubt the user's.
        None => match &ids[..] {
            [only] => only.clone(),
            _ => anyhow::bail!(
                "{} holds {} of the user's blobs, and the phone records none of them as the one the user's own credential opens",
                dir.display(),
                ids.len()
            ),
        },
    };
    let first = protector_at(&dir, &id)?;
    // A phone whose weaver a recovery cannot reach keeps a second blob for the
    // same user, bound to the bytes beside it, which the credential service
    // alone opens: that one is taken instead, as the configuration says.
    if matches!(first.bound, Bound::Weaver(_)) && !weaver_first() {
        let other = ids.iter().find(|other| {
            other.as_str() != id.as_str()
                && dir.join(format!("{other}.secdis")).is_file()
                && dir.join(format!("{other}.pwd")).is_file()
        });
        if let Some(other) = other {
            let alt = protector_at(&dir, other.as_str())?;
            if matches!(alt.bound, Bound::Secdiscardable(_)) {
                return Ok(alt);
            }
        }
    }
    Ok(first)
}

/// What the blob `id` in `dir` keeps a user's secret behind, read as it lies.
fn protector_at(dir: &Path, id: &str) -> Result<crate::user::Protector> {
    // A user who has set a screen lock has what the phone recorded about
    // it beside the blob; a user who has set none has nothing there.
    let recorded = dir.join(format!("{id}.pwd"));
    let lock = match recorded.exists() {
        true => Some(crate::user::Lock::of(
            &std::fs::read(&recorded).with_context(|| format!("read {}", recorded.display()))?,
        )?),
        false => None,
    };
    // A phone with a weaver records the slot it keeps for the blob; any
    // other keeps the bytes the blob is bound to beside it.
    let slot = dir.join(format!("{id}.weaver"));
    let bound = match slot.exists() {
        true => Bound::Weaver(crate::user::weaver_slot(&std::fs::read(&slot).with_context(|| format!("read {}", slot.display()))?)?),
        false => Bound::Secdiscardable(
            std::fs::read(dir.join(format!("{id}.secdis"))).with_context(|| format!("read what the blob in {} is bound to", dir.display()))?,
        ),
    };
    Ok(crate::user::Protector {
        blob: std::fs::read(dir.join(format!("{id}.spblob"))).with_context(|| format!("read the blob in {}", dir.display()))?,
        bound,
        id: id.to_string(),
        lock,
    })
}

/// Whether the weaver's own slots are used for a phone whose blobs are bound
/// to it: said once by the program that read the configuration, before the
/// data partition is opened.
static WEAVER_FIRST: AtomicBool = AtomicBool::new(true);

/// Says whether the weaver's own slots are used, as the phone's configuration
/// asks. Left unsaid, they are.
pub fn use_weaver_first(yes: bool) {
    WEAVER_FIRST.store(yes, Ordering::Relaxed);
}

fn weaver_first() -> bool {
    WEAVER_FIRST.load(Ordering::Relaxed)
}

/// Which of the secure world's slots a user's enrolments sit in. What the
/// user types is enrolled a hundred thousand above their own number,
/// keeping it apart from the enrolments made before phones kept
/// credentials this way; the secret behind it is enrolled under the
/// number itself.
const TYPED_SLOT: u32 = 100_000;

/// What the phone calls the enrolment of a user's own secret, among the
/// blobs it keeps for that user.
const BLOBS: &str = "spblob";
const SECRET_HANDLE: &str = "0000000000000000.handle";

/// Where a user's blobs are.
fn blobs(user: u32) -> PathBuf {
    Path::new(PROTECTORS).join(user.to_string()).join(BLOBS)
}

/// Where a phone keeps the credential it made up itself for a user whose
/// screen lock is the owner's. The file holds a nonce and then the
/// credential, encrypted under a key the owner's own secret guards.
const MADE_UP: &str = "/data/system/users";
const MADE_UP_FILE: &str = "gatekeeper.profile.key";

/// The name the key service keeps that key under, the user's number after
/// it.
const MADE_UP_KEY: &str = "profile_key_name_decrypt_";

/// HyperOS keeps the file beside the blob of the user's protector instead,
/// named after the protector with this ending, and puts the protector's
/// name after the user's number in the key's name too.
const MADE_UP_BESIDE_BLOB: &str = "profile_pwd";

/// The file holding the credential the phone made up for `user`, whose
/// protector is `id`, and the name of the key it is encrypted under: where
/// Android keeps them, or where HyperOS does. `None` where it made none up.
fn made_up_record(user: u32, id: &str) -> Option<(PathBuf, String)> {
    let android = Path::new(MADE_UP).join(user.to_string()).join(MADE_UP_FILE);
    if android.is_file() {
        return Some((android, format!("{MADE_UP_KEY}{user}")));
    }
    let hyperos = blobs(user).join(format!("{id}.{MADE_UP_BESIDE_BLOB}"));
    hyperos.is_file().then(|| (hyperos, format!("{MADE_UP_KEY}{user}.{id}")))
}

/// The credential a phone made up for `user`, whose protector is `id`,
/// where it made one up. A phone does this for the users it keeps beside
/// the owner's own, such as the one that holds a second copy of an app:
/// nobody ever types their credential, so the phone writes one down for
/// itself and keeps it behind the owner's. `owner` is the note the owner's
/// credential gave.
fn made_up_credential(service: &KeyMint, user: u32, id: &str, owner: Option<&Token>) -> Result<Option<Vec<u8>>> {
    let Some((file, alias)) = made_up_record(user, id) else { return Ok(None) };
    let stored = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
    if stored.len() < NONCE_BYTES + TAG_BYTES {
        anyhow::bail!("{} is {} bytes, too few to hold a credential", file.display(), stored.len());
    }
    let (nonce, body) = stored.split_at(NONCE_BYTES);
    let held = crate::keystore2::key_blob(Path::new(crate::keystore2::DATABASE), &alias)?;
    let credential = service.gcm_decrypt(&held, &[], nonce, body, owner).with_context(|| {
        format!("open the credential the phone made up for user {user}, which is kept behind the owner's own")
    })?;
    Ok(Some(credential))
}

/// How long the nonce and the check are on everything the key service
/// encrypts.
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// Let in the key for a user's own files, and give the note the secure
/// world wrote for the secret behind their credential, which is what the
/// keys the phone keeps for them are guarded by. `credential` is what the
/// user would type at their screen lock; a user who has set none needs
/// none, and a user the phone made one up for needs `owner`, the note the
/// owner's own secret gave. The owner's files open, their shared storage
/// shows at /sdcard.
fn open_user_files(service: &KeyMint, user: u32, credential: Option<&[u8]>, owner: Option<&Token>) -> Result<Note> {
    let protector = protector(user)?;
    let stretched = match &protector.lock {
        Some(lock) => {
            let typed = match made_up_credential(service, user, &protector.id, owner)? {
                Some(made_up) => made_up,
                None => credential
                    .with_context(|| format!("user {user} has set a screen lock, and their files do not open without it"))?
                    .to_vec(),
            };
            lock.stretched(&typed).context("stretch the credential")?
        }
        None => crate::user::stretched_without_credential(),
    };
    let (secret, note) = match (&protector.bound, &protector.lock) {
        // The weaver checks the credential itself, and gives back the value
        // the secret is bound to only for the right one. The key holding
        // the secret is then guarded by nothing else.
        (Bound::Weaver(slot), _) => {
            let binder = Binder::open()?;
            let weaver = Weaver::open(&binder)?;
            let key = crate::user::weaver_key(&stretched, weaver.config()?.key_bytes)?;
            let value = weaver.read(*slot, &key)?;
            (crate::user::weaver_protector_secret(&stretched, &value), None)
        }
        // The key holding this user's secret is guarded by the screen lock,
        // so the secure world has to be shown the credential itself before
        // it will use that key.
        (Bound::Secdiscardable(secdiscardable), Some(lock)) => {
            let (binder, hardware) = (Binder::open()?, Binder::hardware()?);
            let note = Gatekeeper::open(&binder, &hardware)?.verify(
                TYPED_SLOT + user,
                &lock.enrolled,
                &crate::user::gatekeeper_password(&stretched),
            )?;
            (crate::user::protector_secret(&stretched, secdiscardable), Some(note))
        }
        (Bound::Secdiscardable(secdiscardable), None) => (crate::user::protector_secret(&stretched, secdiscardable), None),
    };
    let held = crate::keystore2::key_blob(Path::new(crate::keystore2::DATABASE), &protector.key_alias())?;
    let content = crate::user::content(&protector.blob)?;
    if content.len() < NONCE_BYTES + TAG_BYTES {
        anyhow::bail!("the user's blob is too short to hold anything");
    }
    let (nonce, body) = content.split_at(NONCE_BYTES);
    let inner = service
        .gcm_decrypt(&held, &[], nonce, body, note.as_ref())
        .context("open what the secure world holds the user's blob closed with")?;
    let synthetic_password = crate::user::open_inner(&secret, &inner)?;
    let file_secret = crate::user::file_secret(&synthetic_password);

    let mode = std::fs::read_to_string(FILE_MODE).with_context(|| format!("read {FILE_MODE}"))?;
    let wrapped = crate::fscrypt::hardware_wrapped(mode.trim());
    let dir = Path::new(USER_KEYS).join("ce").join(user.to_string()).join("current");
    let stored = StoredKey::load(&dir).with_context(|| format!("read the key in {}", dir.display()))?;
    let raw = stored.unwrap(Unwrap::<KeyMint>::Secret(&file_secret)).with_context(|| format!("unwrap the key in {}", dir.display()))?;
    let raw = match wrapped {
        true => service.ephemeral_storage_key(&raw).with_context(|| format!("make the key in {} usable now", dir.display()))?,
        false => raw,
    };
    crate::fscrypt::add_key(Path::new(MOUNT), &raw, wrapped)?;
    if user == OWNER {
        show_owner_storage()?;
    }
    secret_note(user, &synthetic_password)
}

/// What the secure world wrote for a user's own secret, which is what the
/// credentials the phone made up for other users are kept behind. A user
/// whose secret the secure world knows nothing about has none, and so has
/// one whose secure world would write none; the user's own files are open
/// either way, and only the users kept behind this one stay closed.
#[derive(Debug, Default)]
struct Note {
    token: Option<Token>,
    /// Why the secure world wrote none, where it was asked and would not.
    refused: Option<String>,
}

/// Have the secure world check the user's own secret, so the keys the
/// phone keeps for that user can be used.
fn secret_note(user: u32, synthetic_password: &[u8]) -> Result<Note> {
    let handle = blobs(user).join(SECRET_HANDLE);
    if !handle.is_file() {
        return Ok(Note::default());
    }
    let enrolled = std::fs::read(&handle).with_context(|| format!("read {}", handle.display()))?;
    let (binder, hardware) = (Binder::open()?, Binder::hardware()?);
    let password = crate::user::secret_gatekeeper_password(synthetic_password);
    let asked = Gatekeeper::open(&binder, &hardware)
        .and_then(|gatekeeper| gatekeeper.verify(user, &enrolled, &password))
        .with_context(|| format!("have the secure world check the secret behind user {user}'s credential"));
    Ok(match asked {
        Ok(token) => Note { token: Some(token), refused: None },
        Err(e) => Note { token: None, refused: Some(format!("{e:#}")) },
    })
}

/// Build the device-mapper target that reads the partition through the
/// storage hardware's decryption.
pub fn target(device: &str, sectors: u64, key: &[u8], encryption: &Encryption) -> Target {
    Target::DefaultKey {
        sectors: sectors - sectors % SECTORS_PER_UNIT,
        device: device.to_string(),
        cipher: encryption.cipher.clone(),
        key: key.iter().map(|b| format!("{b:02x}")).collect(),
        wrapped: encryption.wrapped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(extras: &str) -> Entry {
        Entry {
            device: "/dev/block/bootdevice/by-name/userdata".into(),
            mount_point: MOUNT.into(),
            filesystem: "f2fs".into(),
            options: vec!["noatime".into(), "inlinecrypt".into()],
            extras: extras.split(',').map(str::to_string).collect(),
        }
    }

    #[test]
    fn a_partition_of_zeroes_holds_nothing_to_open() {
        let dir = std::env::temp_dir().join(format!("skyward-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("empty");
        std::fs::write(&empty, [0u8; 8192]).unwrap();
        assert!(holds_nothing(&empty), "a partition of zeroes holds nothing");
        let held = dir.join("held");
        std::fs::write(&held, [7u8; 8192]).unwrap();
        assert!(!holds_nothing(&held), "one holding anything holds something");
        assert!(!holds_nothing(&dir.join("missing")), "and one that cannot be read is not taken for empty");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_phones_own_words_for_the_cipher_become_the_kernels() {        let e = entry("metadata_encryption=aes-256-xts:wrappedkey_v0,keydirectory=/metadata/vold/metadata_encryption");
        assert_eq!(Encryption::of(&e).unwrap(), Encryption { cipher: "aes-xts-plain64".into(), wrapped: true });
        let plain = entry("metadata_encryption=adiantum");
        assert_eq!(Encryption::of(&plain).unwrap(), Encryption { cipher: "xchacha12,aes-adiantum-plain64".into(), wrapped: false });
    }

    #[test]
    fn a_cipher_or_a_key_the_phone_does_not_name_is_an_error() {
        assert!(Encryption::of(&entry("quota")).is_err());
        assert!(Encryption::of(&entry("metadata_encryption=rot13")).is_err());
        assert!(Encryption::of(&entry("metadata_encryption=aes-256-xts:keymaster_v9")).is_err(), "no key held that way");
    }

    #[test]
    fn the_key_is_in_a_directory_under_the_one_the_phone_names() {
        let e = entry("keydirectory=/metadata/vold/metadata_encryption");
        assert_eq!(key_dir(&e), Path::new("/metadata/vold/metadata_encryption/key"));
        let silent = entry("latemount,quota");
        assert_eq!(key_dir(&silent), Path::new(KEY_DIRECTORY).join(KEY_DIR), "a table that names none says the default");
    }

    #[test]
    fn the_key_is_found_by_the_entry_that_says_how_the_partition_is_encrypted() {
        let mut other = entry("latemount,keydirectory=/metadata/vold/metadata_encryption");
        other.filesystem = "mifs".into();
        let encrypted = entry("latemount,metadata_encryption=adiantum,keydirectory=/metadata/vold/metadata_encryption");
        let both = [other, encrypted.clone()];
        assert_eq!(keyed_entry(&both).unwrap(), &encrypted, "the one that says it, not the first");
        assert!(keyed_entry(&[]).is_err(), "a table with no entry for the partition at all");
    }

    #[test]
    fn a_table_that_says_nothing_of_encryption_names_the_first_entry() {
        let none = [entry("latemount,quota")];
        assert_eq!(keyed_entry(&none).unwrap(), &none[0], "the partition may be in the clear, which the map tells");
        let error = Encryption::of(&none[0]).unwrap_err();
        assert!(format!("{error:#}").contains("not encrypted whole"), "{error:#}");
    }

    #[test]
    fn the_hardwares_wrapping_is_read_in_every_version_the_phone_names_it_in() {
        for said in ["aes-256-xts:wrappedkey_v0", "aes-256-xts:wrappedkey_v1"] {
            assert!(Encryption::of(&entry(&format!("metadata_encryption={said}"))).unwrap().wrapped, "{said}");
        }
    }

    #[test]
    fn the_partition_is_mapped_in_whole_units_of_the_hardwares_reads() {
        let e = Encryption { cipher: "aes-xts-plain64".into(), wrapped: true };
        let t = target("/dev/block/by-name/userdata", 969_825_751, &[0xAB, 0x01], &e);
        let Target::DefaultKey { sectors, key, cipher, device, wrapped } = t else { panic!("a decrypting target") };
        assert_eq!(sectors, 969_825_744, "a whole number of 4096-byte units");
        assert_eq!(key, "ab01");
        assert_eq!(cipher, "aes-xts-plain64");
        assert_eq!(device, "/dev/block/by-name/userdata");
        assert!(wrapped);
    }
}
