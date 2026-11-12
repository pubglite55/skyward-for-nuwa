![Skyward Recovery](assets/hero.jpg)

# Skyward for nuwa

[English](README.md) | 简体中文

Skyward 是使用 Rust 编写的 recovery，从零开始编写。这个分支只服务一台手机：**小米 13 Pro**（代号 `nuwa`）。上游项目支持的其他机型见 [kirklin/skyward](https://github.com/kirklin/skyward)。

![在小米 13 Pro 上运行的 Skyward：关于、主页、设置和文件管理](docs/overview.webp)

<p align="center">
  <img src="assets/home.png" width="30%" alt="首页">
  <img src="assets/settings-dark.png" width="30%" alt="深色模式下的设置">
  <img src="assets/time-zone.png" width="30%" alt="时区">
</p>

## 功能

- 解密 data 分区
- 安装刷机包、系统更新包和分区镜像，支持内部存储、U 盘和 adb sideload
- 在线系统更新
- 备份和恢复
- 清除数据、恢复出厂设置
- 挂载分区，修复和格式化文件系统
- 文件管理和终端
- 连接 WLAN
- MTP 文件传输、adb 和 fastbootd
- 重启到系统、recovery、fastboot、bootloader 和 EDL，切换启动槽位
- 浅色和深色模式，简体中文、繁体中文和英文

## 支持的机型

| 机型 | 代号 | 测试的系统版本 |
| --- | --- | --- |
| 小米 13 Pro | `nuwa` | OS4.0.24.0.XEOCNXM（Android 17） |

镜像基于原厂 OS3.0.312.0.WMBCNXM 的 recovery 构建，即 `devices/nuwa.toml` 中指定的版本。

## 安装

Skyward 需要已解锁的 bootloader。从本仓库的 [Releases](https://github.com/pubglite55/skyward-for-nuwa/releases) 下载 zip（由 `nuwa-v*` 标签发布），文件名形如 `skyward-nuwa-<版本>-<原厂版本>`。使用 `SHA256SUMS.txt` 核对并解压之后，在手机处于 fastboot 模式的时候，把其中的镜像刷入当前运行槽位的 recovery 分区：

```sh
unzip skyward-nuwa-v1.5-OS3.0.312.0.WMBCNXM.zip
fastboot flash recovery skyward-nuwa-v1.5-OS3.0.312.0.WMBCNXM.img
fastboot reboot recovery
```

之后可以在 Skyward 自己的“安装”页面安装解压出来的新版镜像。打开 **自动重新刷入 Skyward** 之后，系统更新或者刷机包覆盖 recovery 分区的时候，Skyward 会重新刷回这些分区。

super 中没有系统的时候，Skyward 仍然可以启动：屏幕和按键、adb、`adb sideload` 和 fastbootd 都能使用，足够装回一个系统。触摸、WLAN 和解密依赖已安装系统中的程序，系统装回之后随之恢复。

Skyward 中的 adb 是 root shell，手机上无需授权：手机处于 recovery 的时候，任何人拿到手机和数据线都能访问其中的存储。

## 解密

数据分区是**按需打开**的：进入文件管理时打开，需要读写 `/data` 的更新安装也会先打开它。没有设置锁屏时自动打开，设置了锁屏时会询问。metadata 的密钥由手机硬件包裹，不需要输入任何东西。

密钥按照**它被造出来时的构建**来度量，比当前系统更新的密钥会被拒绝使用，直到续期 —— 而更新构建的密钥永远得不到续期。这台手机的数据由 HyperOS 4（Android 17，补丁级 2099-12-31）写出，所以 `devices/nuwa.toml` 声明了那个构建：

```toml
build_release = "17"
build_patch = "2099-12-31"
```

补丁级同时报给系统属性和 **vendor** 属性，因为密钥是对着 vendor 构建度量的。如果手机以后被更新的系统重置过，这两行就要改成那个系统的构建。日志里会写明密钥的代际，以及分区是否打开：

```
skyward: ro.vendor.build.security_patch speaks for the configuration's 2099-12-31, not the installed system's 2025-08-01
skyward: the data partition is open: f2fs at /data
skyward: the data partition stays closed: ... it was made by key format generation 4 ...
```

## 构建

Skyward 使用 `rust-toolchain.toml` 指定的稳定版 Rust 和它的 `aarch64-linux-android` 目标、Android NDK r29 和 [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) 构建。recovery 服务针对手机构建，打包工具针对电脑构建：

```sh
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward
cargo build --release -p skyward-pack
```

打包工具从小米服务器上这个版本的完整更新包中取出原厂 recovery 分区，只读取这个分区所在的部分，并且按照设备配置文件核对 SHA-256：

```sh
target/release/skyward-pack stock --config devices/nuwa.toml --out target/stock/nuwa/recovery.img
```

然后在它的基础上构建镜像：

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

测试和代码检查在电脑上运行；只在手机上运行的代码按照手机目标检查：

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo ndk -t arm64-v8a --platform 30 clippy -p skyward -p skyward-install -- -D warnings
```

`skyward-preview` 无需手机，在电脑上把页面绘制成 PNG 文件：每个页面完整绘制成长截图的样子，覆盖它可能处于的各种状态，另外还有触摸和动画过程中某些时刻的屏幕。`--lang` 按照地区代码选择语言，例如 `en` 或者 `zh-TW`：

```sh
cargo run --release -p skyward-preview -- target/preview --lang en
```

## 许可证

[GPL-3.0-or-later](LICENSE)
