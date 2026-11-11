//! What the phone's own secure services need before they will run inside a
//! recovery: the libraries their partitions were built against, and a
//! service directory that knows the phone describes them.

use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where a phone keeps the bundles of shared libraries its own partitions
/// were built against.
const BUNDLE_DIRS: [&str; 4] = ["/mnt/system_ext/apex", "/mnt/system/system/apex", "/mnt/product/apex", "/vendor/apex"];

/// The file inside a bundle that holds its filesystem. It is stored whole,
/// so the bundle can be opened where it lies.
const BUNDLE_IMAGE: &str = "apex_payload.img";

/// Where Android opens its bundles, and the directory of 64-bit libraries
/// inside one.
const BUNDLE_MOUNT: &str = "/apex";
const BUNDLE_LIBS: &str = "lib64";

/// Which build of the vendor interface the phone's own partitions were
/// made for.
const VENDOR_INTERFACE: &str = "ro.vndk.version";

/// Where the phone describes the services its own partitions carry, and
/// where a recovery looks for such descriptions.
const DESCRIPTIONS: [&str; 2] = ["/vendor/etc/vintf/manifest", "/odm/etc/vintf/manifest"];
const RECOVERY_DESCRIPTIONS: &str = "/system/etc/vintf/manifest";

/// The name init knows the service directory by, and the program it runs.
const DIRECTORY_SERVICE: &str = "servicemanager";
const DIRECTORY_PROGRAM: &str = "/system/bin/servicemanager";

/// Where init says which process each of its services runs as, one
/// property per service, the service's name following this.
const SERVICE_PIDS: &str = "init.svc_debug_pid.";

/// How long a service init starts again may take to register again.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);

/// How long init may take to see a service it stopped gone.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// The loader a program built to run before anything is mounted asks for,
/// and where the installed system keeps the one it was built for: in the
/// bundle that carries the rest of its own runtime.
const EARLY_LOADER: &str = "/system/bin/bootstrap/linker64";
const RUNTIME_BUNDLE: &str = "com.android.runtime";
const BUNDLE_LOADER: &str = "bin/linker64";

/// Which programs may register which hardware services, and where the
/// directory for those services looks for that.
const SERVICE_LABELS: &str = "/mnt/system/system/etc/selinux/plat_hwservice_contexts";
const RECOVERY_LABELS: &str = "/system/etc/selinux";

/// The vendor's own service labels: the folder they live in and the file
/// among them that says which interfaces may register with the hardware
/// directory.
const VENDOR_LABELS: &str = "/vendor/etc/selinux";
const VENDOR_LABEL_FILE: &str = "vendor_hwservice_contexts";

/// Every hardware service's interface chain ends here, and the directory
/// checks the whole chain before it lets one register.
const BASE_INTERFACE: &str = "android.hidl.base::IBase";

/// The driver the phone's hardware services talk over, as they expect to
/// find it and as a recovery has it.
const HARDWARE_DRIVER: &str = "/dev/hwbinder";
const HARDWARE_DRIVER_NODE: &str = "/dev/binderfs/hwbinder";

/// The directory for those services is itself one of them, and it turns
/// itself off when the running system does not describe it.
const HARDWARE_DIRECTORY: &str = "android.hidl.manager@1.2::IServiceManager/default";

/// What says the directory is off. It is read by the services, and in a
/// recovery there is nothing else to set it.
const DIRECTORY_OFF: &str = "hwservicemanager.disabled";

/// Where a recovery is given the descriptions of the services the running
/// system carries. A recovery has no framework of its own, so this is read
/// as the framework's description of what it offers.
const RECOVERY_MANIFEST: &str = "/system/etc/vintf/manifest.xml";

