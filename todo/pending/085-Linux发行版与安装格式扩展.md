# Linux 扩展项目索引与兼容合同

## 需求与授权范围

维护者于 2026-11-16 提供兼容方案，要求采用“最低兼容版本 + 当前主流版本”，评估后纳入待办；2026-11-17 进一步授权 agent 自行拆分项目，慢慢推进、一次一个。发行版分组和实施顺序由 agent 决定，不再逐组索取排期确认；新增 Actions、`.github/` 变更、公开附件扩展、签名、AUR 凭据或自动发布仍按仓库约束单独审核，不由拆分授权推出。

本文件保留跨项目合同、来源和历史调查，作为索引，不作为可与子项目并行接手的第八个实施任务。维护者于 **2026-11-17** 调整优先级：**全部 Linux 后续支持留在 pending 排队，本次 doing 全部完成、TUI/网络诊断打磨、正式 loop、正式版发布及发布后网站收尾之前，不再继续调查或实施 Linux 扩展**，等待 CI 也不例外。0.7.2 正式版、网站和 Slint 升级已完成，当前先处理 [代理组](../doing/000-代理组完整列表与首次展开.md)与 [TUI](../doing/001-TUI设置规则与按键体验.md)。2026-11-18 最新要求为下一次提交触发 CI 后停下等待指令；本系列与 089 不自动恢复，090/100 仍是下下个正式版的远期目标。

## 项目顺序与接手规则

| 顺序 | 独立项目 | 首批候选 | 前置关系 |
| --- | --- | --- | --- |
| 1 | [Debian 兼容验证](085-1-Debian兼容验证.md) | Debian 12/13 x64 | 复用 DEB，先核实共同二进制基线和现有安装合同 |
| 2 | [Linux Mint 兼容验证](085-2-LinuxMint兼容验证.md) | 21.3/22.3，Cinnamon x64 | 复用第一项沉淀的 DEB 验证入口 |
| 3 | [Pop!_OS 兼容验证](085-3-PopOS兼容验证.md) | 22.04 GNOME、24.04 COSMIC，x64 | 复用 DEB；与已有 COSMIC 诊断明确分工 |
| 4 | [Zorin OS 兼容验证](085-4-ZorinOS兼容验证.md) | 17.3/18.1，x64 | 复用 DEB 验证入口 |
| 5 | [Fedora 与 RPM 安装更新](085-5-Fedora与RPM安装更新.md) | Fedora 44/45，x64 | 首次扩展 RPM 打包、更新及发布合同 |
| 6 | [openSUSE RPM 兼容验证](085-6-openSUSE兼容验证.md) | Leap 16.0/16.1 传统可写安装、Tumbleweed，x64 | 依赖第五项已验证的 RPM 基础 |
| 7 | [Arch 配方与包管理器集成](085-7-Arch配方与包管理器集成.md) | 官方 Arch Rolling x86_64 | 单独验证 pacman 所有权、受信布局及内核身份；AUR 自动写入不在首批 |

一次只将一个 Linux 子项目移入 `doing/`；前一项完成或确实因外部条件转入 `blocked/` 后再接下一项。本索引始终留在 `pending/`，子项移动时同步调整上表对应链接；各子项用 `../pending/` 引用本索引，保证移动后仍可读取共同合同。各项完成后按本地待办规则删除子文件，并在本索引去掉对应链接，避免悬空引用；整个系列验收完成后才删除本索引。下方日期、版本与范围均是调查依据，实际接手时重新核对官网，不能把候选表当作公开支持承诺。

每个子项目独立验收：在可丢弃真实 VM 使用同提交候选包，验证原生安装、升级、卸载、普通用户 GUI/TUI/CLI、共享后台、系统代理/TUN、提权拒绝、DNS 与失败恢复、设置持久化，并保存镜像身份、包摘要、来源 CI 与回执。容器、邻近发行版或旧提交结果均不能替代。各端差异按正常平台差异、实现限制与待补缺口分类；纯 CLI 不因新发行版支持增加常驻界面缓存。

分享页未取得正文，以下以维护者直接贴出的内容为来源；候选版本及生命周期不是已经核实的支持承诺。实施前按发行版官网确认实际版本、支持期限、镜像和校验来源，纠正过时或不准确的日期。

## 候选范围

