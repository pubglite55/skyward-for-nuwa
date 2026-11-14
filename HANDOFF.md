# 交接文档 · Skyward for nuwa

写给接手这个仓库的 AI / 开发者。目标是让你在**不改动任何代码**的前提下，先准确知道「这是什么、现在到哪一步、哪些坑已经踩过、下一个动作是什么」。

- 仓库：https://github.com/pubglite55/skyward-for-nuwa
- 上游：https://github.com/kirklin/skyward（本仓库已合并 **0.3.0**，代码基线 0.3.0）
- 当前最新 Release：**nuwa-v1.9**（2026-11-14 第三次发，`3b34ec9`，= 上游 0.3.0 + 本 fork 自有功能 + 密钥链容错 + 清理 + 署名 + 安装前强制摘除分区）；真机已验证启动、密钥链、解密、WLAN、缺程序容错、sideload 安装；备份 / 双系统未在合并后复测。
- 署名：上游 **Kirk Lin** / 本移植 **xiuxiu391**，README（中英）、关于页（`AUTHOR`）、许可证页（`LICENSE_ORIGINAL`）、发布说明都已写上。
- 当前发布分支 `fork/main` 头：与本轮发布同一提交；本地工作分支 `upstream-merge`
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

1. 确认**我们仓库**的 `fork/main` 上 CI 全绿，且**实机验证过**（至少：能启动、能进文件页打开 data）
   —— 注意：本地 `main` 分支追踪的是上游 `origin/main`，不是发布分支（见 §8）
2. 打 tag 并推到 `fork`：
   ```sh
   git tag nuwa-v1.5 HEAD && git push fork nuwa-v1.5
   ```
   **`origin` 指向上游 kirklin/skyward**；往那里推 tag 不会触发本仓库的发布工作流。所有推送都要显式写 `fork`。
3. `release-nuwa.yml` 自动构建并发布；若**同名 Release 已存在**，它会 `gh release upload --clobber` 覆盖资产（不会新建）
4. 需要改说明：`gh release edit nuwa-v1.5 --notes-file notes.md --repo pubglite55/skyward-for-nuwa`
5. **替换某个版本**的正确姿势：`git tag -d nuwa-v1.5 && git tag nuwa-v1.5 <sha> && git push -f fork nuwa-v1.5`
   —— 会触发一次新的构建并覆盖同名资产（已在 2026-11-12 用过两次）

**纪律（用户明确要求过）**：
- 没在真机验证过的构建**不许打 tag**、不许建 Release
- 实验性版本不发布；发过的坏版本要删（`gh release delete <tag> --cleanup-tag --yes --repo pubglite55/skyward-for-nuwa`）
- 不要改我们仓库的名字（曾因腾位 fork 改名被叫停）
- 所有 `gh` 命令都带 `--repo pubglite55/skyward-for-nuwa`：默认 remote 是 `origin`（上游），不写会操作错仓库

---

## 8. 开发环境与工具（这台机器）

**两个目录要分清**：仓库已迁到 `C:/Users/L1872/Desktop/nuwa recovery/skyward`；工具与凭据仍留在 `C:/Users/L1872/Desktop/nuwa ios/`。旧路径 `nuwa ios/work/skyward` 已不存在，旧版文档里的相对路径（`./.tools/`、`work/t.txt`）都以 `nuwa ios/` 为基准，从新仓库目录执行会失效 —— 下面一律写绝对路径。

- 平台：Windows + Git Bash；**WSL 已被清空，本地没有 Rust 工具链** → 一切编译走 GitHub Actions
- `adb` / `fastboot`：`C:/Users/L1872/Desktop/nuwa ios/.tools/platform-tools/`
- `gh`：`C:/Users/L1872/Desktop/nuwa ios/.tools/bin/gh.exe`；token 在 `C:/Users/L1872/Desktop/nuwa ios/work/t.txt`
- 代理：`http://127.0.0.1:7890`，**时有时无**：
  - `api.github.com` 直连可通（`HTTPS_PROXY= gh api ...`）
  - `github.com` 与 Release 下载需要代理
  - **Actions artifact 必须走代理 curl 才快**（直连 ~60 KB/s，代理 ~1.5 MB/s）：
    ```sh
    curl -s -L -x http://127.0.0.1:7890 --retry 2 --retry-all-errors -C - \
      -H "Authorization: Bearer $(cat 'C:/Users/L1872/Desktop/nuwa ios/work/t.txt')" \
      -H "Accept: application/vnd.github+json" \
      -o out.zip "https://api.github.com/repos/pubglite55/skyward-for-nuwa/actions/artifacts/<id>/zip"
    ```
