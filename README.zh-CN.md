![Skyward Recovery](assets/hero.jpg)

# Skyward for nuwa

[English](README.md) | 简体中文

Skyward 是用 Rust 从零编写的 Android recovery：没有 TWRP 的框架，不基于 AOSP recovery，也没有 Java。它像原厂 recovery 一样从 recovery 分区启动，自己绘制界面，做 recovery 该做的事 —— 打开 data 分区、安装刷机包、备份分区、清除数据、挂载分区，以及把存储交给电脑。

这个仓库把它带到一台手机上：**小米 13 Pro**（代号 `nuwa`），基于上游 [kirklin/skyward](https://github.com/kirklin/skyward) 0.3.0。移植与维护：**[xiuxiu391](https://github.com/xiuxiu391)**。

<table>
  <tr>
    <td align="center"><img src="docs/images/home.png" width="230" alt="主页"></td>
    <td align="center"><img src="docs/images/decrypt.png" width="230" alt="解密"></td>
    <td align="center"><img src="docs/images/install.png" width="230" alt="安装"></td>
  </tr>
  <tr>
    <td align="center">主页：手机状态与全部入口</td>
    <td align="center">解密：硬件包裹的密钥，五步进度</td>
    <td align="center">安装：内部存储 / U 盘 / 电脑</td>
  </tr>
  <tr>
    <td align="center"><img src="docs/images/files.png" width="230" alt="文件"></td>
    <td align="center"><img src="docs/images/backup.png" width="230" alt="备份"></td>
    <td align="center"><img src="docs/images/settings.png" width="230" alt="设置"></td>
  </tr>
  <tr>
    <td align="center">文件：内部存储、U 盘与各分区</td>
    <td align="center">备份：每部分单独记录、整体校验</td>
    <td align="center">设置：主题、语言、时区、息屏</td>
  </tr>
</table>

> 这些图由仓库里的预览工具按本机型的配置绘制（`cargo run -p skyward-preview`），不是从别的机器上截的。

## 它能做什么

- **打开 data 分区**：按手机自己的分区描述启动它的密钥程序（监听、keymint、gatekeeper、weaver、安全元件等），取硬件包裹的密钥；支持文件级加密与锁屏保护的用户文件。
- **安装**刷机包、完整系统更新包和分区镜像，来源可以是内部存储、U 盘或 `adb sideload`。
- **备份与恢复**分区和文件，每个备份都会校验。
- **清除数据**，格式化和修复文件系统。
- **挂载**当前槽位的分区，切换槽位。
- **文件管理与终端**：文件管理器、shell 和内核日志。
- **把存储交给电脑**（MTP），同时提供 adb 和 fastbootd。
- **连接 WLAN**，记住了密码的会在开机时自己连上。
- **无线传输**：同一网段的电脑用浏览器打开 `http://手机地址:8080/`，列目录、下载、拖放上传。
- **网络下载**：输入直链即下到 `Download/`，可续传；地址太长时用另一台手机扫页面上的码（8081）输入，或读 `link.txt`。
- **双系统**：在 data 分区里养第二个系统。
- **自我更新**：从自己的 Releases 安装新版本。
- 浅色与深色主题；简体中文、繁体中文和英文；音量键全程可操作。

## 这一版带来的

**data 打不开时，内存顶内部存储。** 分区被清空、还没被系统建过，或者就是打不开时，恢复会拿一块内存挂到 `/sdcard`（可用内存的 3/4，且不少于 8 GB，上限而非预分配），建好 `Download` 和 `Pictures/Screenshots`。于是「内部存储」照常可写：无线传输、MTP、文件管理都能把刷机包送进来，送进来就能刷。重启即清空，页面会说明这一点。正常开机不启用（分区里有文件系统就不碰）。

**格式化 data 只做它能安全做的部分。** 清空分区、丢掉旧密钥、建好文件系统；密钥留给系统下次开机自己建 —— 这颗安全世界不接受恢复替它生成密钥（请求下去整机重启，两次复现）。所以格式化之后：刷入系统、开机，data 就回来了。

**密钥程序从手机自己的分区描述来。** 读 `/vendor`、`/odm`、`/system_ext`、`/system/etc/init/*.rc`，按 init 的语义启动监听、keymint、gatekeeper、目录服务和启动控制；配置里同类的项让位。换系统版本、换移植 ROM，都能对上。

**跨代密钥。** `devices/nuwa.toml` 声明 `build_release = "17"`、`build_patch = "2099-12-31"`，同时写系统属性和 vendor 属性：数据由更新的系统写出时，keymint 不会拿 `-62` 把恢复挡在外面。

**其余是这个分支自己的**：无线传输 8080、下载页二维码 8081 与 `link.txt`、WLAN MAC 记住或生成、锁屏密码记住、触摸屏随面板唤醒、安装前把占用的分区强制摘下。

## 在这台手机上验证过什么

| | 状态 |
| --- | --- |
| 启动、界面、触摸、音量键、截图 | 已验证 |
| 两槽刷入后启动 | 已验证（v2.0.1：`nuwa 2.0.1 recovery up`，misc 为空时直接进首页） |
| 打开 data 分区 | 已验证（密钥由 Android 17 写出，手机运行 16） |
| 密钥程序按分区描述启动 | 已验证（qseecomd / ssgtzd / keymint / hwservicemanager / gatekeeper / boot 六个都起） |
| adb、fastbootd、MTP | 已验证 |
| 无线传输 8080、下载页 8081 | 构建与预览通过 |
| 内存顶内部存储（3/4，≥8 GB） | 已验证：`tmpfs 8.0G /sdcard`，`adb push` 进 `Download/` 成功 |
| 安装、备份与恢复、双系统、在线更新 | 继承自上游，尚未在此机型逐项实测 |

## 刷入

只需要解锁的 bootloader。从 [Releases](https://github.com/pubglite55/skyward-for-nuwa/releases) 下载 zip（由 `nuwa-v*` 标签发布，文件名形如 `skyward-nuwa-<版本>-<原厂版本>`），用 `SHA256SUMS.txt` 核对，解压后把镜像刷进两个槽位：

```sh
unzip skyward-nuwa-v2.0.1-OS3.0.312.0.WMBCNXM.zip
fastboot flash recovery_a skyward-nuwa-v2.0.1-OS3.0.312.0.WMBCNXM.img
fastboot flash recovery_b skyward-nuwa-v2.0.1-OS3.0.312.0.WMBCNXM.img
fastboot reboot recovery
```

之后新版本解压后可以直接在 Skyward 的「安装」页刷入。打开 **自动重新刷入 Skyward** 之后，系统更新或覆盖 recovery 分区的刷机包会把 Skyward 刷回去。

刷 recovery 不会改动系统，OTA 也照常可用；要回到原厂 recovery，刷回原厂镜像即可。

## 页面

| 页面 | 用途 |
| --- | --- |
| 主页 | 手机状态，以及通往其它页面的入口 |
| 解密 | 说明数据分区为什么还关着；需要时提供解锁锁屏的地方 |
| 文件 | 文件管理器，覆盖内部存储、U 盘和当前槽位的分区 |
| 安装 | 刷机包与镜像：从某个位置、U 盘或 `adb sideload` |
| 备份 | 把分区和文件备份，以及从备份恢复 |
| 恢复 | 从备份里挑几部分写回去 |
| 挂载 | 当前槽位的分区：挂载、卸载、格式化、修复 |
| 清除 | 数据、缓存，以及手机自己的日志 |
| 双系统 | 在 data 分区里安装、切换、移除第二个系统 |
| 无线传输 | 把内部存储交给同一网段的浏览器：列目录、下载、上传 |
| 网络下载 | 从直链下载到 `Download/`，可续传；二维码或 `link.txt` 送地址 |
| 高级 | 终端、内核日志、修复 SELinux、文件系统、更新、无线传输开关 |
| 设置 | 主题、语言、时区、息屏时间、振动、文件传输 |
| 重启 | 系统、recovery、fastboot、bootloader、EDL，以及启动哪个槽位 |
| 关于 | 版本、手机信息、项目来源 |

## 解密

数据分区是**按需打开**的：进入文件管理时打开，需要读写 `/data` 的安装也会先打开它。没有设置锁屏时自动打开，设置了锁屏时会询问。metadata 的密钥由手机硬件包裹，不需要输入任何东西。

密钥按照**它被造出来时的构建**来度量，比当前系统更新的密钥会被拒绝使用，直到续期 —— 而更新构建的密钥永远得不到续期。这台手机的数据由 HyperOS 4（Android 17，补丁级 2099-12-31）写出，所以 `devices/nuwa.toml` 声明了那个构建：

```toml
build_release = "17"
build_patch = "2099-12-31"
```

补丁级同时报给系统属性和 **vendor** 属性，因为密钥是对着 vendor 构建度量的。如果手机以后被更新的系统重置过，这两行要改成那个系统的构建。日志会写明密钥的代际与分区是否打开：

```
skyward: ro.vendor.build.security_patch speaks for the configuration's 2099-12-31, not the installed system's 2025-08-01
skyward: the data partition is open: f2fs at /data
```

## 出问题时

**数据分区一直关着。** 看日志里的 `generation N`：密钥由那个代际的构建造出，recovery 必须为那个构建说话 —— 这就是上面的 `build_release` / `build_patch`。

**分区是空的、什么都打不开。** 恢复会把内存挂成内部存储，先刷系统：刷完开机，系统自己把 data 建起来。

**它要求输入密码。** 那是用户个人文件的锁屏密码，就是开机时用的那个；分区本身不需要密码就能打开。

**系统更新之后 Skyward 没了。** 原厂 OTA 会重写 recovery 分区。在设置里打开 **自动重新刷入 Skyward**，或者重新刷一次镜像。

**开不了机，但手机没坏。** 电源 + 音量上进入 recovery，电源 + 音量下进入 fastboot。super 里没有系统时 Skyward 依然能启动，屏幕、按键、adb 和 fastbootd 都在，足够装回一个系统。

**装包时报分区忙。** 已经不报了：恢复自己启动的密钥、触摸、WLAN 程序占着分区时，它把挂载点直接摘下再写，日志里是 `came off the tree at once`。

**电脑上看不见手机。** 设备管理器里明明有 `Android Composite ADB Interface`，`adb devices` 却是空的：电脑上那个常驻的 adb 服务端占着设备句柄，它自己的日志里是 `Failed to open device handle ... Error [5]: 拒绝访问`。把 adb 进程全部结束再起一次就好，手机不用动：

```sh
taskkill /F /IM adb.exe
adb start-server
```

**开机日志里说 WLAN 没起来。** 那是 `the WLAN did not come up: ... No such device`：驱动模块刚加载完、网卡还没注册好，开机那一次自检落空了。`wlan0` 随后就在，WLAN 页的扫描和连接照常。

**adb 不用授权就能访问存储。** 这是设计如此：Skyward 里的 adb 是不需要确认的 root shell。手机处于 recovery 时，任何人拿到手机和数据线都能访问其中的存储。

## 构建

需要 `rust-toolchain.toml` 指定的稳定版 Rust、它的 `aarch64-linux-android` 目标、Android NDK r29 和 [cargo-ndk](https://github.com/bbqsrc/cargo-ndk)。镜像基于 `devices/nuwa.toml` 指定的原厂 recovery 构建，从小米自己的更新包里取出并核对 SHA-256。命令、测试和预览工具见 [docs/BUILDING.md](docs/BUILDING.md)。

## 来源

Skyward 来自 [kirklin/skyward](https://github.com/kirklin/skyward)，作者 kirklin 及其贡献者；小米 13 Pro（`nuwa`）的移植与维护由 **xiuxiu391** 完成，代码基于上游 0.3.0。许可证为 [GPL-3.0-or-later](LICENSE)。
