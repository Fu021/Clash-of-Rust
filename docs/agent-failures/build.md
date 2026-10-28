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

- `scripts/build_inputs.py`、`scripts/workflow_artifacts.py`、`scripts/cache_shards.py`、`scripts/action_cache.py` 与 `.github/actions/local-cache/` 的接口和清单格式是否同步。
- 新字段是否允许旧缓存、旧 manifest 和空/缺失输入；失败时应安全回退，而不是越界。
- `.github/workflows/*.yml` 中冻结输入、artifact schema、job 名称和脚本参数是否一致。

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

**防回归**

- 缓存归档必须校验内容、路径、符号链接和中断状态，不能只校验文件存在。
- 自动测试必须覆盖“部分恢复仍可正常重编”和“旧格式缓存不污染当前构建”。
- 可选缓存上传要有硬超时，不能拖垮已经通过的主构建。

### 本地缓存清理与沙箱只读视图

- 沙箱内的只读挂载不能直接证明宿主机文件系统故障；用户已授权的仓库外清理，按权限流程在沙箱外执行，不据此建议重新挂载根分区。
- Go 模块目录可能默认只读；需要时仅恢复待删目录的用户写权限，清理范围限于用户指定路径，完成后检查残留，不顺带删除其他缓存。
- 证据：2026-10-26，沙箱内 `/home/coder/go` 显示只读；经权限流程恢复目录写权限并删除成功，复查路径不存在。

## 3. Rust 编译与 Clippy 跨 target 差异

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
- 替换 YAML 与二进制协议库时保留行为回归；协议编码改变须递增协议号。AGP 与 Gradle 成对升级，并对最低版本、预发布、损坏版本及未来不兼容主版本保留解析边界检查。具体版本及仍受上游约束的链路见[依赖核对表](../dependency-audit.csv)。