- 真机刷写：`adb reboot bootloader` → `fastboot flash recovery_a/b <img>` → `fastboot reboot recovery`（`adb`/`fastboot` 不在 PATH，实际执行用 §12 的绝对路径形式）
  - **`fastboot reboot recovery` 偶发弹回 fastboot**；可靠路径是 `fastboot reboot` → 进系统 → `adb reboot recovery`
- 取日志：`adb shell dmesg | grep 'skyward:'`

**remote 与分支（改动前务必看清）**：

| 名称 | 指向 | 用途 |
| --- | --- | --- |
| `fork` | `pubglite55/skyward-for-nuwa` | **我们的仓库**；`fork/main` 是发布分支，tag 与推送都走这里 |
| `origin` | `kirklin/skyward` | **上游**，只读参考；**不要往这里推** |
| `aurora-pr` / `guobao` | 两个旁支仓库 | 实验 / 协作，与发版无关 |

本地分支名有误导性：
- 当前工作分支是 `upstream-merge`，虽追踪 `origin/main`，但 HEAD `dfe8f4e` 与 `fork/main` 完全一致，工作区干净。
- 本地 `main` 追踪的是**上游** `origin/main`（已落后 61 个提交），**不是**我们的发布分支；`git checkout main` 会切到上游代码。
- 本地 `pr`、`nuwa`、`aurora*` 等是历史分支。旧版文档写的「remote `pr` = 我们的 main」已不成立：`pr` 现在只是本地分支名，我们仓库的 remote 叫 `fork`。
- 结论：`git push` 一律显式写 `fork`；`gh` 命令一律显式带 `--repo pubglite55/skyward-for-nuwa`。

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
| P3 | 解密后文件页自动重列目录 | **代码层已修，待真机验证**（见下表）；原现象：MTP 已自动跟随 `storage_open`，文件页需要手动退出重进 |
| — | 旧版本（≤v1.4）自更新会提示一次 v1.5 | 因旧构建报告 0.2.0，更新后即正常 |

**本轮改动（CI 已过，实机未验）**：本机无 Rust 工具链，且这台手机当时 adb 协议层未建立（Windows 能看到设备、`adb devices` 为空）。这些改动已提交（`d297db9` 为代码与测试，其后为文档）并合并到 `fork/main`，分支 `fix/recovery-robustness` 保留；2026-11-12 的 `build.yml` 全绿（host clippy + `cargo test` + NDK clippy + 打包），每次推送都会产出 `skyward-<提交>-nuwa-OS3.0.312.0.WMBCNXM.zip`（artifact 保留 7 天）。**未刷机、未实机验证**，真机确认前不要打 tag。当日晚间追加的一批（表末三行）面向「不同系统版本的 fstab 都能解密」，同样只过了 CI。

