# Debian 兼容验证

## 需求与顺序

Linux 系列第一项，复用现有 DEB，候选为 Debian 12/13 x64。维护者已授权自行拆分并逐项推进；当前先完成已有 doing 任务，不与其他 Linux 子项目同时实施。

**2026-11-17 最新排期：保持 pending。** 本次 doing、TUI/网络诊断打磨、正式 loop、正式发布及网站收尾完成前，不继续本项调查或实施，等待 CI 时也不提前推进；保留下方既有元数据证据，之后重新核对来源。

共享合同、既有来源和全系列排期见 [Linux 扩展索引](../pending/085-Linux发行版与安装格式扩展.md)。此候选范围尚未代表已支持，Debian ARM64 不在首批。

## 验收

- [ ] 接手时核对官方版本、维护期限、镜像及校验来源；确认 Debian 12 LTS 的实际包与桌面支持范围。
- [ ] 检查 Ubuntu 20.04 构建产物的真实 ELF 符号、动态依赖与 DEB 元数据，记录 Debian 上的依赖名、polkit、用户服务、桌面集成差异。
- [ ] 在变更工作流前给出具体矩阵、镜像、runner 成本和回执设计，取得所需 `.github/` 审核；复用现有 DEB 和验证入口，保持既有 Ubuntu 两架构门禁。
- [ ] 分别在 12/13 真实 VM 完成索引规定的完整安装、运行、更新和恢复验收，记录同提交来源与覆盖集；不以 Ubuntu 成功替代。
- [ ] 循环复核 GUI/TUI/CLI、权限、持久化及纯 CLI 边界，取得所需同提交验证后才更新公开支持范围，并沉淀供后续 DEB 项复用的入口。

## 进度与交接

2026-11-17 由大任务拆出，尚未实施、创建工作流或下载镜像。第一步读取索引中的既有调查，再对照 DEB 打包、Ubuntu Compatibility 和 Linux 安装/更新调用链，准备可供审核的最小改动。

等待 TUI 最终 CI 时只读追踪了现有入口：`.github/workflows/ubuntu-compatibility.yml` 的来源解析由 `scripts/compatibility.py resolve --suite ubuntu` 生成两个固定 Ubuntu job；`required_jobs`、`input_names`、`resolve` 和回执校验共同绑定现有 job 名称及两架构候选包。`scripts/linux_ci.py` 又固定 `20.04`、`ubuntu20-v1` 与 focal 官方镜像。这些都不是可直接换一个 matrix 版本号的通用发行版入口。

首项审核方案应同时说明：保留 Ubuntu 20.04 两架构默认门禁，增加显式可选 Debian 12/13 x64 覆盖；从同一 CI 消费现有 DEB 和网络夹具，不重复构建；将新的镜像身份、目标覆盖集、job 名称与回执一起设计，避免只加 VM 却被旧严格校验拒绝，也不能让扩展模式漏测原门禁。现有服务端 Ubuntu 预装镜像键不挪给 Debian，首轮先使用经核实的官方镜像，新增缓存服务合同另行协调。尚未给出未经测量的耗时承诺、修改 `.github/` 或声明 Debian 真实验收通过。

### 现成产物的 ELF 预查

2026-11-17 等待 TUI 兼容验证期间，仅对已下载 CI `44031421992`、提交 `37e5a4f51443e8336aa145923c3203dab295afe0` 的 x64 产物做静态分析；归档摘要已校验，四个 ELF 又分别核对 `manifest.linux_elf` 中的 SHA256，未执行它们或安装软件包。恢复时从该 CI 的 `package-linux-x64` 附件重新提取并用 `readelf` 核对，不依赖本机临时文件。

| ELF | 最大 GLIBC 版本需求 | 主要直接动态依赖 |
| --- | --- | --- |
| `clash-desktop-ui` | 2.30 | fontconfig、libgcc、glibc |
| `clash-of-rust` | 2.30 | GIO、GLib、GObject、libgcc、glibc |
| `cor` | 2.30 | GIO、GLib、GObject、libgcc、glibc |
| `clash-tun-launcher` | 2.30 | libgcc、glibc |