| 安装体系 | 发行版 | 待核实的最低版本／主流版本 |
| --- | --- | --- |
| DEB | Ubuntu | 保留既有 20.04 x64／ARM64 真实 5.4 内核门禁及现有 latest 验证 |
| DEB | Debian | 先核对现有构建／安装覆盖，明确具体版本，不把 Ubuntu 结果当作 Debian 桌面验证 |
| DEB | Linux Mint | 21.3、22.3 |
| DEB | Pop!_OS | 22.04 LTS、24.04 LTS；同时确认桌面环境差异和已有 COSMIC 诊断职责 |
| DEB | Zorin OS | 17.3、18.1 |
| RPM | Fedora | 44、45；不将 43 作为新增正式目标 |
| RPM | openSUSE | 最低 Leap 16.0、当前 16.1、Tumbleweed；不新增 Leap 15.6 |
| Arch | Arch Linux | 官方 Rolling，考虑 AUR `-bin` 包 |

首批不投入 NixOS、Alpine、Flatpak、AppImage。Mint／Pop!_OS／Zorin 的首批桌面兼容验证以官网实际提供的架构为准，不凭 Ubuntu ARM64 覆盖声明衍生发行版也受支持。

## 建议的分阶段方案

1. **固定二进制基线与安装契约。** 继续使用 Ubuntu 20.04 用户态作为 GNU/Linux 构建基线，核验实际 ELF 的最高 GLIBC／GLIBCXX 符号与动态依赖；`glibc 2.31` 只是候选下限，不能仅因在该容器构建就宣布满足。保留现有共享后台、普通用户 GUI、特权助手、代理/TUN 授权及退出恢复。先确定各发行版的服务、桌面集成和依赖差异。
2. **DEB 扩展先行。** 在现有 DEB 与 Ubuntu Compatibility 上扩展经审核的可选桌面矩阵，保留现有 Ubuntu 20.04 两架构的 job、回执与发布门禁契约；默认基础验证与较重的衍生桌面验证区分。Mint／Pop!_OS／Zorin 使用可丢弃真实 VM，容器只能证明用户态装包，不能代替桌面、授权、内核或 TUN 兼容。新增正式支持前取得同提交真实验收。
3. **RPM 候选包与独立验收。** CI 复用同一次冻结的 Linux 二进制与依赖来源组装 RPM，不为各发行版重复编译。评估 Fedora／SUSE 的依赖名称、包脚本和升级语义是否能共用规范；必要差异留在安装适配层。拟新增一个 RPM Compatibility，覆盖 Fedora、Leap 与 Rolling；先测 x64，再根据官方镜像、维护成本和实际需求评估 ARM64。安装、升级、卸载、普通用户启动、提权拒绝、后台所有权、系统代理/TUN、DNS 和恢复均需证据。
4. **Arch 配方与独立验收。** 拟新增一个 Arch Compatibility，校验 PKGBUILD／`.SRCINFO`、固定发行来源与 SHA256、官方 x86_64 及干净环境中的安装升级卸载。Arch Linux ARM 是独立项目，首批不从现有 ARM64 二进制推断支持。倾向先提供引用 GitHub 已公开 Linux 附件、对应 SHA256 与来源收据的 `-bin` 配方，不在 AUR 重建或另维护第三方依赖快照；现有桌面包不能被称为已经 GPG 签名，新增 Linux 发行签名另行授权。先查包名与现有维护者，不能覆盖他人 AUR 包；发布自动更新另行授权。
5. **发行附件与 AUR。** 修改既有 Publish Release，复用同提交候选包与兼容回执；新增 RPM 后须同步当前“九包／18 附件”的校验契约、下载选择、SHA256、网站双语数据、更新通道和安装器格式识别，不只上传文件。AUR 更新可放在同一工作流的隔离 job，须在 GitHub 实际发行成功后执行；AUR 失败不能误报主发行失败后重复创建 Release，也不得在 `check_only` 使用写凭据或发布。

建议仅新增 RPM Compatibility、Arch Compatibility 两个工作流，修改 CI、Ubuntu Compatibility、Publish Release；这是候选设计，待整体成本与门禁契约评估后提交审核，不在当前发布中临时增设流程。

## 验收与决策

- [ ] 官网核实候选版本、生命周期、官方镜像与摘要；给出最低版／主流版／滚动版的明确支持层级。
- [ ] 对照当前构建、安装、自动更新、服务、特权授权、卸载、网站和发布脚本绘出调用关系，避免重复入口。
- [ ] 提交矩阵规模、runner／镜像成本、排队影响及新增两个工作流的审核方案；说明哪些属于每次发布门禁、哪些按相关变更或手动执行。
- [ ] 单独确认支持期限届满后的淘汰规则；滚动镜像记录版本、日期和摘要，不用“Rolling”掩盖不可复现输入。
- [ ] 在维护者确认实施范围后逐阶段实现，所有公开支持声明均绑定同提交验收；不以容器或邻近发行版替代未验证平台。

