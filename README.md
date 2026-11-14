![Skyward Recovery](assets/hero.jpg)

# Skyward for nuwa

English | [简体中文](README.zh-CN.md)

Skyward is a recovery for Android phones, written from scratch in Rust: no
TWRP tree, no AOSP recovery, no Java. It boots from the recovery partition as
the stock one does, draws its own interface, and does the work a recovery is
for — opening the data partition, installing packages, backing up partitions,
wiping, mounting, and handing the storage to a computer.

This repository carries Skyward to one phone: the **Xiaomi 13 Pro**
(codename `nuwa`). Upstream supports other devices:
[kirklin/skyward](https://github.com/kirklin/skyward).

The port is by [xiuxiu391](https://github.com/xiuxiu391), who keeps it.

![Skyward on the Xiaomi 13 Pro: About, home, settings and the file manager](docs/overview.webp)

<p align="center">
  <img src="assets/home.png" width="30%" alt="Home">
  <img src="assets/settings-dark.png" width="30%" alt="Settings in dark mode">
  <img src="assets/time-zone.png" width="30%" alt="Time zone">
</p>

## What it does

- **Opens the data partition.** Wrapped keys through the phone's key service,
  file-based encryption, and the users' own files behind their screen lock.
- **Installs** zips, full system update packages and partition images, from
  internal storage, a USB drive or `adb sideload`.
- **Backs up and restores** partitions and files, with a check of each backup.
- **Wipes** data, and formats or repairs filesystems.
- **Mounts** the slot's partitions, and switches slots.
- **Files and a terminal**: a file manager, a shell, and the kernel log.
- **Hands storage to a computer** over MTP, and speaks adb and fastbootd.
- **WLAN**, joining the network kept at the next start by itself, for updates
  straight from the network.
- **Wireless transfer**: a browser on the same network opens
  `http://<phone>:8080/` to list, download and drop files onto the phone.
- **Downloads from an address**: straight into `Download/`, resumed when
  stopped; a code on the page (8081) takes an address typed on another phone,
  and `link.txt` takes one a computer left.
- **Two systems** on one phone, from the Dual boot page.
- **Updates itself** from its own releases.
- Light and dark themes; Simplified Chinese, Traditional Chinese and English.

| | On this phone |
| --- | --- |
| Booting, the interface, touch, keys | verified |
| Opening the data partition | verified (keys made by Android 17, phone on 16) |
| adb | verified |
| Installing, MTP, fastbootd, WLAN, backup and restore, Dual boot, online updates | inherited from 0.2.0, not yet exercised here |
| The kept WLAN and the code on the download page | built and unit-tested; the look is still to be seen on the phone |

## Getting it onto the phone

An unlocked bootloader is the only prerequisite. Download the zip from
[Releases](https://github.com/pubglite55/skyward-for-nuwa/releases) — published
from tags named `nuwa-v*`, each file named
`skyward-nuwa-<version>-<stock build>` — check it against `SHA256SUMS.txt`,
unzip it, and flash the image inside to the recovery partition of the slot the
phone runs from:

```sh
unzip skyward-nuwa-v1.6-OS3.0.312.0.WMBCNXM.zip
fastboot flash recovery_a skyward-nuwa-v1.6-OS3.0.312.0.WMBCNXM.img
fastboot flash recovery_b skyward-nuwa-v1.6-OS3.0.312.0.WMBCNXM.img
fastboot reboot recovery
```

From then on a newer image, unzipped, installs from Skyward's own Install page.
With **Flash Skyward again** on, a system update or a zip that writes over the
recovery partition puts Skyward back in it.

Nothing of the system is touched by flashing recovery, and OTA updates go on
working; flashing the stock recovery back is all it takes to return to it.

## The pages

| Page | What it is for |
| --- | --- |
| Decrypt | What keeps the data partition closed, and the field for the password when a user's own files need one |
| Home | The phone's state, and the way into everything else |
| Files | The file manager, over internal storage, a USB drive and the slot's partitions |
| Install | Packages and images: from a place, from a USB drive, or from `adb sideload` |
| Backup | Partitions and files into a backup, and restoring one |
| Mount | The slot's partitions, mounted, unmounted, formatted, repaired |
| Wipe | Data, cache, and the phone's own logs |
| Dual boot | A second system in the data partition, installed, switched to, removed |
| Wireless | Internal storage handed to a browser on the same network: list, download, upload |
| Download | An address taken into `Download/`, resumed when stopped; a code or `link.txt` brings one |
| Tools | A terminal, the kernel log, and the recovery's own log |
| Settings | Theme, language, time zone, screen-off time, vibration, file transfer |
| Reboot | System, recovery, fastboot, bootloader, EDL, and the slot to boot |
| About | The version, the phone, and where the project comes from |

## Decryption

The data partition opens on demand: the file manager opens it when entered, and
an update that needs `/data` opens it before reading it. Where no screen lock is
set it opens by itself; where one is, the page asks for it. The metadata key is
wrapped by the phone's hardware and needs nothing typed.

A key is measured against the build it was made under, and a key made by a newer
build than the one installed is refused until it is renewed — a renewal a key of
a newer build never gets. This phone's data was made by HyperOS 4 (Android 17,
patch level 2099-12-31), so `devices/nuwa.toml` names that build:

```toml
build_release = "17"
build_patch = "2099-12-31"
```

The patch level is reported for the system's properties and the vendor's alike,
because the vendor build is the one a key is measured against. Should the phone
be reset from a newer system, those two lines have to name that system's build.
The log then says which generation the key was made in, and whether the
partition opened:

```
skyward: ro.vendor.build.security_patch speaks for the configuration's 2099-12-31, not the installed system's 2025-08-01
skyward: the data partition is open: f2fs at /data
skyward: the data partition stays closed: ... it was made by key format generation 4 ...
```

## When something goes wrong

**The data partition stays closed.** Read the `generation N` in the log line
above: the key was made by a build of that generation, and the recovery has to
speak for it. That is what `build_release` and `build_patch` in
`devices/nuwa.toml` are for.

**It asks for a password.** The users' own files are behind their screen lock;
the password is the one the phone boots with. The partition itself opens
without it.

**Skyward is gone after a system update.** Turn on **Flash Skyward again** in
Settings, or flash the image again; a stock OTA rewrites the recovery partition.

**Nothing boots, but the phone is fine.** Hold power and volume up for recovery,
or power and volume down for fastboot. With no system in super, Skyward still
starts with its screen, keys, adb and fastbootd — enough to install one.

**adb reaches the storage without authorization.** This is by design: adb in
Skyward is a root shell that asks nothing. Anyone with the phone and a cable
reaches its storage while it is in recovery.

## Building

The build needs the stable Rust that `rust-toolchain.toml` names, its
`aarch64-linux-android` target, the Android NDK r29 and
[cargo-ndk](https://github.com/bbqsrc/cargo-ndk). The image is built on the
stock recovery of the build `devices/nuwa.toml` names, taken from Xiaomi's own
update package and checked against its SHA-256. Commands, tests and the preview
tool are in [docs/BUILDING.md](docs/BUILDING.md).

## Where it comes from

Skyward is [kirklin/skyward](https://github.com/kirklin/skyward), by kirklin and
its contributors; the port to this phone and its upkeep are by **xiuxiu391**,
on top of upstream 0.3.0. Licensed under
[GPL-3.0-or-later](LICENSE).