/// What the directory for the phone's hardware services of the older kind
/// needs before it will run inside a recovery. The program is the installed
/// system's own: it is built to run before any partition is mounted, so it
/// asks for a loader under a name a recovery does not carry; it decides what
/// may register from labels a recovery does not carry either; and it reads
/// the driver under a name that a recovery, which has the driver's own
/// filesystem, does not have.
///
/// `wanted` names the hardware services that will be asked for.
pub fn prepare_hardware_directory(wanted: &[String]) -> Result<()> {
    // A loader and the lowest libraries are built together and agree
    // between them on how a program's own storage is laid out, so the
    // installed system's programs are loaded by the installed system's
    // loader rather than by the older one a recovery carries.
    let loader = Path::new(EARLY_LOADER);
    let from = bundle(RUNTIME_BUNDLE)?
        .map(|at| at.join(BUNDLE_LOADER))
        .filter(|p| p.is_file())
        .with_context(|| format!("the installed system carries no {RUNTIME_BUNDLE}, which holds the loader its own programs ask for"))?;
    let dir = loader.parent().expect("the loader is in a directory");
    fs::create_dir_all(dir).with_context(|| format!("make {}", dir.display()))?;
    // Made under the recovery's own umask the folder is root's alone, and
    // the programs that ask for this loader run as the users their init
    // scripts name: as the system's own folder is, it is open to all.
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .with_context(|| format!("open {} to every user", dir.display()))?;
    if loader.exists() {
        fs::remove_file(loader).with_context(|| format!("take away {}", loader.display()))?;
    }
    std::os::unix::fs::symlink(&from, loader).with_context(|| format!("point {} at {}", loader.display(), from.display()))?;
    let driver = Path::new(HARDWARE_DRIVER);
    if !driver.exists() {
        std::os::unix::fs::symlink(HARDWARE_DRIVER_NODE, driver)
            .with_context(|| format!("point {HARDWARE_DRIVER} at {HARDWARE_DRIVER_NODE}"))?;
    }
    // The system's own init opens the driver to every program, as it does
    // /dev/binder; a recovery's, which uses none of it, leaves it root's
    // alone, and the directory and the services that register with it run
    // as the users their init scripts name.
    fs::set_permissions(driver, std::os::unix::fs::PermissionsExt::from_mode(0o666))
        .with_context(|| format!("open {HARDWARE_DRIVER} to every program"))?;
    let from = Path::new(SERVICE_LABELS);
    let into = Path::new(RECOVERY_LABELS).join(from.file_name().expect("a named file"));
    fs::create_dir_all(RECOVERY_LABELS).with_context(|| format!("make {RECOVERY_LABELS}"))?;
    // The directory reads both of these as the user its init script names,
    // and what the recovery makes under its own umask is root's alone: they
    // are given the modes the system's own copies have.
    fs::set_permissions(RECOVERY_LABELS, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .with_context(|| format!("open {RECOVERY_LABELS} to every user"))?;
    fs::copy(from, &into).with_context(|| format!("copy {} to {}", from.display(), into.display()))?;
    declare_service_labels(wanted)?;
    fs::write(RECOVERY_MANIFEST, describe_hardware(wanted)?).with_context(|| format!("write {RECOVERY_MANIFEST}"))?;
    // The recovery service starts with umask 077, so the file is left one
    // only root may read. The programs that register with the directory run
    // as users, and each reads this before registering to learn whether the
    // phone allows it; finding it unreadable, they refuse to register and
    // the credential service never comes up. Leave the file open and the
    // folder above it walkable.
    let manifest = Path::new(RECOVERY_MANIFEST);
    if let Some(dir) = manifest.parent() {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("open {} to the services", dir.display()))?;
    }
    fs::set_permissions(manifest, fs::Permissions::from_mode(0o644))
        .with_context(|| format!("open {RECOVERY_MANIFEST} to the services"))?;
    open_descriptions(Path::new(RECOVERY_DESCRIPTIONS))?;
    // The directory turns itself off when it cannot find its own
    // description, and a run of it before this was written may have left
    // that behind.
    skyward_sys::set_prop(DIRECTORY_OFF, "false")?;
    Ok(())
}

/// The description a recovery offers of the hardware services that will be
/// asked for, and of the directory they are asked of. Each is named as the
/// package it is in and the rest of its name, which is how a description
/// holds the two.
fn describe_hardware(wanted: &[String]) -> Result<String> {
    let mut out = String::from("<manifest version=\"1.0\" type=\"framework\">\n");
    for name in std::iter::once(HARDWARE_DIRECTORY).chain(wanted.iter().map(String::as_str)) {
        let (package, rest) = name
            .split_once('@')
            .with_context(|| format!("{name} does not read as a hardware service's name"))?;
        // A description holds the version and the interface as elements of
        // their own; the one-line name alone is not read as either, and a
        // service it names is then refused as undescribed.
        let (version, rest) = rest
            .split_once("::")
            .with_context(|| format!("{name} does not name a version"))?;
        let (interface, instance) = rest
            .split_once('/')
            .with_context(|| format!("{name} does not name an instance"))?;
        out.push_str("    <hal format=\"hidl\">\n");
        out.push_str(&format!("        <name>{package}</name>\n"));
        out.push_str("        <transport>hwbinder</transport>\n");
        out.push_str(&format!("        <version>{version}</version>\n"));
        out.push_str("        <interface>\n");
        out.push_str(&format!("            <name>{interface}</name>\n"));
        out.push_str(&format!("            <instance>{instance}</instance>\n"));
        out.push_str("        </interface>\n");
        out.push_str(&format!("        <fqname>@{version}::{interface}/{instance}</fqname>\n"));
        out.push_str("    </hal>\n");
    }
    out.push_str("</manifest>\n");
    Ok(out)
}