四者没有 GLIBCXX/CXXABI 版本引用，GCC 符号需求最高 4.2.0；未发现本轮 x64 二进制抬高既有 glibc 2.31 构建边界。`DT_NEEDED` 不能覆盖按需加载的 X11/Wayland/EGL 等插件，也不能证明 Debian 图形、权限与网络兼容。当前仍未检查实际 DEB 控制字段、ARM64、Debian VM 或正式支持范围。

打包源码会额外声明 `libxkbcommon0`、`libxkbcommon-x11-0`、`libx11-6` 等依赖，并以 `pkexec | policykit-1` 推荐授权组件；接手时必须用实际 DEB 与 Debian 版本的包元数据核对，不能仅根据上述直接依赖表删减。Debian 官方云镜像目录本轮访问受限，镜像的日期 URL/校验清单尚未冻结，保留为未核实项。

### 实际 DEB 元数据与布局预查

同一已验收 CI `44031421992` 的 `package-linux-x64` 现成产物已下载，未安装或运行。包 SHA256 `447895e9097c27e48b468d1b4b2ab066f65ae88bad5b91e54b8f4f3bf2f7fcf7` 与 sidecar、package receipt 三者一致；回执 commit 为 `37e5a4f...`、CI id 正确、原生 tests_passed 为 true。

`dpkg-deb --field/--contents` 的静态结果保存在 `control-fields.txt` 和 `package-contents.txt`：包名 `clash-of-rust`、架构 amd64、Debian 版本 `0.7.2~fix.5.tui-update-feedback`；直接依赖下限为 libc6 2.30、libfontconfig1 2.12.6、libgcc-s1 4.2、libglib2.0-0 2.39.4，另有完整 X11/Wayland、字体和系统工具依赖；推荐项为 `pkexec | policykit-1`。Debian 13 的 t64 包名与带版本旧名依赖能否经 Provides 正确满足，仍要在该发行版的 apt/真实 VM 中核实，不能只凭虚拟包页面认定安装成功。

归档保留受信 `/opt/clash-of-rust`：`usr/bin/cor` 链接到 `/opt/clash-of-rust/cor`；内核启动器为 root/root 0755、polkit action 为 root/root 0644，桌面入口为 root/root 0644。未发现这份 DEB 自行迁移到 `/usr/lib` 的 AUR 路径问题。以上只补实际包静态证据，不替代 Debian 安装、GUI 授权、用户服务、真实代理/TUN、更新、失败恢复或卸载；该任务仍在 pending，未修改工作流或扩大支持声明。

### Debian 官方包索引预查

2026-11-17 等待 macOS 批 CI 期间，通过 `deb.debian.org` 读取 bookworm/trixie 的 `InRelease` 及 amd64 `Packages.gz`（by-hash URL），逐一核对压缩索引的 SHA256 和大小；只下载元数据，没有下载或安装发行版依赖。下面保留当次摘要与静态结论，恢复时重新获取官方索引；未验证 InRelease 的 OpenPGP 签名，不能记为完整 apt 信任链验收。

| 索引 | SHA256 | 直接元数据结果 |
| --- | --- | --- |
| bookworm amd64 | `6777ea16514725f9427f48c240d92e0a6b8496138e37a1491d8597e66774e309` | DEB 的 23 组 Depends 和一组 Recommends 均存在满足版本约束的原包或 provider；GLib 仍为 `libglib2.0-0`。 |
| trixie amd64 | `42c23aaca08e8225ed0449360724b92531952cbe4ce3bbf189298a7108f40a85` | 同样没有直接声明未匹配项；`libglib2.0-0t64 2.84.4-3~deb13u5` 的带版本 Provides 满足旧名 `libglib2.0-0 (>= 2.39.4)`；授权推荐由 `pkexec 126-2` 满足。 |

检查使用实际 DEB 控制字段，版本关系交给本地 `dpkg --compare-versions`；没有执行 apt 的完整依赖求解、处理所有传递依赖与安装冲突，也未证明 GUI、polkit 会话或真实网络能工作。索引日期与版本可能更新，实施时重新冻结。本项保持 pending，下一步准备可供审核的 Ubuntu 工作流扩展与覆盖回执设计，再取得 `.github/` 变更审核；不为静态匹配结果提前宣称 Debian 支持。
