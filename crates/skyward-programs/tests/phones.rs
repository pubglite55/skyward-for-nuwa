//! What a recovery does to use the phone's keys, found in what the phone's
//! own partitions say, read from the files `skyward-pack described` took
//! out of the phone's stock build into tests/described. The phone is the
//! one this fork serves, and what is found is what was started on it.

use skyward_programs::phone::{Partition, Phone, PARTITIONS};
use skyward_programs::secure::{self, Step};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn described(codename: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/described").join(codename)
}

/// Where a recovery has each partition mounted: the phone's own where the
/// phone has them, the installed system's under /mnt.
fn mounted(partition: &str) -> String {
    match partition {
        "vendor" | "odm" => format!("/{partition}"),
        other => format!("/mnt/{other}"),
    }
}

/// The phone `codename` names, as its stock build describes it, with the
/// properties its bootloader gives.
fn phone(codename: &str, properties: &[(&str, &str)]) -> Phone {
    let at = described(codename);
    let partitions = PARTITIONS.iter().map(|name| Partition { name: name.to_string(), mounted: mounted(name).into(), at: at.join(name) }).collect();
    let properties: HashMap<String, String> = properties.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect();
    Phone::read(partitions, properties).unwrap()
}

/// The steps, each as the name of the program's file, or as the secure
/// element service where the recovery serves it.
fn names(steps: &[Step]) -> Vec<&str> {
    steps
        .iter()
        .map(|step| match step {
            Step::Start(started) => started.service.path.rsplit('/').next().unwrap(),
            Step::ServeElements => "(the secure element service)",
        })
        .collect()
}

/// The phone this fork serves: the six programs its own partitions say its
/// keys are used through, in the order the recovery starts them. They were
/// seen started in this order on the phone, in its kernel log.
#[test]
fn nuwa_s_programs_are_found_in_the_order_they_were_started_on_the_phone() {
    let properties = [
        ("ro.boot.hardware", "qcom"),
        ("ro.boot.product.vendor.sku", "kalama"),
        ("ro.boot.product.hardware.sku", "nuwa"),
    ];
    let steps = secure::steps(&phone("nuwa", &properties)).unwrap();
    assert_eq!(
        names(&steps),
        [
            "qseecomd",
            "ssgtzd",
            "android.hardware.security.keymint-service-qti",
            "hwservicemanager",
            "android.hardware.gatekeeper@1.0-service-qti",
            "android.hardware.boot@1.2-service",
        ],
        "the listener, its opener, the key service, the hardware service directory, the credential service and the boot control"
    );
}