/// Let the services read the descriptions. They run as users, and the
/// recovery service writes under umask 077, so every description it wrote
/// is one only root may read. That is not a local failure: a single
/// unreadable fragment fails the whole framework manifest, and the services
/// are then told the phone describes nothing at all.
fn open_descriptions(dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("open {} to the services", dir.display()))?;
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o644))
                .with_context(|| format!("open {} to the services", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Give the vendor's service labels the lines for the services this phone
/// still carries. hwservicemanager decides whether one may register by
/// looking its interface up in this file; a lookup that finds nothing is a
/// refusal of its own, not a policy decision, so a phone in permissive mode
/// refuses just the same. Android 17's file no longer names the older
/// services, so a copy with the lines added is shown in its place.
fn declare_service_labels(wanted: &[String]) -> Result<()> {
    let dir = Path::new(VENDOR_LABELS);
    let labels = dir.join(VENDOR_LABEL_FILE);
    if !labels.is_file() {
        return Ok(());
    }
    let have = fs::read_to_string(&labels).with_context(|| format!("read {}", labels.display()))?;
    let mut missing = Vec::new();
    if !have.contains(BASE_INTERFACE) {
        missing.push(BASE_INTERFACE.to_string());
    }
    for name in wanted {
        if let Some(interface) = interface_of(name) {
            if !have.contains(&interface) {
                missing.push(interface);
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    // The installed system's own file still names them, so the lines are
    // taken from there rather than written out here: each carries the label
    // the system's policy gives that service.
    let plat = fs::read_to_string(SERVICE_LABELS).with_context(|| format!("read {SERVICE_LABELS}"))?;
    let mut add = String::new();
    for interface in missing {
        if let Some(line) = plat.lines().find(|line| line.starts_with(&interface)) {
            add.push_str(line);
            add.push('\n');
        }
    }
    if add.is_empty() {
        return Ok(());
    }
    let staged = Path::new("/tmp/selinux");
    if staged.exists() {
        fs::remove_dir_all(staged).with_context(|| format!("clear {}", staged.display()))?;
    }
    copy_labels(dir, staged)?;
    let staged_labels = staged.join(VENDOR_LABEL_FILE);
    let mut text = fs::read_to_string(&staged_labels).with_context(|| format!("read {}", staged_labels.display()))?;
    text.push_str(&add);
    fs::write(&staged_labels, text).with_context(|| format!("add the services to {}", staged_labels.display()))?;
    fs::set_permissions(&staged_labels, fs::Permissions::from_mode(0o644))?;
    skyward_block::mount::bind(staged, dir).with_context(|| format!("show {} at {}", staged.display(), dir.display()))?;
    Ok(())
}

/// The interface a service's name belongs to, without the version or the
/// instance: `pkg@1.2::IFace/default` becomes `pkg::IFace`.
fn interface_of(name: &str) -> Option<String> {
    let (package, rest) = name.split_once('@')?;
    let (_, rest) = rest.split_once("::")?;
    let (interface, _) = rest.split_once('/')?;
    Some(format!("{package}::{interface}"))
}

/// Copy the labels folder, keeping it readable: the directory and every
/// file in it are read by services that run as users, and the recovery
/// service writes under umask 077.
fn copy_labels(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("make {}", to.display()))?;
    fs::set_permissions(to, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("open {} to the services", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_labels(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("copy {} to {}", entry.path().display(), target.display()))?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
                .with_context(|| format!("open {} to the services", target.display()))?;
        }
    }
    Ok(())
}

/// Open the bundle of libraries the phone's own partitions were built
/// against and give the directory they are in. A phone whose partitions
/// need no bundle has none to open.
pub fn libraries() -> Result<Option<PathBuf>> {    let Some(version) = skyward_sys::prop(VENDOR_INTERFACE) else { return Ok(None) };
    Ok(bundle(&format!("com.android.vndk.v{version}"))?.map(|at| at.join(BUNDLE_LIBS)))
}

/// Open the bundle of this name where it lies, and give where it is. A
/// bundle the phone does not carry gives nothing; one already open is left
/// as it is.
fn bundle(name: &str) -> Result<Option<PathBuf>> {
    let at = Path::new(BUNDLE_MOUNT).join(name);
    if skyward_block::mount::is_mount_point(&at)? {
        return Ok(Some(at));
    }
    let Some(file) = BUNDLE_DIRS.iter().map(|d| Path::new(d).join(format!("{name}.apex"))).find(|p| p.is_file()) else {
        return Ok(None);
    };
    let (offset, length) = image(&file)?;
    let device = skyward_block::loopdev::attach(&file, offset, length)?;
    fs::create_dir_all(&at).with_context(|| format!("make {}", at.display()))?;
    skyward_block::mount::mount_ro(&device, &at)?;
    Ok(Some(at))
}

/// Where the filesystem lies inside a bundle.
fn image(file: &Path) -> Result<(u64, u64)> {
    let f = fs::File::open(file).with_context(|| format!("open {}", file.display()))?;
    let mut bundle = zip::ZipArchive::new(f).with_context(|| format!("read what is in {}", file.display()))?;
    let entry = bundle.by_name(BUNDLE_IMAGE).with_context(|| format!("{} holds no {BUNDLE_IMAGE}", file.display()))?;
    if entry.compression() != zip::CompressionMethod::Stored {
        anyhow::bail!("{} keeps its {BUNDLE_IMAGE} packed, which cannot be opened where it lies", file.display());
    }
    let start = entry.data_start().with_context(|| format!("{BUNDLE_IMAGE} in {} says where it is not", file.display()))?;
    Ok((start, entry.size()))
}

/// Give the phone's service directory the descriptions of the services in
/// `wanted`, and the description files in `files`, which the phone keeps
/// apart from the rest. The directory turns away a hardware service the
/// running system does not describe, and a recovery starts up describing
/// only the few services of its own. Only the descriptions that are needed
/// are taken: the directory reads the whole of its description directory
/// at once, and one file it cannot make sense of leaves it describing
/// nothing.
pub fn publish_descriptions(wanted: &[String], files: &[String]) -> Result<usize> {
    let into = Path::new(RECOVERY_DESCRIPTIONS);
    fs::create_dir_all(into).with_context(|| format!("make {}", into.display()))?;
    // Both directories read these as the users their init scripts name,
    // and what the recovery makes under its own umask is root's alone: the
    // folder and each description are given the modes the system's own
    // copies have.
    fs::set_permissions(into, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .with_context(|| format!("open {} to every user", into.display()))?;
    let mut copied = 0;
    let mut take = |from: &Path, text: &str| -> Result<()> {
        let name = from.file_name().with_context(|| format!("{} names no file", from.display()))?;
        let to = into.join(name);
        fs::write(&to, as_framework(text)).with_context(|| format!("copy {}", from.display()))?;
        fs::set_permissions(&to, std::os::unix::fs::PermissionsExt::from_mode(0o644))
            .with_context(|| format!("let every user read {}", to.display()))?;
        copied += 1;
        Ok(())
    };
    for dir in DESCRIPTIONS {
        let listing = match fs::read_dir(dir) {
            Ok(listing) => listing,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("read {dir}")),
        };
        for entry in listing {
            let from = entry.with_context(|| format!("read {dir}"))?.path();
            if from.extension().is_none_or(|e| e != "xml") {
                continue;
            }
            let text = fs::read_to_string(&from).with_context(|| format!("read {}", from.display()))?;
            if wanted.iter().any(|w| describes(&text, w)) {
                take(&from, &text)?;
            }
        }
    }
    for file in files {
        let from = Path::new(file);
        take(from, &fs::read_to_string(from).with_context(|| format!("read {file}"))?)?;
    }
    let rewritten = say_framework(into)?;
    open_descriptions(into)?;
    if copied > 0 || rewritten > 0 {
        // The directory reads the descriptions once and keeps to them, so
        // it has to start again to see these. It comes back knowing nothing
        // of what had registered with it, and nothing registers again by
        // itself, so init starts each of those services again, as the
        // installed system's init does whenever its directory restarts.
        let registered = registered()?;
        let was = crate::services::running(DIRECTORY_PROGRAM);
        skyward_sys::set_prop("ctl.restart", DIRECTORY_SERVICE).context("have the service directory start again")?;
        answering_again(was)?;
        register_again(&registered)?;
    }
    Ok(copied)
}

/// What has registered with the service directory, each with the init
/// service whose process serves it; the directory's own entry is left out.
fn registered() -> Result<Vec<(String, String)>> {
    let Some(directory) = crate::services::running(DIRECTORY_PROGRAM) else { return Ok(Vec::new()) };
    let binder = skyward_binder::driver::Binder::open()?;
    let started = skyward_sys::props(SERVICE_PIDS);
    let mut out = Vec::new();
    for name in skyward_binder::service::names(&binder)? {
        let Some(service) = skyward_binder::service::find(&binder, &name)? else { continue };
        let pid = skyward_binder::service::pid(&service).with_context(|| format!("ask which process serves {name}"))?;
        if pid == directory {
            continue;
        }
        let (prop, _) = started
            .iter()
            .find(|(_, value)| value.parse() == Ok(pid))
            .with_context(|| format!("{name} is served by process {pid}, which init does not run, so nothing would register it again"))?;
        out.push((name, prop[SERVICE_PIDS.len()..].to_string()));
    }
    Ok(out)
}

/// Have init start again the services behind what had registered, and wait
/// until each name is registered once more. A restart through init waits
/// out the pause it keeps between two starts of a service, seconds after
/// the service last started; stopping it and starting it does not.
fn register_again(registered: &[(String, String)]) -> Result<()> {
    let services: BTreeSet<&str> = registered.iter().map(|(_, service)| service.as_str()).collect();
    for service in &services {
        skyward_sys::set_prop("ctl.stop", service).with_context(|| format!("have init stop {service}"))?;
    }
    for service in &services {
        skyward_sys::wait_prop(&format!("init.svc.{service}"), "stopped", STOP_TIMEOUT)?;
        skyward_sys::set_prop("ctl.start", service).with_context(|| format!("have init start {service} again"))?;
    }
    let binder = skyward_binder::driver::Binder::open()?;
    let until = Instant::now() + REGISTER_TIMEOUT;
    for (name, service) in registered {
        while skyward_binder::service::find(&binder, name)?.is_none() {
            ensure!(Instant::now() < until, "{name} has not registered again since init started {service} again");
            std::thread::sleep(Duration::from_millis(50));
        }
        skyward_sys::kmsg(&format!("{name} registered again from {service}"))?;
    }
    Ok(())
}

/// A description of what the running system offers, rather than of what
/// the phone's own partitions carry. A recovery publishes a description
/// into the place both readers take as the framework's, and the newer of
/// the two readers drops the whole set over one description that says it
/// is the phone's.
fn as_framework(text: &str) -> String {
    text.replace("type=\"device\"", "type=\"framework\"").replace("type='device'", "type='framework'")
}

/// Say the same of the descriptions already there, which a recovery ships
/// with, and give how many had to be rewritten.
fn say_framework(dir: &Path) -> Result<usize> {
    let mut rewritten = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry.with_context(|| format!("read {}", dir.display()))?.path();
        if path.extension().is_none_or(|e| e != "xml") {
            continue;
        }
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let said = as_framework(&text);
        if said != text {
            fs::write(&path, said).with_context(|| format!("rewrite {}", path.display()))?;
            rewritten += 1;
        }
    }
    Ok(rewritten)
}

/// Whether a description is of the service registered under `name`, which
/// reads as the interface and then the instance: the description names the
/// two separately.
fn describes(text: &str, name: &str) -> bool {
    let Some((interface, instance)) = name.rsplit_once('/') else { return false };
    let Some((package, interface)) = interface.rsplit_once('.') else { return false };
    text.contains(package) && text.contains(&format!("{interface}/{instance}"))
}

/// Wait until the service directory is a new process and takes calls
/// again. init is only asked to restart it, so for a moment the one that
/// has not read the new descriptions is still the one answering.
fn answering_again(was: Option<u32>) -> Result<()> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let now = crate::services::running(DIRECTORY_PROGRAM);
        // Looking the directory up in itself answers nothing, and that it
        // answers at all is the whole question.
        if now.is_some() && now != was && crate::services::reachable() {
            return Ok(());
        }
        if std::time::Instant::now() >= until {
            anyhow::bail!("the service directory did not come back");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
