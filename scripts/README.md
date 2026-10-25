# 脚本维护

本目录只保留构建、验证、打包、缓存、内存基准和 UI 预览入口。应用运行时不依赖这些脚本；新增脚本前先检查能否扩展现有共享模块或入口，不要为单次排障复制包装脚本。

## 职责清单

| 脚本 | 类型 | 职责 |
| --- | --- | --- |
| `build_support.py` | 共享模块 | 平台识别、版本、资源、下载、架构和暂存目录等标准库辅助。 |
| `build_inputs.py` | CLI / 模块 | 冻结并应用同一份 Rust、Go、Android 和资源依赖快照。 |
| `workflow_artifacts.py` | CLI / 模块 | 导出、下载和校验不可变 CI 产物，不重新编译。 |
| `action_cache.py` | CLI / 模块 | 认证私网缓存协议、恢复预算、校验和清单。 |
| `cache_shards.py` | 模块 | Rust 32 分片缓存的恢复、失效和增量上传。 |
| `workflow_storage.py` | CLI | 限制 GitHub Actions 热缓存容量并保留需要的版本。 |
| `prepare-resources.py` | CLI | 校验并准备 mihomo、Geo 和桌面共享资源。 |
| `prepare-assets.py` | 维护 CLI | 生成图标、国旗，或核验 Windows 嵌入图标；普通构建不运行。 |
| `android.py` | CLI | Android 编译、模拟器测试、产物导出与已测试 APK 打包。 |
| `build-installer.py` | CLI | 只组装已验证产物为 Windows、Linux 或 macOS 安装包。 |
| `macos_package.py` | 模块 | macOS App / DMG 组装；被打包器和 macOS 验证延迟导入。 |
| `publish-release.py` | CLI / 模块 | 校验成功 CI，复用已验证产物创建测试包或公开正式/预发布 Release。 |
| `run-ci.py` | CLI | 单个原生平台的编译、Clippy、源码测试与代理/TUN 集成入口。 |
| `windows_ci.py` | 模块 | Windows 普通用户、服务、安装器、更新及安装包/未压缩 EXE/DLL 的 Defender 验证，扫描证据进入打包收据。 |
| `macos_ci.py` | CLI / 模块 | macOS 原生验证、安装检查和诊断报告。 |
| `test-proxy-modes.py` | CLI | Windows、Linux、macOS 共用的系统代理、HTTP 和 TUN 流量检查。 |
| `test-linux-tun.py` | root CLI | Linux 隔离网络命名空间内的 TUN 授权、DNS 和恢复测试。 |
| `tun_dns_fixture.py` | 子进程夹具 | Linux TUN 测试的隔离上游；必须作为独立进程启动。 |
| `test-release-package.py` | CLI | 统一调度安装包结构、安装后验证和 Linux 兼容性检查。 |
| `run-memory-benchmark.py` | CLI | 隔离运行 API、GUI、托盘和 CLI 内存基准。 |
| `memory_desktop.py` | 模块 / 子进程 | 托盘宿主与窗口夹具；测量进程外运行，不能并入基准入口。 |
| `preview-slint.py` | CLI | 使用官方 Slint Viewer 检查或渲染正式组件，不调用 Cargo。 |
| `test-build-tools.py` | 离线测试 | 构建输入、缓存、打包、Windows/macOS 辅助逻辑的单元测试。 |
| `test-workflow-artifacts.py` | 离线测试 | 不可变产物、安装包和内存工作流的离线回归测试。 |

## 仓库内其他脚本

工作流复合操作 `.github/actions/local-cache/index.cjs` 只负责调用 `action_cache.py`，不复制缓存实现。`resources/claude-check/browser.js` 是检测页面资源，随客户端提供，保留在资源目录。

以下 Rust 构建脚本由各自的 Cargo 包调用，按包的构建生命周期保留；协议生成共用 `crates/desktop-protocol/schema.rs`，不另建包装入口：

| 路径 | 职责 |
| --- | --- |
| `apps/desktop/build.rs` | 注入应用版本；Windows 下编译图标和版本资源。 |
| `apps/desktop-ui/build.rs` | 从正式 Slint 状态生成窗口进程与共享协议之间的数据转换。 |
| `apps/android/native/build.rs` | 仅在 Android target 编译正式 Android Slint 入口并嵌入资源。 |
| `crates/client-ui/build.rs` | 编译桌面正式 Slint 入口。 |
| `crates/desktop-protocol/build.rs` | 从正式 Slint 状态生成序列化协议、默认值和状态访问接口。 |

## 合并与删除规则

- 新行为属于现有生命周期时，扩展已有共享模块或 CLI，不新增同义入口。
- 只有不同操作系统权限、独立进程边界、独立离线测试套件或清晰用户入口才保留独立脚本。
- `memory_desktop.py`、`tun_dns_fixture.py` 必须保持可独立启动；`macos_package.py`、`windows_ci.py` 保持可跨平台延迟导入，不能并入只适用于单一宿主的入口。
- 删除脚本前先用 `rg` 检查 `.github/`、文档、测试和生产脚本调用；同步删除引用、测试和缓存键影响。
- 共享路径、下载、版本、资源和暂存逻辑优先复用 `build_support.py`；排障逻辑进入现有平台模块或离线测试。
- 每次最终合并并推送 `main` 前，审查上述脚本及工作流内嵌命令，按职责和调用关系决定合并、删除或保留，并更新相关文档与失败复盘。

2026-10-24 审计结果：24 个 Python 脚本均通过静态语法解析，其中 23 个有工作流、生产脚本或测试调用者，`preview-slint.py` 是文档提供的独立用户入口；另核对了 5 个 Rust 构建脚本及上述 2 个 JavaScript 文件的职责。未发现可安全删除的孤立脚本，也没有需要合并的重复入口。未执行编译、打包或脚本功能测试。