| 文件 | 修了什么 |
| --- | --- |
| `crates/skyward/src/ui.rs` | P3：解密成功 / 解锁成功后重列文件页与存储选择页（不只判断栈顶页面）；自更新 zip 名只取 `file_name()` 再拼 `RUN` |
| `crates/skyward-net/src/github.rs` | tag 前缀由写死 `nuwa-` 改为通用剥离；`version_of` 支持前导 `v` 与预发布、拒绝四段；`assets` 缺省 / 空列表明确报错；拒绝含路径的资产名 |
| `crates/skyward-install/src/image.rs` | `flash()` 的 footer 迁移条件补上剩余空间判断，避免 `size - len - FOOTER_LEN` 无符号下溢导致 AVB footer 丢失 |
| `crates/skyward/src/main.rs` | 配置代为声明构建值时，不再要求 installed build.prop 必须提供同名属性 |
| `crates/skyward-net/src/release.rs` | 校验 `version` 不可穿越镜像 URL 路径；无 code 且无 latest 的回答不再被当成「服务器说没有更新」 |
| `crates/skyward-block/src/fstab.rs` | 解析改为只取前 5 列、容忍多余列（只有 4 列也接受）；此前多一列就整行丢弃，带 `metadata_encryption` 的 `/data` 行会凭空消失 |
| `crates/skyward-crypto/src/data.rs` | 解密不再依赖某一份 fstab：条目按 `metadata_encryption` 选（`fstab::encrypted`，原来固定取第一条，同一挂载点多条时会选中不带该参数的那条并报「未整盘加密」）；探测不出文件系统时回落到该条目而非第一条；`keydirectory` 缺省按 fs_mgr 的 `/metadata/vold/metadata_encryption`；`wrappedkey_v0` 与 `v1` 及以后都按硬件包装处理，其它后缀明确报错；表里没有任何整盘加密条目、或在原始分区上就能读出文件系统（说明分区是明文）时，直接挂载原分区；映射后读不出文件系统则明确报「密钥与分区不匹配」，不再退化成挂载错误 |
| `crates/skyward/src/wipe.rs` | 恢复出厂改为按 `metadata_encryption` 取条目判断 `rebuilt`（原来固定第一条，可能误判为不可重建而拒绝）；显式 reformat 时改用目标文件系统那条条目的参数去建文件系统 |
| `crates/skyward-block/src/fsck.rs` | 挂载前的检查补上 ext4 的 `e2fsck -p`（原来只支持 f2fs，data 为 ext4 的系统会直接失败） |
| `skyward-info` / `skyward-ui` / `skyward/src/ui.rs` | 开发构建（未设 `SKYWARD_VERSION`，版本显示 0.2.0）不再向 GitHub 查新版本，更新页写明「试验构建，不查找新版本」；此前会认为 `1.5` 比 `0.2.0` 新并可一键**降级**回 v1.5 |
| `crates/skyward-crypto/src/data.rs`、`skyward-ui`、`skyward-config` | 借鉴 pudding：跳过第二空间用户 999（其凭据由主用户代持，二次解锁可能卡住）；解密分 5 步上报进度，解密页画进度条；新增 `[data] weaver_first`（默认 true）——绑定 weaver 的 blob 打不开时，可改用同目录凭据服务守护的 `.secdis`+`.pwd` blob |
| `crates/skyward/src/wlan.rs`、`skyward-config`、`devices/nuwa.toml` | WLAN 固件里的校准文件（persist 的 `kiwi_v2/wlan_mac.bin`）标为「可选」：缺失时**在内存里按序列号派生一个本地管理地址**写到 `/etc/firmware`（不写 persist），记日志后继续启动。此前 persist 被清过的机器会一直报「无法开启 WLAN」 |
| `crates/skyward-net/src/serve.rs`（新）、`iface.rs`、`skyward-ui`、`crates/skyward/src/ui.rs` | 新增「无线传输」（借鉴 pudding）：高级 → 无线传输，开启后在同一网段的浏览器打开 `http://<手机地址>:8080/` 即可列目录、下载（支持 `Range` 续传）与拖放上传（`PUT`，先写 `.part` 再改名，断线不留半截文件）。地址取自 WLAN 接口的真实地址并每 2 秒跟随；`..` 与指向目录外的符号链接一律 403；进入本页会顺带开始解密，data 未打开时开关处于「等待中」，分区打开后自动开始，data 一旦关闭就停 |
| `crates/skyward-modules`、`crates/skyward-ui`、`crates/skyward-preview` | 内核模块按「本次启动已交给内核」去重：同一 `.ko` 被两个请求（或同一请求的两条依赖链）共享时只 insmod 一次，此前会被重复交给内核并触发 `EEXIST`；解密页在分区打开且没有用户仍需解锁时**自行返回**（回到打开它的页面），不再需要手动跳过 |
| `crates/skyward-ui`（`app`/`settings`/`pages`/`search`） | 文件管理器排序：高级 → 文件排序，可按名称 / 大小 / 修改时间，升降序，文件夹置顶；排序在界面侧完成，改选即重排已打开的列表，选择写入设置文件（`sort`），下次启动保留 |
| `crates/skyward-net/src/download.rs`、`crates/skyward/src/updates.rs`、`skyward-ui`、`crates/skyward/src/ui.rs` | 新增「网络下载」：高级 → 网络下载，输入 `http://` / `https://` 直链，按回车即下到内部存储 `Download/`；支持 `Range` 续传、可停止（已下部分保留，再下同一地址接着下）、同名文件自动加序号不覆盖。为此给下载层加了「无摘要」模式（`Sum::None`，手输地址不带校验值）与文件名推导（`name_from` / `free_name`） |

