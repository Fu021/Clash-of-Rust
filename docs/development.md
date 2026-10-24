# 开发与验证

按本次改动阅读职责、数据流、平台或验证的相关章节；跨任务约束与收尾流程见 [AGENTS](../AGENTS.md)。

## 目录与职责

| 目录 | 职责 |
| --- | --- |
| `crates/client-core` | 共享配置、存储、订阅、规则、控制器、DNS 与检测；不依赖 GUI。 |
| `crates/client-runtime` | 桌面/CLI 内核生命周期、平台网络、更新与共享控制服务。 |
| `crates/client-ui` | 正式 Slint 组件、界面数据与列表模型。 |
| `crates/desktop-protocol`、`crates/desktop-shell` | 宿主/窗口协议；托盘等桌面原生集成。 |
| `apps/desktop`、`apps/desktop-ui` | 常驻宿主与状态投影；可关闭并重建的窗口进程。 |
| `apps/cli` | `cor` 与后台入口，见 [CLI 文档](cli.md)。 |
| `crates/mobile-ffi` | 共享移动会话，以 `NativeHost` 接入原生能力；不依赖 JNI、Slint 或 Android SDK。 |
| `apps/android` | `native` Rust/Slint/JNI 宿主、`app` 生命周期/VPN/通知、`core` Go/mihomo 桥接。 |
| `apps/ios` | 独立宿主占位与 UI 原型，尚非可用 VPN 客户端。 |
| `resources`、`scripts`、`docs` | 必需资源、[维护脚本](../scripts/README.md)与文档。 |

界面细节见 [Slint 说明](../crates/client-ui/README.md)，Android 生命周期与构建见 [Android 说明](../apps/android/README.md)。尽量在现有层次解决问题，不把平台权限、文件访问或网络请求写进 `.slint`。

## 修改前后都要检查

- 读实际调用链、相关测试与 diff，保留用户未提交的内容，不修改无关文件或撤销他人改动。
- GUI/CLI 共享数据目录、后台所有者和内核；检查两端刷新、退出与重新附加，避免重复内核、轮询或测速。
- 界面选项贯通 Slint 状态/回调、协议、宿主、业务、持久化和刷新，并检查桌面/移动端。协议变更同步收发端、序列化、预览和测试，明确兼容性与版本变化。
- 外部更新不得覆盖规则、订阅或设置草稿；保留并发校验、删除确认、原订阅与配置原子保存/回滚，覆盖拒绝、超时、退出和重启恢复。
- 控制器只监听本机，使用随机密钥并处理内部端口冲突；密钥不得进入日志、示例或界面。
- 系统代理、TUN、DNS、提权、自启动、通知与更新沿用平台入口，检查授权、拒绝、重复操作和失败清理。
- Android VPN/通知独立于 Activity，关闭窗口不停止 VPN；保留套接字绕过自身 TUN。macOS 原生 UI/Dock/主线程、Windows 消息循环与窗口重建遵循各自宿主生命周期。
- 维护脚本优先扩展现有共享模块；独立权限、进程、离线测试或用户入口的保留依据见[脚本说明](../scripts/README.md)。

## 内存与响应速度

- 日志、连接、列表和缓存有界，按页面构建数据、复用模型，释放隐藏页面与关闭窗口的闲置资源。
- UI 线程或全局引擎锁内不做长时间网络/进程等待；后台任务可取消，迟到结果不覆盖新状态。轮询复用已有后台所有者。
- 性能结论必须实测并注明版本、环境、输入和方法；区分 RSS/PSS、窗口开/关与相同负载。基准按任务需要安排，常规使用低强度场景，压力场景另标；预览不作基准。

## 本地验证与 UI 预览

- 资源充足时优先做相关本地验证，再按风险扩大范围；沿用同一 Release profile 与 `--locked`。文档改动只做内容、链接等静态检查；新增测试覆盖行为、边界或恢复，不镜像实现。构建与 CI 范围见 [README](../README.md#编译与打包)及[工作流说明](workflows.md#ci)。
- UI 使用持久安装、版本匹配的官方 Slint Viewer 直接预览正式组件，可用 `--viewer PATH` 指定；参数见 [UI 预览](workflows.md#ui-预览与静态资源)及[Slint 说明](../crates/client-ui/README.md)。
- 桌面默认 950×700、最小 800×450，检查主题、强调色、字号、滚动、弹窗；移动端检查横竖屏、平板与系统字号。复用主题控件与间距，语义颜色不被强调色覆盖；安全区/键盘只扣一次，内容合理顶部对齐。
- 固定演示数据只能验证外观，外观改动附实际截图；交互、后端与系统功能另验。真实代理/TUN 测试只在可丢弃环境执行，不擅自改用户路由、防火墙、代理或重启网络。
- 汇报区分“通过 / 失败 / 未执行 / 仅静态检查”，说明资源、设备、授权或工具链缺口；其他提交的 CI 不算本次验证。

## 依赖维护

- [依赖核对表](dependency-audit.csv)记录 2026-10-24 的升级前后版本、官方元数据与兼容性限制；后续升级同步声明、锁文件和对应行，不把核对日期当成持续安全保证。
- YAML 使用维护中的 `yaml_serde` / `libyaml-rs`，替换已停止维护的 `serde_yaml` / `unsafe-libyaml`；保留原订阅、别名、标签和引号标量语义。桌面私有管道使用 `postcard`，协议号为 6，收发端须一起更新，保留帧长度、截断和尾随数据检查。
- HTTP 客户端统一使用 `clash_client_core::network::client_builder`，保留内置公共根证书；`reqwest` 0.13 的平台证书校验默认值不能直接用于未接入 Java verifier 的 Android 宿主。JNI 0.22 原生入口先经 `EnvUnowned::with_env`，工作线程经 VM 回调获取 `Env`，全局引用和异常处理遵循其生命周期。
- macOS `tray-icon` 仍固定在与 Slint 共用原生菜单类的版本。Slint/ksni 的 zbus 5、字体栈的 `ttf-parser` 及 `typed-index-collections` 的可选 bincode 尚受上游约束；不以强制替换不兼容主版本掩盖这些链路。
