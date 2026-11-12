![Skyward Recovery](assets/hero.jpg)

# Skyward for nuwa

English | [简体中文](README.zh-CN.md)

Skyward is a recovery written in Rust, from scratch. This fork is dedicated to
one phone: the **Xiaomi 13 Pro** (codename `nuwa`). For the other devices the
upstream project supports, see [kirklin/skyward](https://github.com/kirklin/skyward).

![Skyward running on the Xiaomi 13 Pro: About, home, settings and the file manager](docs/overview.webp)

<p align="center">
  <img src="assets/home.png" width="30%" alt="Home">
  <img src="assets/settings-dark.png" width="30%" alt="Settings in dark mode">
  <img src="assets/time-zone.png" width="30%" alt="Time zone">
</p>

## Features

- Decrypt the data partition
- Install zips, system update packages and partition images, from internal storage, a USB drive or adb sideload
- Online system updates
- Backup and restore
- Wipe data and factory reset
- Mount partitions, repair and format filesystems
- File manager and terminal
- WLAN
- MTP file transfer, adb and fastbootd
- Reboot to system, recovery, fastboot, bootloader or EDL, and switch slots
- Light and dark themes; Simplified Chinese, Traditional Chinese and English

## Supported device

| Device | Codename | Tested system version |
| --- | --- | --- |
| Xiaomi 13 Pro | `nuwa` | OS4.0.24.0.XEOCNXM (Android 17) |

The recovery image is built on the stock recovery of OS3.0.312.0.WMBCNXM, the
build `devices/nuwa.toml` names.

## Installing

Skyward needs an unlocked bootloader. Download the zip from this repository's
[Releases](https://github.com/pubglite55/skyward-for-nuwa/releases), which are
published from tags named `nuwa-v*`. Each file is named
`skyward-nuwa-<version>-<stock build>`. Check it against `SHA256SUMS.txt`,
unzip it, and flash the image inside to the recovery partition of the slot the
phone runs from, with the phone in fastboot mode:

```sh
unzip skyward-nuwa-v1.5-OS3.0.312.0.WMBCNXM.zip
fastboot flash recovery skyward-nuwa-v1.5-OS3.0.312.0.WMBCNXM.img
fastboot reboot recovery
```

Afterwards a newer image, unzipped, installs from Skyward's own Install page. With
**Flash Skyward again** on, a system update or zip that writes over the recovery
partitions gets Skyward back in them.

Without a system in super, Skyward still starts, with its screen and keys, adb,
`adb sideload` and fastbootd, enough to install one. Touch, WLAN and decryption run
from the installed system and come back with it.

adb in Skyward is a root shell that needs no authorization on the phone: anyone
with the phone and a cable can reach its storage while it is in recovery.

## Decryption

The data partition opens on demand: the file manager opens it when it is
entered, and an update that needs `/data` opens it before reading it. Where no
screen lock is set it opens by itself; where one is, the page asks for it. The
metadata key is wrapped by the phone's hardware and needs nothing typed.

A key is measured against the build it was made under, and a key made by a
newer build than the one installed is refused until it is renewed — a renewal
that a key of a newer build never gets. This phone's data was made by HyperOS 4
(Android 17, patch level 2099-12-31), so `devices/nuwa.toml` names that build:

```toml
build_release = "17"
build_patch = "2099-12-31"
```

The patch level is reported for the system's properties and the vendor's alike,
because the vendor build is the one a key is measured against. Should the phone
ever be reset from a newer system, those two lines have to name that system's
build. The log then says which generation the key was made in, and whether the
partition opened:

```
skyward: ro.vendor.build.security_patch speaks for the configuration's 2099-12-31, not the installed system's 2025-08-01
skyward: the data partition is open: f2fs at /data
skyward: the data partition stays closed: ... it was made by key format generation 4 ...
```

## Building

Skyward builds with the stable Rust that `rust-toolchain.toml` names, with its
`aarch64-linux-android` target, the Android NDK r29 and
[cargo-ndk](https://github.com/bbqsrc/cargo-ndk). The recovery service is built for
the phone, and the packer for the computer:

```sh
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward
cargo build --release -p skyward-pack
```

The packer takes the stock recovery partition out of the build's full update
package on Xiaomi's server, reading only the part of the package the partition is
in, and checks its SHA-256 against the device file:

```sh
target/release/skyward-pack stock --config devices/nuwa.toml --out target/stock/nuwa/recovery.img
```

Then it builds the image on top of it:

```sh
target/release/skyward-pack recovery \
  --stock target/stock/nuwa/recovery.img \
  --binary target/aarch64-linux-android/release/skyward \
  --config devices/nuwa.toml \
  --font resources/fonts/MiSansVF.ttf \
  --mono resources/fonts/KirkMono-Regular.ttf \
  --mono-license resources/fonts/KirkMono-OFL.txt \
  --out target/skyward-nuwa.img
```

Tests and lints run on the computer; what runs only on the phone is linted for it:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo ndk -t arm64-v8a --platform 30 clippy -p skyward -p skyward-install -- -D warnings
```

`skyward-preview` draws the pages on the computer, as PNG files, without a phone:
each page whole, as a long screenshot shows it, in the states it can be in, and the
screen at moments of touches and motion. `--lang` picks the language by a locale
such as `en` or `zh-TW`:

```sh
cargo run --release -p skyward-preview -- target/preview --lang en
```

## License

[GPL-3.0-or-later](LICENSE)