#- 第三次重发（18:10）：`nuwa-v1.9` 顶到 `3b34ec9`，zip sha256 `eb867c20…`；两槽刷入，真机 `nuwa 1.9`、密钥链 6 条；用 `grep -c "came off the tree at once" /system/bin/recovery` 确认镜像里就是新逻辑。

## 2026-11-14 深夜 · 照 TWRP 的做法：data 打不开时用内存顶内部存储

- **参考对象**：用户给的 `recovery.img`（TWRP，boot v4 / lz4 legacy，ramdisk 157 MB）。解出来看到 TWRP 的两条关键做法：
  1. **密钥栈整个塞进 ramdisk**：`vendor/bin/qseecomd`、`vendor/bin/hw/android.hardware.security.keymint-service-qti`、`gatekeeper@1.0-service-qti`、`vendor/lib64/libqtikeymint.so` 等，全部自带；`init.recovery.qcom.rc` 用 `on property:vendor.sys.listeners.registered=true` 起 gatekeeper 与 keymint，`seclabel u:r:recovery:s0`、`LD_LIBRARY_PATH=/vendor/lib64:/vendor/lib64/hw:/system/lib64:/sbin`。所以 **vendor 分区不在也能解密**。
  2. **密钥由自己造**：`system/bin/recovery` 里链进了 vold 的 `KeyStorage`（`createSecdiscardable` / `readSecdiscardable` / `keymaster_key_blob` / `/metadata/vold/metadata_encryption`），格式化 data 后自己造 key，再提示「You may need to reboot recovery to be able to use /data again.」。fstab：`metadata_encryption=aes-256-xts:wrappedkey_v0`、`keydirectory=/metadata/vold/metadata_encryption`。
  3. 打包的刷机包放 `/tmp`（tmpfs），所以没有 /data 也能 sideload/安装。
- **真正的原因（2026-11-14 深夜查明）**：`main()` 只看程序名，`name == "recovery"` 就直接起整个界面服务，**不看有没有子命令**。设备上入口只有 `/system/bin/recovery`，所以 `recovery keygen|rebuild-data|stage|keys` 全都是「再起一个恢复服务」，第二个服务抢 DRM/输入设备 → 手机直接挂。**与 TEE 无关**，`generateKey` 那条结论作废。修复 `c911f26`：只有不带参数调用才是服务启动（init 就是这样调的）。
- **因此改为**：
  - 新增 `crates/skyward/src/stage.rs`：data 无密钥（`data::key_kept()`）或打开失败时，把内存（可用内存一半，512 MB–6 GB）挂到 `/sdcard`，建 `Download`、`Pictures/Screenshots`；打开 data 前用 `umount_detach` 摘掉；重启即清空。这样无线传输 8080、MTP、文件页、安装页在**没有 data 时全部可用**——等价于 TWRP 的 /tmp，但落在我们已有的路径上。
  - `rebuild::data` 不再造 key：只清空 userdata + 删掉旧密钥目录 + 建 f2fs；密钥交给系统下次开机自己建（与 TWRP 格式化后的终态一致，少了它那步造 key）。失败页按钮改名「格式化 data（清空）」，并注明「系统下次开机时建立密钥」。
  - `recovery stage` / `recovery stage --off`（`--force` 可在 data 已打开时也挂，供查看）开关内存盘；`recovery rebuild-data [--force]` 只做格式化。
  - 内存盘大小：**可用内存的 3/4，且不少于 8 GB**（上限，不是预分配）。
  - 真机实测（`c911f26`）：`recovery keys` 正常打印六行；`stage --force` → `tmpfs 8.0G /sdcard`，内含 `Download`、`Pictures`；`adb push` 进 `/sdcard/Download` 成功；`stage --off` 干净摘除；有数据的 `rebuild-data` 正确拒绝（rc=1）不擦盘；正常开机不挂内存盘（`holds nothing` 计数 0）。
  - 另修：`ui.rs` 里 `detach(cfg.services.clone())` 改为 `detach(programs.to_vec())`（合并上游后遗留，导致安装前 pin 的是配置里的三个程序而不是真正在跑的密钥链）。

