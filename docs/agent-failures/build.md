# 构建、缓存与 Rust 检查

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 1. 共用构建/发布脚本先失败

**典型症状**

- CI 在 `Resolve build inputs` 的 “Validate common build and package logic” 失败，其他平台尚未开始或全部被跳过。
- Publish Release 在 `Resolve verified CI` 的 “Verify CI and freeze source” 失败。
- Python 测试出现 `IndexError`、`ValueError`，但日志只显示最终 `Process completed with exit code 1`。

**代表证据**

- CI [40005445463](https://github.com/Fu021/Clash-of-Rust/actions/runs/40005445463)：`action_cache.py` 对空 `prefix` 取值触发 `IndexError`。
- CI [39771037547](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771037547)：共用构建测试失败。
- Publish Release [39815271615](https://github.com/Fu021/Clash-of-Rust/actions/runs/39815271615)：上游 CI 必需 job 未通过，发布被正确拒绝。

**首先检查**

- `scripts/build_inputs.py`、`scripts/workflow_artifacts.py` 和 `scripts/action_cache.py` 的接口是否与工作流同步，见[脚本职责](../../scripts/README.md#职责清单)。
- 新字段是否允许旧缓存、旧 manifest 和空/缺失输入；失败时应安全回退，而不是越界。
- `.github/workflows/*.yml` 中冻结输入、artifact schema、job 名称和脚本参数是否一致。
- composite action 的步骤字段受独立元数据契约约束，不能直接复制工作流步骤的 `timeout-minutes`；脚本内设置硬超时，调用方保留总预算。缓存允许失败时仍需核对步骤和日志，绿色 job 可能掩盖整个缓存入口加载失败。
- 离线脚本测试须隔离 `RUNNER_TEMP`、依赖快照路径及 Actions 的 env/output/state/summary 文件。检查模拟产物和结果只写入测试目录。
- 引导读取工具版本时不能提前导入 `tomllib` 等待安装解释器才提供的模块；固定 Python 须核对各 OS/架构实际提供的二进制，见[运行依赖缓存](../workflows/cache-coverage.md)，不能直接推广 Linux 的安全维护补丁号。
- 绿色 CI 仍检查实际日志：Slint `viewport-y/height` 迁移为 `content-y/height`；Android 后端明确传入已选择的 SDK/build-tools，不能自动探测 runner 上的预览版；Gradle 显示完整弃用来源，Java 保留细粒度诊断。NativeActivity 的旧系统回退仅在必需方法上标注兼容，不能全局屏蔽。
- 独立 Python 的 `ssl.get_default_verify_paths().cafile` 可能为空或不存在，不能直接据此认定 runner 缺少 CA。Linux 容器引导先查解释器默认路径，再查 runner 的 `/etc/ssl/certs/ca-certificates.crt`；两者都不存在才拒绝冷构建，不关闭 TLS 校验。CI `42768224908` 的两个 Linux 编译任务因此在构建前失败，三个安装验证任务只因缺候选包连带失败；本地回归覆盖默认路径、系统回退和全部缺失。

**防回归**

- 同时修改生产脚本和 `scripts/test-build-tools.py` / `scripts/test-workflow-artifacts.py`。
- 对空输入、缺字段、过期 schema、重复 artifact、下载中断和校验失败写测试。
- 不把“重新运行 Actions”当作根因修复；若只是外部瞬时故障，记录证据并在[最近复盘](reviews.md)标明。

## 2. 缓存不完整或跨版本复用

**典型症状**

- `serde_core` 缺少 `OUT_DIR/private.rs`，或 Cargo 在尚未编译的依赖目录中报找不到构建产物。
- `skia-bindings` 的 Git 拉取出现 `expected 'acknowledgments'`、`expected 'packfile'` 或 503。
- 缓存下载超时、分片不完整，随后正常编译也失败。

**代表证据**

- CI [39946180066](https://github.com/Fu021/Clash-of-Rust/actions/runs/39946180066)：部分恢复的 Cargo 产物缺少 `serde_core` 生成文件。
- Android CI [38929932155](https://github.com/Fu021/Clash-of-Rust/actions/runs/38929932155)、[38925737608](https://github.com/Fu021/Clash-of-Rust/actions/runs/38925737608)：Skia 源获取失败并留下损坏 Git 状态。
- 修复提交 `91df487`、`eb08c8d`、`5b70648`、`adeb083`、`f0b61b1`。

**首先检查**

- 缓存是否按源代码、工具链、`Cargo.lock`、Gradle/Go 锁和构建脚本共同失效。
- 恢复失败后是否丢弃不完整编译对象，同时保留已验证源码快照和测试报告。
- 远端限流、超时和 DNS 故障是否有有界重试、兼容缓存和正常下载回退。
- Linux 构建容器在宿主准备资源后复核摘要。Windows 恢复文件应在目标目录暂存，避免继承仅管理员可访问的私有临时目录 ACL，再由普通用户真实读取验证。
- 基础网络验收不应安装未执行的桌面会话。CI `42797177155` 的 Ubuntu latest 为 GNOME 间接拉取 245 个包，两个 Ubuntu 26.04 `libgphoto2` 原包返回官方 404；改为只准备网络工具，图形依赖保留给扩展验收，旧 VM 模板清单不变。不以跳过代理/TUN、关闭签名校验或盲重跑处理镜像同步问题。

**防回归**

- 缓存归档必须校验内容、路径、符号链接和中断状态，不能只校验文件存在。
- 源码时间戳回执须先确认 JSON 根类型是对象，再读取 schema 和成员；数组、空值等损坏回执应忽略并正常交给 Cargo 校验，不能在可选缓存阶段中止构建。回归同时核对源文件内容、时间戳和现有编译对象未被修改。
- 自动测试必须覆盖“部分恢复仍可正常重编”和“旧格式缓存不污染当前构建”。
- 可选缓存上传要有硬超时，不能拖垮已经通过的主构建。
- main 缓存保护须限定为每个目标的最新代，并把迁移前后命名空间归为同组；仅排除其他 ref 会使旧 main 完整归档永久累积。按创建时间而非读取时间选最新代，区分架构和 profile，未知 main 类型保守处理；分支只读 main，不保存完整对象或差异包，详见[缓存分工](../workflows/cache-coverage.md#编译对象只用-github)。
- Linux workspace 缓存按源码分键，容器与宿主共用经过内容核对的时间戳回执。大归档的压缩、恢复和保存分开计时。Docker 编译环境缓存仅包含构建依赖，恢复时核对归档与加载后的镜像身份。
- 缓存 Action 的“workspace”不等于根目录内所有 path 依赖。`Swatinem/rust-cache` 把 workspace 外部依赖和成员列为保留项，显式排除的本地 Slint 库会被清掉；`c3888a5` 的 macOS Intel 在源码未变化、518 个时间戳已复用时仍重编软件渲染库及上游 UI，构建耗时约 520 秒。通过现有外部回执目录另存限定包的对象、指纹和构建输出，恢复前核对整份内容，源文件变化仍失效。不要仅提高容量、只保存时间戳或为了缓存把 vendor 加入项目 lint／测试范围。
- 调整下载或缓存预算后，用实际保存、下载和校验的阶段耗时判断效果。
- CI `42565924150` 的 Android ARM64 在下载固定 Gradle 9.9.0 时遇到 504。统一使用 `build_inputs.py prepare-gradle` 下载并核对官方 SHA256，编译前验证 PATH 上实际版本。
- GitHub `latest` 资源的附件 ID 可能被上游重建。官方 Geo 附件返回 404 时重新解析并有界重试，继续核对 SHA256，确保同轮所有平台使用同一快照。


**Ubuntu 测试依赖验证**

APT 可能复用本次运行下载目录中同大小的 DEB。每轮用签名软件源索引核对包名、大小和 SHA256 后再安装，缺失或损坏时重新下载。云镜像与 Python 按冻结的上游 SHA256 校验，旧系统用新建磁盘执行安装和网络检查。见 [Ubuntu 环境](../workflows/ci.md#ubuntu-2004-编译与验证环境)。

`apt-get --print-uris` 的本地应用 DEB 记录可能没有摘要；先区分本地输入，再要求远程依赖具备四字段及 SHA256，不能因此使整组依赖缓存失效。回归覆盖本地三字段与远程强校验混合记录，缺少摘要的远程记录仍拒绝。Linux 包按 `debian/<包名>` 暂存，扫描 ELF 前创建 `DEBIAN/`，供 `dpkg-shlibdeps` 识别包根目录；仅调整暂存路径不足以消除警告。独立 D-Bus 测试会话只更新自己的激活环境。旧内核 VM 复用本轮 SSH 控制连接，减少 TCG 下重复握手，不更改实际 GUI/代理/TUN 验证。NSIS 等原始下载按内容摘要去重；当前工作流不再上传安装目录或 Gradle 转换缓存。

2026-11-10 审查的外部边界：JNI 0.22.4 的 `jni-macros/src/native_method.rs` 仍生成 `fetch_update`；`download-artifact` v8.0.2 内部仍输出 `Buffer()` 弃用提示，[上游修复](https://github.com/actions/download-artifact/pull/490)尚未合并。CI `41687501026` 的 Gradle 栈确认 `Configuration.setVisible` 来自 AGP 内置 Kotlin 插件，[上游问题](https://issuetracker.google.com/issues/560282299)也记录过修复后仍可复现。普通构建保留 `--warning-mode all`；`RUNNER_DEBUG=1` 显示详细诊断及弃用栈。Android 编译显式传递冻结 NDK 版本后，CI `42845828066` 三个 ABI 未再出现原生库无法剥离提示；Ubuntu 20.04 x64 的 guest SSH 仍报告非后量子密钥交换。保留来源，不通过修改告警级别伪装为已修复；复查要求见[日志与耗时审查](../workflows.md#运行日志与耗时审查)。

## 3. Rust 编译与 Clippy 跨 target 差异

Rust 工具链升级后须检查各平台展开 `cfg` 后的控制流；Linux 带兼容回退的分支在 Windows 上可能退化为多余布尔判断。修复表达式并保留回退的短路语义，不关闭新 lint。Rust 1.100 的 `cargo::unused_dependencies` 还会报告清单中的无用依赖；先查全部源码、测试与平台入口，删除无引用项，把只用于 Windows 的依赖移到对应 target，而不是全局压低告警。版本及锁文件随清单同步，仍由各平台严格 Clippy 验证。

移除依赖版本、vendor 或 feature 后，锁文件也可能残留不再启用的可选依赖边。仅检查版本、摘要和引用唯一性不能证明 Cargo 接受该图；应由 Cargo 解析并保留同提交的 `--locked` 验证，不能删掉该参数绕过失败。Slint 菜单栈统一后残留的 `bitflags → serde_core` 即属此类，解析修复不需要顺带升级其他依赖。

**典型症状**

- `-D warnings` 下出现死代码、未使用函数、未构造枚举变体、多余 `return` 或可合并 `if`。
- 某个平台通过而另一个平台因 `cfg`、依赖 feature 或测试 target 失败。
- 测试 target 缺少仅在主二进制启用的模块、crate 或 import。

**代表证据**

- CI [39805637475](https://github.com/Fu021/Clash-of-Rust/actions/runs/39805637475)：六个桌面平台均因 `timestamp`、`traffic` 未被使用被 Clippy 拒绝。
- CI [39771203085](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771203085)：枚举变体未构造导致 Clippy 失败。
- CI [39752822767](https://github.com/Fu021/Clash-of-Rust/actions/runs/39752822767)：多余 `return`。
- CI [40454555654](https://github.com/Fu021/Clash-of-Rust/actions/runs/40454555654)：Geo 事务恢复遗漏 `TempDir::keep()` 的返回值，所有平台严格 lint 失败。
- CI [39017685938](https://github.com/Fu021/Clash-of-Rust/actions/runs/39017685938)：`ManuallyDrop` 联合字段与 `chunks_exact` 的严格 lint。
- CI [39004271043](https://github.com/Fu021/Clash-of-Rust/actions/runs/39004271043)、[39002539543](https://github.com/Fu021/Clash-of-Rust/actions/runs/39002539543)：缺少 `log`/`url` 导入、Android 类型或稳定通道不支持的 API。

**首先检查**

- 条件编译后的每个 target、lib、bin、test 和 example 是否都实际使用新增代码。
- 公开常量、错误枚举、序列化分支和平台 helper 是否只在测试中被引用。
- 是否使用了当前 Rust stable 不支持的 API 或只在某个 target 存在的类型。

**防回归**

- 工具链充足时执行与 CI 一致的 workspace Release Clippy 和测试；资源受限时明确标注未执行。
- Rust 格式验收使用与 CI 相同的 `cargo fmt --all -- --check`；它只做静态格式检查，可在资源受限环境运行。单独入口的 rustfmt 结果不能代替 workspace 检查。
- 测试与生产代码共享 helper 时检查 `cfg(test)` 和平台边界，避免测试专用分支进入生产编译。
- Rust 导入按 `lib.rs` 的实际模块声明核对，不能把源码目录直接当模块路径；`#[path]` 映射到根模块的文件同样适用。[CI 40557511991](https://github.com/Fu021/Clash-of-Rust/actions/runs/40557511991) 的 DNS 测试曾误用 `crate::detection::claude`，实际入口为 `crate::claude_check`，导致桌面测试目标全部编译失败。
- 检查返回值的 `must_use`：保留临时目录时显式处理返回路径，不能假定方法返回空值。
- 网络测试夹具同样处理 `read()` 的返回长度与 EOF；只发送预定错误响应时也要跳过空连接，不能忽略读取量或用 lint 豁免隐藏问题。订阅更新回归的 [CI 40175249665](https://github.com/Fu021/Clash-of-Rust/actions/runs/40175249665) 曾因此在桌面 Clippy 阶段失败。
- 不以放宽 lint 掩盖未使用代码；无用代码应删除，确需保留时说明原因并限制作用域。

### 依赖主版本迁移

- 逐一检查所有 workspace、target、构建和测试声明，避免只改一处而保留同一直接依赖的旧版本。锁文件含上游可选包不代表生产功能启用了它，也不能据此宣称依赖已从整个图中消失。
- `reqwest` 0.13 的 TLS feature、查询参数和证书来源与旧版不同；Android 需继续用共享构建器的内置根证书。JNI 0.22 要迁移 `EnvUnowned`/`Env`、带类型全局引用、线程回调与 JNI 字符串/签名，不能仅改版本号。
- SHA-2 0.11 的摘要数组不再实现 `LowerHex`，原 `format!("{:x}", digest)` 会令所有平台失败，见 [CI 40107022637](https://github.com/Fu021/Clash-of-Rust/actions/runs/40107022637)。使用[共享十六进制编码](../../crates/client-core/src/storage/hash.rs)，同步 Geo、规则修订、下载校验与特权安装器，保留前导零和已知 SHA256 向量的回归。
- 目标专用声明和测试也须迁移：`winreg` 0.56 的 `RegValue.bytes` 改为 `Cow<[u8]>`，旧测试夹具的 `Vec` 需显式转换；移动保护校验迁出 Android 后清理残余导入。后续 [CI 40108770502](https://github.com/Fu021/Clash-of-Rust/actions/runs/40108770502) 分别暴露了 Windows 类型错误和 Android 未使用导入，不能只凭共享 crate 编译通过判定升级完成。
- 替换 YAML 与二进制协议库时保留行为回归；协议编码改变须递增协议号。AGP 与 Gradle 成对升级，并对最低版本、预发布、损坏版本及未来不兼容主版本保留解析边界检查。当前版本来源、审计入口和上游约束见[依赖维护](../development.md#依赖维护)；[旧核对表](../dependency-audit.csv)只保留当时的升级证据。
