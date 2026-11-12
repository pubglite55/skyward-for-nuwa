# 交接文档 · Skyward for nuwa

写给接手这个仓库的 AI / 开发者。目标是让你在**不改动任何代码**的前提下，先准确知道「这是什么、现在到哪一步、哪些坑已经踩过、下一个动作是什么」。

- 仓库：https://github.com/pubglite55/skyward-for-nuwa
- 上游：https://github.com/kirklin/skyward（本仓库基于 **0.2.0**）
- 当前最新 Release：**nuwa-v1.5**（2026-11-12 重建，含自更新修复）
- 当前 `main` 头：`3a0a54d`（写这份文档时；以实际为准）
- 设备：小米 13 Pro，代号 **nuwa**，型号 2210132C，SM8550（骁龙 8 Gen 2）

---

## 1. 这是什么

Skyward 是**用 Rust 从零写的 Android recovery**（不是 TWRP，不基于 AOSP recovery，无 Java）。它从 recovery 分区启动，自带界面与触摸，做 recovery 的事：打开 data 分区、刷包、备份、清除、挂载、MTP/终端。

本仓库是**单机型分支**：只服务 nuwa。上游支持 20+ 机型；这里 `devices/` 只有 `nuwa.toml`，CI 也只构建这一个机型。

**它不是**：
- 不是 ROM / 内核项目（不碰 system、vendor、boot）
- 不是 TWRP 的皮肤或插件
- 不是通用跨机型发行版（虽然代码通用，但配置与验证只针对 nuwa）

---

## 2. 目录与 crate 职责

```
crates/
  skyward/              recovery 主程序：启动序列、UI 主循环、服务管理、解密调度、更新
  skyward-ui/           界面与状态机（App、页面、绘制、输入），可在电脑上预览
  skyward-config/       devices/*.toml 解析（DeviceConfig::parse / spoken_for）
  skyward-crypto/       解密：keymint 客户端、FBE、用户密钥、mount
  skyward-binder/       binder 客户端（parcel/driver/service）：与 hwservicemanager/keymint 对话
  skyward-sys/          libc/syscall 封装：prop、ueventd、block、misc、fastboot…
  skyward-block/        块设备、分区、dm、挂载辅助
  skyward-bootimg/      boot/recovery 镜像解析与打包（含 LZ4 标准帧 + legacy 帧）
  skyward-pack/         PC 侧打包器：取原厂 recovery 分区 → 叠加 Skyward → 输出 .img
  skyward-install/      刷机包解析（zip/OTA/镜像）、签名、写入、upgrade（自更新写分区）
  skyward-backup/       备份与恢复
  skyward-systems/      双系统（data 分区里养第二个系统）
  skyward-system-init/  第二个系统的第一个程序
  skyward-vab/          A/B 槽位、boot control
  skyward-info/         设备信息（电量、型号、VERSION）
  skyward-mtp/          MTP 文件传输
  skyward-net/          HTTP、GitHub release 解析（自更新用）
  skyward-modules/      模块加载（按 libmodprobe 规则）
  skyward-lp/           逻辑分区（super）辅助
  skyward-preview/      电脑上把页面渲染成 PNG（无手机也能看界面）
  skyward-files/        文件操作（复制/移动/重命名）
```

其它关键位置：

```
devices/nuwa.toml              唯一的机型配置（见 §3）
.github/workflows/build.yml    每次 push：lint+test，并构建 devices/ 下的机型
.github/workflows/release-nuwa.yml  tag nuwa-v*：只构建 nuwa 并发布 Release
.github/workflows/build-nuwa-twrp.yml  手动：在 TWRP 底座上构建（实验用，见 §9）
docs/BUILDING.md               构建命令、测试、预览、发版规则
docs/overview.webp             README 配图
README.md / README.zh-CN.md    面向用户的说明（英文 / 中文）
```

---

## 3. 机型配置 `devices/nuwa.toml`

关键字段（改动前先读注释，里面写了每个值的来源）：