## 2026-11-14 晚 · 安装前卸载分区改为强制

- 现象：装包时 `move off the installed system: unmount /vendor: Device or resource busy (os error 16)`，什么都没写。
- 原因：`install::detach` → `services::pin` 只盯**配置里**的程序（清理后只剩 post_boot / touch_report / touchsensor）；密钥链（qseecomd、ssgtzd、keymint、gatekeeper、hwservicemanager）与 wpa_supplicant 都是运行时由程序描述启动的，不在这个列表里，于是 /vendor、/mnt/system、/mnt/system_ext 被判为「不忙」，走了严格 `umount`，内核 EBUSY。
- 改法：`skyward_vab::release_partitions` 对每个挂载点先按原语义卸载，失败即 `umount_detach_over`（惰性卸载，进程保留已映射的页），仍不行就记日志跳过；返回 `Released { forced, devices, kept }` 供调用方写内核日志。`detach` 不再因卸载失败返回错误，`pin` 失败也降级为日志。备份恢复里 boot/EFS 的卸载同样加了惰性兜底。
- 验证：`adb reboot sideload` + `adb sideload` 一个只含 `META-INF/com/google/android/update-binary` 的测试 zip，日志出现 `/vendor was in use, and came off the tree at once`、`/mnt/system …`、`install: Ran { wipe_cache: false }`，安装走完。
- 留下的：dm 设备（vendor_a 等）因程序仍持有 fd 无法移除，日志会写 `its device-mapper device stays`；挂载页手动卸载仍按设计拒绝「正在使用」的分区。

## 2026-11-14 傍晚 · 密钥链容错、清理、署名，重发 v1.9

- **容错**：`services::start` 原本遇到第一个起不来的程序就 `bail!`，`keys::start` 逐步调它，于是缺一个就整条链停住（data 解不开，日志只有一行）。改成「每个都试，记第一个原因，最后统一报告」，`bring_up` 里 `services::start` 与 `keys::start` 失败只记日志、继续往下（否则密钥链仍被跳过）。
- **真机验证方法**（可复用）：把 `devices/nuwa.toml` 里第一个服务路径改成不存在的 `/vendor/bin/not-on-this-phone`，编一版刷进去，看 `ps` 里 `touch_report`/`touchsensor` 是否仍起来、`dmesg` 是否有 `left out of the chain`、`/data` 是否在 10 秒内挂上。验完还原配置再提交（临时提交已压掉）。
- **清理**：本地 13 个旧分支、远端 5 个旧分支全删（远端只剩 `main`）；移除与 nuwa 无关的远端 `aurora-pr`、`guobao`；删掉其它 20 个机型的描述语料（13 828 个文件、27 MB），只留 `tests/described/nuwa`，`phones` 测试改为核对 nuwa 的六个密钥程序与顺序（需要 `ro.boot.product.vendor.sku=kalama` 才会读 `manifest_kalama.xml`）。
- **署名**：README 中英、「关于」页 `AUTHOR`（`© 2026 Kirk Lin · xiuxiu391 (nuwa)`）、许可证页 `LICENSE_ORIGINAL`、发布说明。
- **重发**：删掉旧 `nuwa-v1.9` 发布与标签，在新提交上重打；`nuwa-v1.8` 仍是上一版可回退。

