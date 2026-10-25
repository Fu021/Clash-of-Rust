# Android 工具链与 VPN

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 5. Android 工具链与原生依赖

**典型症状**

- SDK/NDK 下载包损坏：`Error on ZipFile unknown archive`。
- Skia、HarfBuzz 或 Chromium 源站 503，Git 结果不完整。
- Gradle lint、aapt2 元数据、GLES 依赖、Java/Rust 类型或 Android API 限制导致编译失败。

**代表证据**

- Android CI [39984691528](https://github.com/Fu021/Clash-of-Rust/actions/runs/39984691528)：NDK 28.2 archive 损坏，`sdkmanager` 返回非零。
- Android CI [39032112812](https://github.com/Fu021/Clash-of-Rust/actions/runs/39032112812)、[39004271043](https://github.com/Fu021/Clash-of-Rust/actions/runs/39004271043)、[39002539543](https://github.com/Fu021/Clash-of-Rust/actions/runs/39002539543)：三种 ABI 的编译或 lint 错误。
- CI [38966108986](https://github.com/Fu021/Clash-of-Rust/actions/runs/38966108986)：Android Clippy 拒绝可合并 `if`。
- 修复提交 `3fe26d8`、`023b126`、`4827013`、`65f3962`、`65446a7`、`7a24898` 等 Android 依赖与 native host 修复。

**首先检查**

- `scripts/android.py`、Gradle 配置、NDK/SDK 版本、Go module、Rust target 和构建标签是否一致。
- 下载缓存是否完整并校验；失败后是否清理不完整压缩包和 Git 源。
- Android target 是否遵守 stable Rust、`minSdk 26`、`compileSdk 36` 和 16 KiB ELF 对齐要求。

**防回归**

- 三种 ABI 都要编译；x86_64 还要运行模拟器集成测试。
- 新增 native 依赖必须固定来源、版本和校验，并保留许可证。
- 不把桌面端 feature、线程或文件系统假设带入 Android 宿主。

## 6. Android VPN、DNS 与系统 UI 时序

**典型症状**

- 真实代理/TUN 测试没有观察到 TUN 连接，或 VPN 重建期间出现直连窗口。
- 快捷磁贴未渲染、冷启动状态错误，或读取系统 UI 设置被拒绝。
- 模拟器连接异常、路由未就绪、DNS 不可用。

**代表证据**

- CI [39028411244](https://github.com/Fu021/Clash-of-Rust/actions/runs/39028411244)、[38993577546](https://github.com/Fu021/Clash-of-Rust/actions/runs/38993577546)：真实 TUN/模拟器连接失败或超时。
- CI [39771203085](https://github.com/Fu021/Clash-of-Rust/actions/runs/39771203085)：快捷磁贴未被 System UI 渲染。
- CI [39005660080](https://github.com/Fu021/Clash-of-Rust/actions/runs/39005660080)：target SDK 36 读取受保护设置触发 `SecurityException`。
- 修复提交 `a706150`、`0f7d6a3`、`a2efffa`、`7cd86ae`、`bd23c5b`、`ee44de5`。

**首先检查**

- 新 `VpnService` 接口、地址和默认路由是否已由系统回调确认，再启动 kernel packet readers。
- 测试是否设置 direct-only 负对照；若直连也能通过，测试结论无效。
- 快捷磁贴是否通过可访问的系统 UI/广播状态验证，而不是读取普通应用无权访问的 `Settings.Secure` 键。

**防回归**

- 覆盖首次授权、拒绝、取消后的迟到授权、已授权、VPN 重建和关闭清理。
- 关闭界面不得停止 VPN；套接字必须绕过自身 TUN。
- 测试辅助状态必须有稳定来源，等待状态变化而不是固定 sleep。
- VPN 授权测试不能把无障碍 `ACTION_CLICK` 返回成功当作对话框已关闭；要等应用窗口实际恢复前台与焦点，再发起下一次授权，避免连续拒绝/允许误点正在退出的旧窗口。证据见 [CI 40161810492](https://github.com/Fu021/Clash-of-Rust/actions/runs/40161810492) 的 Android x64 logcat。
- 订阅保护校验放在共享会话，在下载、保存或重载前拒绝编辑和更新受保护订阅，同时允许修改其他订阅；界面禁用按钮只提供反馈，不能代替业务校验。
- 网络开关同步移动协议、宿主投影、持久化与预览；依赖项被规范化关闭后要有重新开启入口。回归覆盖 DNS 覆盖关闭、重开、Fake-IP 手动恢复和重启保留，见[移动会话测试](../../crates/mobile-ffi/src/lib.rs)。