| 字段 | 值 | 含义 |
| --- | --- | --- |
| `[device] codename` | `nuwa` | 机型代号；镜像内嵌，自更新校验用 |
| `[device] build_release` | `"17"` | **解密关键**：对外声称的系统版本（见 §4） |
| `[device] build_patch` | `"2099-12-31"` | **解密关键**：同时报给 `ro.build.version.security_patch` 与 `ro.vendor.build.security_patch` |
| `[display]` | 1440×3200 / 560dpi | 面板模式 |
| `[modules] load` | `fts_touch_spi` | 触控驱动（ST FTS over SPI） |
| `[[owners]]` | 触控节点属主 | 来自 odm 的 touchfeature rc |
| `[wlan]` | kiwi_v2 | WCNSS 配置与 MAC |
| `[stock] version` | `OS3.0.312.0.WMBCNXM` | 底座原厂版本；`package` 是小米官方 OTA 直链，`recovery` 是 recovery 分区的 SHA-256 |
| `[backup] efs` | modemst1/2, fsg, fsc, persist | 基带/EFS 备份分区 |
| `[wipe] logs` | /data/miuilog 等 8 项 | 清数据时一并清掉的日志目录 |

**解密服务栈**（顺序即启动顺序，缺一不可）：

| 服务 | 路径 | 就绪判据 |
| --- | --- | --- |
| qseecomd | `/vendor/bin/qseecomd` | `vendor.sys.listeners.registered` |
| keymint | `/vendor/bin/hw/android.hardware.security.keymint-service-qti` | `android.hardware.security.keymint.IKeyMintDevice/default` |
| hwservicemanager | `/mnt/system_ext/bin/hwservicemanager` | `hwservicemanager.ready` |
| gatekeeper | `/vendor/bin/hw/android.hardware.gatekeeper@1.0-service-qti` | `android.hardware.gatekeeper@1.0::IGatekeeper/default` |
| boot control | `/vendor/bin/hw/android.hardware.boot@1.2-service` | `android.hardware.boot@1.2::IBootControl/default` |

另有两个触控相关服务（`/vendor/bin/sh`、`/odm/bin/touch_report`、`/odm/bin/touchsensor`）。

---

## 4. 解密（本项目最重要的知识）

### 模型

1. `/metadata` 的密钥是**硬件包裹**的（`wrappedkey_v0`），由 TEE 里的 keymint 解包 —— **不需要用户输入任何东西**。
2. 拿到 metadata 密钥后挂载 `/data`，再按用户解开 FBE：无锁屏则自动；有锁屏则读该用户文件时询问密码（第二层）。
3. data 分区是**按需打开**的：进「文件」页、进解密页、或 OTA 需要 data 时才发起（`Effect::OpenData`）。**开机日志里没有 `the data partition is open` 是正常的**，验证时必须手动进一次文件页（或看后续是否有 OTA 需求）。

### 根因（2026-11-12 定位，实机验证）

keymint 用 **vendor 构建的补丁级**（`ro.vendor.build.security_patch`）来度量密钥代际：

- 本机数据由 Android 17 / 补丁 `2099-12-31` 的系统写出 → 密钥 blob 是 **KeyMint 格式 generation 4**（头部 `85 1a "KMKD" 1b 00000004`）
- 当时手机跑 Android 16，vendor 分区是 Android 13 → vendor 补丁报 `2025-08-01`
- 密钥「比当前构建新」→ `begin()` 返回 **-62 KEY_REQUIRES_UPGRADE** → `upgradeKey` 返回 **-38 UNIMPLEMENTED**（续期只能向前，不能向后）

**修复**：`build_release`/`build_patch` 现在同时写三个属性（`main.rs` 的 `adopt_installed_version` → `DeviceConfig::spoken_for`）：

```
ro.build.version.release            = 17
ro.build.version.security_patch     = 2099-12-31
ro.vendor.build.security_patch      = 2099-12-31   ← 关键
```

**注意**：`ro.*` 属性只能设置一次，所以覆盖值必须在**第一次 set 时**就写入，不能先 set 成已安装系统的值再改（会 `init refused` 并中断启动）。

### 日志（排查只看这几行）