## 2026-11-14 下午 · 合并上游 0.3.0（programs / binder / omapi）

- 以 `git merge v0.3.0` 整块合入，保留本 fork 的全部自有功能。
- 冲突只有 13 个文件，逐个处理：README 留我们；binder 三个文件、`skyward-sys`、`services.rs`、`secure.rs`、`ui.rs` 取上游（ui 另把 `install_asked`/`run`/`read_mounts` 的 `programs` 参数补回我们的版本）；`skyward-config` 取我们（去掉上游对已删机型的测试）；`main.rs` 一处冲突取上游。
- 上游 `Service` 去掉 `description` 字段，我们的 `programs` 发现模块（config 与 skyward 两处）随之删除，统一由 `skyward-programs` 承担。
- `devices/nuwa.toml` 只留 `post_boot`、`touch_report`、`touchsensor` 三项；密钥链（qseecomd、ssgtzd、keymint、hwservicemanager、gatekeeper、boot）由手机自己的 init 描述产生。
- 真机验证（`6017c8d`）：`for the phone's keys: ...` 列出六个程序（比手写多出 `ssgtzd`），五个进程在跑；点「解密」后 `[3586] the data partition is open: f2fs at /data`，`ro.vendor.build.security_patch` 仍报 2099-12-31。
- 未做/待做：`skyward-pack described` 子命令可用但未接 CI。
- 2026-11-14 清理：删掉其它 20 个机型的描述语料（13 828 个文件、27 MB），只留 `tests/described/nuwa`；`phones` 测试改为核对 nuwa 的六个密钥程序与顺序；无关远端（`aurora-pr`、`guobao`）与全部旧分支（本地 13 个、远端 5 个）已移除，远端只剩 `main`。

## 2026-11-14 凌晨 · 移植上游触屏唤醒修复，发 v1.8

- 上游 0.3.0 的 `37ef67e`（fix(ui): wake the touchscreen when the recovery service starts again）：`skyward-sys` 记「这次是不是第二次启动」（`keep_log` 时看日志文件在不在，`RUNS_AGAIN`），`ui::run` 在 `Inputs::open` 之前 `surface.rest()` → `present(黑帧)` → `settle()`。
- 我们这边 `KmsSurface` 的 `rest/present/settle` 与原版同名同义，直接可用；改动落在 `crates/skyward-sys/src/lib.rs` 与 `crates/skyward/src/ui.rs`。
- 真机验证方法：`adb shell kill -9 <pidof recovery>`，init 的 `service recovery`（非 oneshot）会重启；`/skyward/run/recovery.log` 出现第二条 `recovery up` 即证明 `runs_again()` 命中。v1.8 上验过：第二条 `[25.57] nuwa 1.8 recovery up`，`touch_report` 正常，FTS `fts_probe` 正常，无 DRM 报错；用户确认可以正常点。
- 发布：`nuwa-v1.8` → `c93c48f`，zip sha256 `01f24e0c…`；两个槽位都刷了该镜像（`fa0a9735…`）。
- **Windows 侧坑**：`adb devices` 空、但设备管理器有 `Android ADB Interface`，且 adb 服务端日志报 `Failed to open device handle ... Error [5]: 拒绝访问`，说明有另一个**提权 adb 进程**独占 USB 句柄；结束它（管理员 `taskkill /F /PID <pid>`）即恢复。不是手机问题。

## 2026-11-13 晚 · 撤掉锁屏层（内置浏览器后来也整体删除）