## 进度与交接

已根据维护者贴出的完整内容记录候选版本与五阶段方案，尚未修改工作流、打包器或发布格式。官网核实、工作流审核和实施范围确认留待接手本项；当前 086 的 Android 与 Ubuntu 兼容验证均已成功，旧 Android 零 job 排队的首次原因仍未定位，不能写成由某个修补根治。

2026-11-16 等待 TUI 同提交 CI 期间做首批官方来源核对，未下载镜像、未修改工作流或扩大公开支持：

- [Linux Mint 版本表](https://www.linuxmint.com/download_all.php)确认 21.3 支持至 2027 年 4 月、22.3 支持至 2029 年 4 月；首批以 Cinnamon 为代表，不把 MATE／Xfce 当作已验。
- [Zorin 技术详情](https://zorin.com/os/details/)确认当前 18.1，17.3 支持至 **2027-06-01**、18.1 支持至 2029-06-01；两者官方处理器范围为 x86_64，不能照搬 Ubuntu ARM64。
- [System76 下载页](https://system76.com/download-pop/)同时提供 Pop!_OS 24.04 与旧 22.04 及校验值，24.04 有 ARM 专用包；[升级说明](https://system76.com/support/upgrade-pop/)区分 22.04 GNOME 与 24.04 COSMIC。本轮尚未找到明确给出旧版精确终止日期的官方声明，不能仅由 Ubuntu LTS 推出 System76 承诺。
- [Fedora 官方发行公告](https://fedoramagazine.org/announcing-fedora-linux-45/)确认 45 已于 **2026-11-03** 发布，原 44／45 候选保留；生命周期精确日期仍待按官方 release engineering 核对。
- [openSUSE 官方公告](https://news.opensuse.org/2026/11/03/leap-unites-traditional-and-immutable/)确认 **16.1 已于 2026-11-03 发布**，因此修正为最低 16.0＋当前 16.1；先验传统可写安装，Immutable Mode 另列支持边界，不能以 RPM 可安装推断系统服务、特权助手和自动更新可写。[15 系结束公告](https://news.opensuse.org/2026/04/01/leap-15-eol/)确认不新增 15.6；Lifetime 页面返回 403，精确维护终点尚未核验。
- [Arch 官方 FAQ](https://wiki.archlinux.org/index.php?title=Frequently_asked_questions)说明官方仅支持 x86_64，已收紧首批矩阵；AUR 名称占用与维护者、PKGBUILD 依赖名、发行包来源仍待调查。

这些仅是待办方案的来源修正；TUI 验收仍优先。新增两工作流、现有 `.github/` 变更、RPM 附件与 AUR 自动发布均待维护者确认具体方案，未据此执行。

### 待审核的执行规模与发布契约

建议分批开放，不一次声明全平台支持。下表都是候选设计，未实施：

| 流程 | 候选工作量 | 是否重复编译 | 默认触发与回执 |
| --- | --- | --- | --- |
| CI | 现有 DEB 加 RPM spec／PKGBUILD 元数据和包内容检查；先只增加 RPM x64 候选包 | 复用同提交 Linux 二进制，不为发行版重复 Cargo | 继续普通 push；新候选包进入统一 artifact／digest／收据契约 |
| Ubuntu Compatibility | 原 20.04 x64／ARM64 两项保持；新增可选衍生桌面组，Mint／Pop!_OS／Zorin 各最低＋当前，共六个 x64 VM | 否，下载同一 CI DEB | 保持手动，以 CI run 为来源；衍生组最多并行两项，明确覆盖集后记录回执 |
| RPM Compatibility | Fedora 44／45、Leap 16.0／16.1、Tumbleweed，共五个 x64 VM；首批只测传统可写安装 | 否，复用同一 CI RPM | 拟新增手动流程，最多并行两项；记录镜像日期／摘要、包身份、安装升级卸载及网络恢复 |
| Arch Compatibility | 官方 Arch Rolling 一个 x64 VM；PKGBUILD 和 `.SRCINFO` 配套 | 否，消费公开附件或本轮冻结候选源；测试模式不得指向未公开的标签 | 拟新增手动流程；AUR 写入不在兼容测试执行 |
| Publish Release | 首批只增加一个 RPM x64 时，候选包由九包／18 附件变为十包／20 附件；ARM64 RPM 另议 | 不重建桌面，仅复用验收产物 | `check_only` 不改发行／AUR；公开模式在包与兼容回执一致后才上传 |

六个 DEB 衍生、五个 RPM、一个 Arch 合计 **12 个新增桌面目标**，不是 12 次 Rust 编译。新增 VM 的镜像容量、启动／安装／网络耗时需用现有环境先测，未实测不承诺固定分钟数；每组最多并行两项减少 runner 和镜像服务争抢，代价是发布等待增长。Debian 的版本与两种架构尚未确定，不偷偷计入这 12 项或声称已有 Ubuntu 证据覆盖。

发布门禁建议：每批取得同提交真实 VM 证据后才开放该批支持；正式宣布的最低／当前目标均进入相应回执覆盖集，发布时不以某一桌面或容器代替其他目标。若维护者更重视发布时长，可先只开放完成验收的部分发行版，其余保持实验候选；不能已宣布支持又静默抽样跳过。现有 Android 与 Ubuntu 两套必需回执不降低。新增回执须绑定同一 CI 输入 id／name／digest、镜像身份、完整目标集及最终成功状态。

AUR 配方的测试来源与公开来源分开：兼容流程使用冻结候选包验证安装契约，发行成功后才把真实公开 URL 和 SHA256 写入正式配方；不能让未公开标签成为必定 404 的安装来源。AUR 自动更新失败只报告配方待补发，不重复上传主发行；凭据需求、包名维护权及是否启用发布 job 均需维护者决定。

### 2026-11-17 补充调查

只读核对当前源码后，RPM/Arch 需要同时扩展更新契约，不能仅新增打包 job：

| 现有入口 | 当前行为 | 新格式实施时必须同步 |
| --- | --- | --- |
| `scripts/build-installer.py` | GNU/Linux 只组装 DEB；使用 `dpkg-shlibdeps` 计算 ABI 依赖，并推荐 `pkexec`/`policykit-1` | 各格式从同一冻结 payload 生成；RPM 的依赖名称、文件权限、capabilities、polkit 与卸载脚本另按实际发行版验证 |
| `crates/client-runtime/src/update/mod.rs::package_name` | 按目标架构固定选择 `linux-amd64.deb`/`linux-arm64.deb` | 使用已确认的安装来源/格式选择附件；不只按 `ID_LIKE` 猜测，不在未知系统下载 DEB 后再失败 |
| `crates/client-runtime/src/update/install.rs` 与 `install/terminal.rs` | GUI 走 `pkexec`/`dpkg`，终端走既有 sudo、安装失败恢复及用户服务交接 | RPM 和 pacman 要有对应的正式安装入口、包管理器冲突/拒绝、旧服务恢复和普通用户启动；不能令 AUR 用户同时被应用内更新覆盖 |
| `scripts/test-release-package.py`、`workflow_artifacts.py`、`compatibility.py`、`publish-release.py` | 九包、18 附件及同 CI 输入/安装/兼容回执绑定 | 新格式扩展命名、digest、格式/版本身份、安装收据、附件唯一性及发布门禁，网站和 GUI/TUI/CLI 下载状态一起同步 |

候选政策：AUR 首批只跟随稳定通道，预发布继续提供经授权的下载附件，不自动覆盖稳定 AUR 配方；若要独立预发布 AUR 包再确认名称与维护权。RPM/Arch 版本映射需用各自原生比较器验证“旧预发布 < 新预发布 < 正式版”，不能机械删除 SemVer 的连字符后沿用错误顺序。AUR 安装来源应明确推荐包管理器升级；应用内更新不能绕过 pacman 的所有权。

额外官方来源与尚未核实的边界：

- [Debian release 表](https://www.debian.org/releases/)和 [Bookworm 的维护交接公告](https://www.debian.org/News/2026/20260712)确认 Debian 12 已转 LTS、至 **2028-06-30**；13 为当前 stable。可把 **12/13 x64** 作为另一个最低/当前候选组；它们尚未列入原 12 项，更未验收。若维护者同意同时实施，总新增桌面目标变为 **14**；Debian ARM64 另议。Debian 11 的常规 LTS 已于 2026-08-31 结束，不依据 GLIBC 2.31 可运行就宣布官方支持。
- [openSUSE 的维护公告](https://news.opensuse.org/2025/09/03/leap-16-doubles-support/)给 Leap 16 每个版本 **24 个月**支持；原 Lifetime 页本次仍 403，尚未给出经核实的精确日历终点。[16.1 下载页](https://get.opensuse.org/leap/16.1/)同时有传统、Immutable 与 ARM64 镜像及摘要/签名链接，首批只用传统 x64；页面部分介绍仍遗留“16.0”字样，镜像身份需按实际签名文件确认。
- Fedora 官方 docs/wiki 本次受 Anubis/访问限制，未取得 44/45 的精确 EOL；不绕过访问保护，也不按固定“13 个月”猜具体日期。[Fedora 项目 2026-11-16 周报](https://planet.apps.ocp.fedoraproject.org/)说明 43 将于 **2026-12-02** 结束维护，仍保留不新增 43 的候选决策。45 的 2026-11-03 发布公告已在前轮核实。
- [Arch 官方下载校验说明](https://archlinux.org/download/)提供 BLAKE2b 与 PGP/WKD 校验。实施时先核对官方签名身份，再把实际 ISO、日期、源摘要与内部 SHA256 写入本轮镜像来源收据，不把自算哈希当作独立签名。当前未下载 ISO、未申请密钥或新增私网访问。

本段原列三个待选择事项；维护者于 2026-11-17 授权自行分项目渐进推进后，现按上方顺序选 Debian 12/13 x64 为首项，不再为发行版顺序重复询问。Arch 首批先做官方配方模板与包管理器集成验证，稳定 `-bin` 包的 AUR 协作及自动写入留在第七项另行决策。新增工作流与发行格式仍需要维护者明确批准，现有 CI/两套兼容门禁保持原契约。

### 已存在的 AUR 包与安装契约差异

2026-11-17 通过 AUR 官方 RPC 匿名只读查询，发现 **`clash-of-rust-bin` 已由第三方维护，当前 0.7.1-1**，上游 URL 指向本仓库；不是可直接注册的新名称。来源：[包页面](https://aur.archlinux.org/packages/clash-of-rust-bin)、[RPC](https://aur.archlinux.org/rpc/v5/info?arg%5B%5D=clash-of-rust-bin)、[当前 PKGBUILD](https://aur.archlinux.org/cgit/aur.git/plain/PKGBUILD?h=clash-of-rust-bin)。网页工具未取得正文，普通 HTTPS RPC 与 cgit 原文读取成功，未执行 PKGBUILD 或下载二进制，也未发送维护者消息。

公开来源如上；恢复任务时重新读取 RPC 与 PKGBUILD，不依赖本机临时文件。不在任务里复制维护者邮箱或整份第三方配方。

- 现配方下载公开 DEB，声明 `aarch64`/`x86_64`，把 `/opt/clash-of-rust` 内容搬到 `/usr/lib/clash-of-rust`，并用软链接把内置 `resources/mihomo` 换成 `/usr/bin/mihomo`。这些是配方声明，不能当作两架构已验收，更不能据此把 Arch Linux ARM 算入官方 Arch。
- 已对照当前 main 与 `v0.7.1` 的 `platform/linux/tun/trusted.rs`：启动器、GUI 授权入口、内核与 DNS 助手均使用固定 `/opt/clash-of-rust` 路径，原生读取拒绝跟随符号链接；polkit 文件的 exec.path 也固定指向 `/opt/clash-of-rust/clash-of-rust`。现配方的移位和外部内核与此合同不一致；按源码推断普通用户 TUN 授权会被拒绝，尚未在 Arch VM 安装复现，不能写成已完成运行诊断。
- 此外，现配方用独立系统 `mihomo`，不再复用本项目本轮冻结的内核；自动更新若仍交给客户端的 DEB/dpkg 路径，也与 pacman 所有权冲突。不能通过放宽受信路径、允许任意内核或 root GUI 来迁就打包差异。

建议调整首批 AUR 方案：**先提供与现有安装合同一致的官方配方模板和 Arch x86_64 验收，优先保留 `/opt` 布局与内置固定内核；由维护者决定是否与现 AUR 维护者协作。自动写 AUR 暂不进入首批**，取得共同维护权限、包名与发布政策授权后再加 Publish Release 的隔离 job。若坚持 `/usr/lib` 与系统内核，需要另审完整受信路径/内核身份设计，并重新验证 GUI/TUI/CLI、polkit、DNS、升级卸载，不能只改一个安装路径。不要覆盖现包、擅自申请移交或发送外部消息。
