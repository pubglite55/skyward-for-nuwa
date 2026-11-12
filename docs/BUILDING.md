# Building Skyward for nuwa

The build needs the stable Rust that `rust-toolchain.toml` names, its
`aarch64-linux-android` target, the Android NDK r29 and
[cargo-ndk](https://github.com/bbqsrc/cargo-ndk).

## The image

The recovery service is built for the phone, and the packer for the computer:

```sh
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward-system-init --target-dir target/system-init
cargo build --release -p skyward-pack
```

The packer takes the stock recovery partition out of the build's full update
package on Xiaomi's server, reading only the part of the package that partition
is in, and checks its SHA-256 against the device file:

```sh
target/release/skyward-pack stock --config devices/nuwa.toml --out target/stock/nuwa/recovery.img
```

Then it builds the image on top of it. `--system-init` is the second system's
first program, which the Dual boot page runs:

```sh
target/release/skyward-pack recovery \
  --stock target/stock/nuwa/recovery.img \
  --binary target/aarch64-linux-android/release/skyward \
  --system-init target/system-init/aarch64-linux-android/release/system-init \
  --config devices/nuwa.toml \
  --font resources/fonts/MiSansVF.ttf \
  --mono resources/fonts/KirkMono-Regular.ttf \
  --mono-license resources/fonts/KirkMono-OFL.txt \
  --out target/skyward-nuwa.img
```

## Tests and lints

Tests and lints run on the computer; what runs only on the phone is linted for
it:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo ndk -t arm64-v8a --platform 30 clippy -p skyward -p skyward-install -- -D warnings
```

## The pages, without a phone

`skyward-preview` draws the pages on the computer, as PNG files: each page
whole, as a long screenshot shows it, in the states it can be in, and the screen
at moments of touches and motion. `--lang` picks the language by a locale such
as `en` or `zh-TW`:

```sh
cargo run --release -p skyward-preview -- target/preview --lang en
```

## Releasing

A tag named `nuwa-v*` builds this phone's image alone and publishes it as a
release: `.github/workflows/release-nuwa.yml`. Every other tag goes through
upstream's `build.yml`, which builds every device its configuration names — this
repository names one.