- **按用户要求移除**：下拉控制中心、锁屏（日期/大字时间/电量/滑动解锁）、双击亮屏。做法是 `git revert` 回退引入它们的提交（`a937b4a`、`9c21ac6`、`2a18e51`、`37a6213`、`dd1791b`、`7fa9075`），不是逐行删；`sleep()` 回到单参数，`compose()` 不再画锁屏。保留：记住锁屏密码（`remember`）、锁屏页（解密页的密码输入）、截图快捷键。
- ~~新增内置浏览器~~ **已按用户要求整体删除**（2026-11-13 深夜）：`skyward-net/src/html.rs`、`skyward/src/browser.rs`、`skyward-ui/src/picture.rs` 与 `jpeg-decoder` 依赖、高级页入口、`Screen::Browser`/`Screen::PickFolder`、`browse_folder` 设置项一并移除；下载页的二维码与 `link.txt` 仍保留（HTTP 服务端在 `skyward-net/src/portal.rs`，与本项无关）。删除原因：recovery 里没有安卓运行时，无法运行 Chromium/Via；自绘的简易浏览器排版达不到「正常浏览器」，用户决定不要了。
- 上面的截图对齐表里，控制中心与锁屏两行已随回退作废，其余（解密页圆环、WLAN 记忆、备份跳过、下载页二维码）仍保留。

### 2026-11-13 · 对齐 pudding 截图（控制中心、锁屏、解密页、下载页）

| 文件 | 改了什么 |
| --- | --- |
| `crates/skyward/src/ui.rs` | **开机进首页**：删去 `first == Home → Decrypt` 的映射，解密页与其他页面一样从首页进入；data 仍按需打开（进解密页/文件/OTA）。发布说明与两份 README 同步重写 |
| `crates/skyward-ui/src/app.rs`、`crates/skyward/src/ui.rs` | **下拉控制中心**：顶边 40dp 内下拉（`SHADE_PULL_DP`=110dp 算全开，松手按位置+速度走 1.8s smoothstep 收放），面板四块：WLAN（已开/已关，点了直接开关）、截图、重启（开重启页）、熄屏（锁屏待双击）；亮度/振动两条滑条即时生效（亮度走 `state.brightness`，即背光）。截图在面板收起后 130ms 取，避免把面板拍进照片。锁屏加日期（`App::set_date`，ui 每分钟跟时钟一起刷新），时间/电量按截图排在上三分之一 |
| `crates/skyward-ui/src/pages.rs` | 解密页：锁图标外套一圈进度环（`stroke_arc`），副标题按步骤换文案（打开分区 → 导入共享密钥 → 读取用户密钥 → 登记系统服务）；进度条加宽到 200dp |
| `crates/skyward/src/wlanmem.rs`（新）、`skyward-ui`（`settings`/`app`/`pages`）、`crates/skyward/src/ui.rs` | **WLAN 自动连接 / 记住密码**（两开关在 WLAN 页）：自动连接=开机即开 WLAN 并在扫描到后连上次的网络；记住密码=连接成功后把 SSID（十六进制）与密码写 `/metadata/skyward/wlan`（0600，原子 rename）。密码被拒（`WrongPassword`）自动删除；手动断开不会被暗中重连；关掉开关即删除 |
| `crates/skyward-backup/src/archive.rs`、`crates/skyward/src/backup.rs`、`skyward-ui` | **备份跳过读不了的文件并计数**：第二空间（999）等无密钥文件返回 EACCES/ENOKEY 时不再让整份备份失败，跳过并累加；结果页写「跳过 N 个无法读取的文件」；列不出的目录也当作空的继续 |
| `crates/skyward-net/src/portal.rs`（新）、`skyward-ui`、`crates/skyward/src/ui.rs`、`vendor/qrcodegen/` | **网络下载页加二维码**：进入下载页且有 WLAN 地址时在 8081 起一个小网页（`Portal`），另一台手机扫码后输入网址，回传即填进地址栏；二维码由 vendored 的 `qrcodegen` 1.8（MIT，`workspace.exclude` 单列，不参与 lint）绘制。另有「从 /sdcard/Download/link.txt 读链接」一行，取第一行非空内容；页面按截图重排：输入框 → 居中按钮 → 文件入口 → 二维码 → 说明 → 进度 |

新增单元测试 24 个（`github.rs` 3、`image.rs` 1、`release.rs` 1、`fstab.rs` 2、`data.rs` 3、`wipe.rs` 1、`fsck.rs` 1、`serve.rs` 6、`download.rs` 2、`skyward-ui` 4）。**验证进度**：推分支 ✅ → `build.yml` lint + test 绿 ✅ → 合并 `fork/main` ✅ → 第二轮改动待 CI → 打包刷机（待做）→ 真机进文件页确认解密后自动重列（待做）。