```
skyward: nuwa 1.5 recovery up, ...                                  ← 版本（SKYWARD_VERSION）
skyward: ro.vendor.build.security_patch speaks for the configuration's 2099-12-31, not the installed system's 2025-08-01
skyward: the data partition is open: f2fs at /data                  ← 成功
skyward: /dev/block/dm-8 holds f2fs at /data
skyward: the users whose own files are still closed: [0] → []
skyward: the data partition stays closed: ... it was made by key format generation 4 ...   ← 失败，带代际与指引
```

### 换系统 / 换机后的规则

- 若数据由一个**更新的系统**写出，`build_release`/`build_patch` 必须改成那个系统的构建（版本 + 补丁级），否则又是 -62/-38。
- 若数据由**更旧的系统**写出，保持现值即可（密钥更旧 → 允许向前续期）。
- 密钥代际**不可逆**：v3 安全世界读不了 v4 密钥。唯一解是刷回写出该密钥的系统（或让配置声称那个构建）。

### 已验证 / 未验证

| 项 | 状态 |
| --- | --- |
| 打开 data 分区（Android 17 写的数据 / 手机跑 16） | ✅ 实机验证，多次冷启动 |
| 无锁屏自动开、锁屏询问 | ✅ 无锁屏路径验证；锁屏路径未专门验 |
| 安装、MTP、fastbootd、WLAN、备份、双系统、在线更新 | ⬜ 继承自 0.2.0，**在本机型未实测** |

---

## 5. 自更新（本次新增）

- 查询地址：`crates/skyward-net/src/github.rs` 的 `LATEST` →
  `https://api.github.com/repos/pubglite55/skyward-for-nuwa/releases/latest`
- tag 解析：支持 `nuwa-v1.5`（本仓库）与 `v0.1.4`（上游）；**两段版本按 `1.5.0` 读**（semver 需要三段）
- 资产匹配：上游式 `skyward-<tag>-<codename>-*.zip`，或本仓库式 `skyward-<tag>-*.zip`（codename 在 tag 里）
- 自身版本：`skyward_info::VERSION` 优先取编译期环境变量 `SKYWARD_VERSION`，否则 `CARGO_PKG_VERSION`（0.2.0）。
  `release-nuwa.yml` 会用 `${GITHUB_REF_NAME#nuwa-v}` 传入，所以 **v1.5 的构建报告自己是 1.5**
- 写入：下载 zip → 校验 GitHub 给的 SHA-256 → `upgrade::unzip`（zip 内必须**只有一个 .img**）→ `upgrade::check`（比对镜像内嵌 codename）→ 写 recovery_a/b
- 已知：**v1.1–v1.4 的旧构建报告 0.2.0**，它们会看到 v1.5「更新」；更新到 v1.5 后即正常

---

## 6. 构建与 CI

| 工作流 | 触发 | 做什么 | 产物 |
| --- | --- | --- | --- |
| `build.yml` | 每次 push、`v*` tag、手动 | lint+test，然后按 `devices/*.toml` 构建每个机型（现在只有 nuwa） | artifact：`skyward-<sha>-nuwa-OS3.0.312.0.WMBCNXM.zip` |
| `release-nuwa.yml` | push tag `nuwa-v*` | 只构建 nuwa + 生成 SHA256SUMS + 发布 Release | Release 资产（zip + SHA256SUMS.txt） |
| `build-nuwa-twrp.yml` | 手动 | 在 TWRP 底座上构建（实验，见 §9） | artifact |

构建链（详见 `docs/BUILDING.md`）：

```sh
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward
cargo ndk -t arm64-v8a --platform 30 build --release -p skyward-system-init --target-dir target/system-init
cargo build --release -p skyward-pack
skyward-pack stock    --config devices/nuwa.toml --out stock/recovery.img   # 从官方 OTA 取原厂 recovery，校验 SHA-256
skyward-pack recovery --stock stock/recovery.img --binary .../skyward --system-init .../system-init \
  --config devices/nuwa.toml --font ... --mono ... --out out/skyward-nuwa.img
```

- `--system-init` 是 0.2.0 新增的**必填**参数（双系统首程序），漏了会报 unexpected argument。
- 打包器会拒绝“非原厂底座”，除非加 `--custom-base`（TWRP 底座实验用）。

---

## 7. 发版流程

