//! The programs a phone's own init scripts say its key service is made of.
//!
//! A configuration names them, and this fork's does for the phone it was
//! written on. Another phone's system may keep the same programs elsewhere,
//! or call them something else: its init scripts say which they are and how
//! they run, and the recovery reads them as it starts. That is the part of
//! the phone's own system the key service needs, and nothing more: no binder
//! service of the recovery's own is served here, and the secure element is
//! not spoken to.

use crate::Service;

/// What a program is, of the ones the key service is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The secure world's listener, which the key service talks through.
    Qseecomd,
    /// The directory the services of the older kind register with.
    Directory,
    /// The key service itself.
    Keymint,
    /// The credential service, which a screen lock is checked by.
    Gatekeeper,
}

/// Read the programs of the key service out of the text of one init script,
/// as init reads a service block: the `service` line names the program and
/// where it is, the lines under it say who it runs as and what it is ready
/// with.
pub fn parse(text: &str) -> Vec<Service> {
    let mut out = Vec::new();
    let mut block: Option<Block> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("service ") {
            out.extend(block.take().and_then(Block::build));
            let mut words = rest.split_whitespace();
            if let (Some(_), Some(path)) = (words.next(), words.next()) {
                block = Some(Block::new(path));
            }
        } else if line.trim().is_empty() {
            out.extend(block.take().and_then(Block::build));
        } else if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(block) = block.as_mut() {
                block.option(line.trim());
            }
        } else {
            // Another kind of block, or a command: the service's is over.
            out.extend(block.take().and_then(Block::build));
        }
    }
    out.extend(block.take().and_then(Block::build));
    out
}

/// Put the programs in the order the key service needs them, one of each
/// kind: the listener, the directory, the key service, the credential
/// service.
pub fn in_order(mut found: Vec<Service>) -> Vec<Service> {
    found.sort_by_key(|s| kind_of(&s.path));
    found.dedup_by(|a, b| a.path == b.path);
    let mut seen = Vec::new();
    found.retain(|s| {
        let kind = kind_of(&s.path);
        if seen.contains(&kind) {
            return false;
        }
        seen.push(kind);
        true
    });
    found
}

/// What `path` is, where it is one of the key service's programs.
fn kind_of(path: &str) -> Option<Kind> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.contains("keymint") {
        Some(Kind::Keymint)
    } else if name.contains("gatekeeper") {
        Some(Kind::Gatekeeper)
    } else if name == "qseecomd" {
        Some(Kind::Qseecomd)
    } else if name == "hwservicemanager" {
        Some(Kind::Directory)
    } else {
        None
    }
}

/// A service block being read.
struct Block {
    path: String,
    user: Option<String>,
    groups: Vec<String>,
    capabilities: Vec<String>,
    oneshot: bool,
}

impl Block {
    fn new(path: &str) -> Self {
        Block { path: path.to_string(), user: None, groups: Vec::new(), capabilities: Vec::new(), oneshot: false }
    }

    fn option(&mut self, line: &str) {
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else { return };
        match first {
            "user" => self.user = words.next().map(str::to_string),
            "group" => self.groups = words.map(str::to_string).collect(),
            "capabilities" => self.capabilities = words.map(str::to_string).collect(),
            "oneshot" => self.oneshot = true,
            _ => {}
        }
    }

    fn build(self) -> Option<Service> {
        let kind = kind_of(&self.path)?;
        // What each program says it is ready with, as its own build has it:
        // the listener by a property, the directory by one, the key service
        // by the name it registers, and the credential service by its own.
        let (ready_property, ready_service, ready_hardware_service) = match kind {
            Kind::Qseecomd => (Some("vendor.sys.listeners.registered".to_string()), None, None),
            Kind::Directory => (Some("hwservicemanager.ready".to_string()), None, None),
            Kind::Keymint if self.path.contains("security.keymint") => {
                (None, Some("android.hardware.security.keymint.IKeyMintDevice/default".to_string()), None)
            }
            Kind::Keymint => (None, Some("android.hardware.keymint@1.0::IKeymintDevice/default".to_string()), None),
            Kind::Gatekeeper => {
                let version = ["1.0", "1.1", "1.2"].into_iter().find(|v| self.path.contains(&format!("gatekeeper@{v}-"))).unwrap_or("1.0");
                (None, None, Some(format!("android.hardware.gatekeeper@{version}::IGatekeeper/default")))
            }
        };
        Some(Service {
            path: self.path,
            args: Vec::new(),
            libraries: Vec::new(),
            oneshot: self.oneshot,
            ready_property,
            ready_service,
            registers: Vec::new(),
            description: None,
            ready_hardware_service,
            when: None,
            user: self.user,
            groups: self.groups,
            capabilities: self.capabilities,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the 13 Pro's own vendor partition says of the programs the key
    /// service is made of, and the touch programs besides them.
    const NUWA: &str = "\
service vendor.qseecomd /vendor/bin/qseecomd
    socket notify-topology stream 660 system drmrpc
    class core
    user root
    group root drmrpc

service vendor.keymint-qti /vendor/bin/hw/android.hardware.security.keymint-service-qti
    class early_hal
    user system
    group system drmrpc

service gatekeeper-1-0 /vendor/bin/hw/android.hardware.gatekeeper@1.0-service-qti
    class early_hal
    user system
    group system

service touchfeature_v2-hal-1-0 /odm/bin/hw/vendor.xiaomi.hw.touchfeature@1.0-service
    class hal
    user system
    group system system input
";

    #[test]
    fn the_key_services_are_read_out_of_the_phone_s_own_init_script() {
        let found = in_order(parse(NUWA));
        assert_eq!(found.len(), 3, "the touch program is not one of them");
        assert!(found[0].path.ends_with("qseecomd"));
        assert_eq!(found[0].ready_property.as_deref(), Some("vendor.sys.listeners.registered"));
        assert_eq!(found[0].user.as_deref(), Some("root"));
        assert!(found[1].path.contains("security.keymint"));
        assert_eq!(found[1].ready_service.as_deref(), Some("android.hardware.security.keymint.IKeyMintDevice/default"));
        assert_eq!(found[1].groups, ["system", "drmrpc"]);
        assert!(found[2].path.contains("gatekeeper@1.0"));
        assert_eq!(found[2].ready_hardware_service.as_deref(), Some("android.hardware.gatekeeper@1.0::IGatekeeper/default"));
    }

    #[test]
    fn the_directory_comes_before_the_key_service() {
        let text = "\
service vendor.keymint-qti /vendor/bin/hw/android.hardware.security.keymint-service-qti
    user system

service hwservicemanager /system/bin/hwservicemanager
    user system
    group system readproc
";
        let found = in_order(parse(text));
        assert_eq!(found.len(), 2);
        assert!(found[0].path.ends_with("hwservicemanager"), "the directory is started before what registers with it");
        assert_eq!(found[0].user.as_deref(), Some("system"));
    }

    #[test]
    fn one_program_of_each_kind_is_kept() {
        let text = "\
service a /vendor/bin/qseecomd
service b /system/bin/qseecomd
service c /vendor/bin/hw/android.hardware.security.keymint-service-qti
";
        assert_eq!(in_order(parse(text)).len(), 2, "the second listener is left out");
    }

    #[test]
    fn a_service_of_another_kind_is_left_out() {
        assert!(parse("service x /vendor/bin/hw/vendor.xiaomi.hardware.mikeybag-service\n    user system\n").is_empty());
    }
}