---

## 11. 上手检查清单（新接手时按顺序做）

1. `gh run list --repo pubglite55/skyward-for-nuwa --limit 3`：确认最新构建绿
2. `gh release view nuwa-v1.5 --repo pubglite55/skyward-for-nuwa`：确认 Latest 资产（zip 约 43 MB + SHA256SUMS.txt）
3. 确认工作分支与 `fork/main` 一致、`git status` 干净；本地 `main` 是上游镜像，别切错（见 §8）
4. 手机接上：`adb devices` / `fastboot devices`（工具不在 PATH，用 §8、§12 的绝对路径）
5. 要验证解密：进 recovery → **点「文件」页** → `adb shell "mount | grep ' /data '"` 应看到 `dm-8 ... f2fs (rw,...,inlinecrypt)`；
   `dmesg | grep 'skyward:'` 应有 `the data partition is open: f2fs at /data`
6. 任何改动先 `git status`，提交后推到 `fork/main` 并等 `build.yml` 绿；**不发版就不打 tag**

---

## 12. 常用命令速查

```sh
# 前置：仓库在 nuwa recovery/skyward，工具与凭据在 nuwa ios/
GH="C:/Users/L1872/Desktop/nuwa ios/.tools/bin/gh.exe"
PF="C:/Users/L1872/Desktop/nuwa ios/.tools/platform-tools"
TOKEN="C:/Users/L1872/Desktop/nuwa ios/work/t.txt"
REPO="C:/Users/L1872/Desktop/nuwa recovery/skyward"
IMG="out/skyward-nuwa.img"     # 改成实际镜像路径
RUN_ID=43184098214             # 改成实际 run id
ARTIFACT_ID=123456789          # 改成实际 artifact id
cd "$REPO"                     # 相对路径以仓库根为准
export GH_TOKEN="$(cat "$TOKEN")"
export HTTPS_PROXY= HTTP_PROXY= ALL_PROXY=   # api.github.com 直连；curl 的 -x 单独指定代理

# 状态（一律带 --repo，默认 remote 是上游 origin）
"$GH" run list --repo pubglite55/skyward-for-nuwa --limit 3
"$GH" release view nuwa-v1.5 --repo pubglite55/skyward-for-nuwa
"$GH" api "repos/pubglite55/skyward-for-nuwa/actions/runs/$RUN_ID/jobs" \
  --jq '[.jobs[] | .name + ":" + (.conclusion // .status)]'

# 下载 artifact（走代理）
curl -s -L -x http://127.0.0.1:7890 -H "Authorization: Bearer $(cat "$TOKEN")" \
  -H "Accept: application/vnd.github+json" -o a.zip \
  "https://api.github.com/repos/pubglite55/skyward-for-nuwa/actions/artifacts/$ARTIFACT_ID/zip"

# 刷机（adb/fastboot 不在 PATH）
"$PF/adb.exe" reboot bootloader
"$PF/fastboot.exe" flash recovery_a "$IMG" && "$PF/fastboot.exe" flash recovery_b "$IMG"
"$PF/fastboot.exe" reboot   # 再 adb reboot recovery（比 reboot recovery 可靠）

# 取证
"$PF/adb.exe" shell dmesg | grep 'skyward:'
"$PF/adb.exe" shell "mount | grep -E ' /data | /sdcard '"
"$PF/adb.exe" shell "ls /sdcard/DCIM"

# 分支 / remote 核对
git -C "$REPO" remote -v
git -C "$REPO" branch -vv
```

---

*本文档由 2026-11-12 的工作整理：解密根因定位与修复、仓库收敛为单机型、README 重写、自更新指向本仓库、v1.5 三次重建发布。*

*2026-11-12 16:41 修订：仓库迁至 `nuwa recovery/skyward` 后，更新路径、分支与 remote 描述；发版命令的推送目标由 `origin` 更正为 `fork`，`gh` 命令补上 `--repo`。*