1. 确认 `main` 上 CI 全绿，且**实机验证过**（至少：能启动、能进文件页打开 data）
2. 打 tag 并推：
   ```sh
   git tag nuwa-v1.5 HEAD && git push origin nuwa-v1.5
   ```
3. `release-nuwa.yml` 自动构建并发布；若**同名 Release 已存在**，它会 `gh release upload --clobber` 覆盖资产（不会新建）
4. 需要改说明：`gh release edit nuwa-v1.5 --notes-file notes.md`
5. **替换某个版本**的正确姿势：`git tag -d nuwa-v1.5 && git tag nuwa-v1.5 <sha> && git push -f origin nuwa-v1.5`
   —— 会触发一次新的构建并覆盖同名资产（已在 2026-11-12 用过两次）

**纪律（用户明确要求过）**：
- 没在真机验证过的构建**不许打 tag**、不许建 Release
- 实验性版本不发布；发过的坏版本要删（`gh release delete <tag> --cleanup-tag --yes`）
- 不要改我们仓库的名字（曾因腾位 fork 改名被叫停）

---

## 8. 开发环境与工具（这台机器）

- 平台：Windows + Git Bash；**WSL 已被清空，本地没有 Rust 工具链** → 一切编译走 GitHub Actions
- `adb` / `fastboot`：`./.tools/platform-tools/`（在 `C:/Users/L1872/Desktop/nuwa ios/`）
- `gh`：`./.tools/bin/gh.exe`；token 在 `work/t.txt`
- 代理：`http://127.0.0.1:7890`，**时有时无**：
  - `api.github.com` 直连可通（`HTTPS_PROXY= gh api ...`）
  - `github.com` 与 Release 下载需要代理
  - **Actions artifact 必须走代理 curl 才快**（直连 ~60 KB/s，代理 ~1.5 MB/s）：
    ```sh
    curl -s -L -x http://127.0.0.1:7890 --retry 2 --retry-all-errors -C - \
      -H "Authorization: Bearer $(cat work/t.txt)" -H "Accept: application/vnd.github+json" \
      -o out.zip "https://api.github.com/repos/pubglite55/skyward-for-nuwa/actions/artifacts/<id>/zip"
    ```
- 真机刷写：`adb reboot bootloader` → `fastboot flash recovery_a/b <img>` → `fastboot reboot recovery`
  - **`fastboot reboot recovery` 偶发弹回 fastboot**；可靠路径是 `fastboot reboot` → 进系统 → `adb reboot recovery`
- 取日志：`adb shell dmesg | grep 'skyward:'`
- 本地仓库路径：`C:/Users/L1872/Desktop/nuwa ios/work/skyward`（remote `pr` = 我们的 main）

---

## 9. 踩过的坑（照做能省几小时）

**解密相关**
1. **`ro.vendor.build.security_patch` 是胜负手**。只改 `ro.build.version.*` 不够（v1.5–v1.9 全在这上面白试）。
2. `ro.*` 只能 set 一次：覆盖值必须第一次就写（先写已装系统值会 `init refused` 并中止启动）。
3. **compat 路线对本机无效**：`android.security.compat`（keystore2 的 `IKeystoreCompatService`）需要 HIDL **Keymaster 4.x** 实例；nuwa 的 vendor 只有 AIDL keymint → 必然 `NAME_NOT_FOUND`。同理 TWRP 自带的 `libkm_compat.so` 是“旧 keymaster→新 keymint”的适配器，对我们没用。
4. **TWRP 底座实验**：能启动，但 skyward 拉起的 `keystore2` 立刻 SIGSEGV（环境变量补齐也没用），且它的 init 会自己起一个 keymint 抢服务名 → 现场有两个 keymint，实验数据不可信。**结论：解密不需要 TWRP 底座**，已回到原厂底座路线。
5. TWRP 能跨版本解密的原因是自带整套现代用户态 + 自己的客户端栈，不是“参数魔法”。

**代码层**
6. **libbinder 事务包头有构建标记**：recovery 构建写 `RECO`（`0x5245434F`），系统构建写 `SYST`（`0x53595354`）；不一致直接返回 `BAD_TYPE = -2147483647`。非原厂底座（TWRP）上所有调用都会被拒。已修：`skyward-binder` 收到 BAD_TYPE 时换标记重试并记住；服务目录方法号也是问出来的（4 还是 6）。
7. **LZ4 legacy 帧**：TWRP 的 ramdisk 用 legacy framing（magic + 每块长度），`lz4_flex` 只认标准帧。已在 `skyward-bootimg` 补 legacy 解码。
8. **semver 要三段**：`Version::parse("1.5")` 失败，需要补 `.0`。
9. `git add -A` 会把 `stock/`、`out/`（数百 MB）吞进提交 —— 已在 `.gitignore` 排除，提交前仍要 `git status` 看一眼。
10. Windows 上 git 会警告 LF→CRLF；`git add` 时用 `-c core.autocrlf=false` 可避免整文件“假改动”。

**CI / 发布**
11. `release-nuwa.yml` 曾把 `SHA256SUMS.txt` 写在仓库根、上传时却找 `out/SHA256SUMS.txt` → 重发同 tag 时 zip 传成功、校验文件失败、再走 `create` 撞同名整体失败。已修（`bd3c11b`）。
12. Release 的 zip 里**只能有一个 .img**（`upgrade::unzip` 会 `ensure!(len == 1)`）。
13. artifact 下载慢的锅在网络，不在 `gh`（见 §8）。

---

## 10. 已知限制 / 待办

| 优先级 | 事项 | 备注 |
| --- | --- | --- |
| P1 | 备份/还原全流程实测 | 用户最常用；目前只是“继承上游未实测” |
| P1 | 给上游提 PR：`adopt_installed_version` 漏 `ro.vendor.build.security_patch` | 对所有“密钥比固件新”的用户都有效 |
| P2 | 双系统（Dual boot）实测 | 会占用 data 分区，先备份 |
| P2 | 开机自动尝试解密（可选开关） | 现在按需解密，容易被误判“解密坏了” |
| P3 | 解密后文件页自动重列目录 | MTP 已自动跟随 `storage_open`，文件页需要手动退出重进 |
| — | 旧版本（≤v1.4）自更新会提示一次 v1.5 | 因旧构建报告 0.2.0，更新后即正常 |

---

## 11. 上手检查清单（新接手时按顺序做）

1. `gh run list --repo pubglite55/skyward-for-nuwa --limit 3`：确认最新构建绿
2. `gh release view nuwa-v1.5`：确认 Latest 资产（zip 约 43 MB + SHA256SUMS.txt）
3. 手机接上：`adb devices` / `fastboot devices`
4. 要验证解密：进 recovery → **点「文件」页** → `adb shell "mount | grep ' /data '"` 应看到 `dm-8 ... f2fs (rw,...,inlinecrypt)`；
   `dmesg | grep 'skyward:'` 应有 `the data partition is open: f2fs at /data`
5. 任何改动先 `git status`，提交后等 `build.yml` 绿；**不发版就不打 tag**

---

## 12. 常用命令速查

```sh
# 状态
gh run list --repo pubglite55/skyward-for-nuwa --limit 3
gh api repos/pubglite55/skyward-for-nuwa/actions/runs/<id>/jobs --jq '[.jobs[] | .name + ":" + (.conclusion // .status)]'

# 下载 artifact（走代理）
curl -s -L -x http://127.0.0.1:7890 -H "Authorization: Bearer $(cat work/t.txt)" \
  -H "Accept: application/vnd.github+json" -o a.zip \
  "https://api.github.com/repos/pubglite55/skyward-for-nuwa/actions/artifacts/<id>/zip"

# 刷机
adb reboot bootloader
fastboot flash recovery_a <img> && fastboot flash recovery_b <img>
fastboot reboot            # 再 adb reboot recovery（比 reboot recovery 可靠）

# 取证
adb shell dmesg | grep 'skyward:'
adb shell "mount | grep -E ' /data | /sdcard '"
adb shell "ls /sdcard/DCIM"
```

---

*本文档由 2026-11-12 的工作整理：解密根因定位与修复、仓库收敛为单机型、README 重写、自更新指向本仓库、v1.5 三次重建发布。*
