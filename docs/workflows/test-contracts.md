# 修改前后的测试契约

[CI](ci.md#测试覆盖) · [安装验收](install-validation.md) · [UI 尺寸](../loop-ui.md)

推送前从正式入口追踪产品与测试双方，不能只搜索变更文件中的断言。测试名称、数量和截图不证明覆盖；应确认工作流实际执行哪些程序、测试依赖什么前置状态，以及失败后是否仍能收齐其他场景的证据。

## 入口与职责

| 范围 | 权威入口 | 修改时核对 |
| --- | --- | --- |
| 桌面界面与后台 | [桌面 UI 回归](../../apps/desktop/src/application/ui_tests.rs)、[集成测试](../../apps/desktop/tests/core_integration.rs) | 正式消息投影、真实指针回调、长名称、通知分级、保护门禁、后台状态与草稿保留。 |
| 共享与移动界面 | [软件窗口测试](../../crates/client-ui/tests/mobile_lists.rs)、[emoji](../../crates/client-ui/tests/emoji.rs)、[模型](../../crates/client-ui/tests/models.rs) | Android/iOS 原型完整入口、手机/Pad、列表/宫格、字号、滚动、触控区域、计时器与模型替换。iOS 原型不等于原生宿主。 |
| 原生窗口 | [Windows 窗口测试](../../apps/desktop-ui/tests/window_reopen.rs)、[macOS 入口](../../scripts/macos_ci.py)、[Linux 入口](../../scripts/linux_gui.py) | 首次显示、隐藏、重建、缩放、托盘和关闭策略；源码软件窗口不替代安装后原生窗口。 |
| Android 生命周期与网络 | [instrumentation](../../apps/android/app/src/androidTest/java/org/clashofrust/android/ProxyTunTest.java)、[会话退出](../../apps/android/native/src/session_worker.rs)、[运行脚本](../../scripts/android.py) | 常规 CI 执行真实授权、代理/TUN 流量、DNS 和恢复，并保留轻量宿主取消回归；应用分流随相关改动执行，设备生命周期和冷磁贴在扩展模式运行。 |
| 业务、持久化与协议 | [共享业务](../../crates/client-core/src/lib.rs)、[桌面运行时](../../crates/client-runtime/src/lib.rs)、[移动会话](../../crates/mobile-ffi/src/lib.rs)、[CLI](../../apps/cli/src/main.rs) | 输入校验、序列化及所有构造点、读取/写入失败、取消、恢复、保护态、凭据脱敏。UI 变化不得使宿主或协议只在预览中可用。 |
| 下载与真实 DIRECT | [下载路线](../../crates/client-core/src/subscription/routes.rs)、[订阅回归](../../crates/client-core/src/subscription/mod.rs)、[Linux 隔离测试](../../scripts/test-linux-tun.py) | 代理失败后 SOCKS5 远端解析、请求目标与完整响应；不可达节点下仍须真正完成订阅导入和更新，不能只测端口或 DNS。 |
| 构建、产物与发布 | [脚本回归](../../scripts/test-build-tools.py)、[产物回归](../../scripts/test-workflow-artifacts.py)、[CI 驱动](../../scripts/run-ci.py) | 条件编译、严格 lint、测试收集、平台职责、超时清理、失败阻止导出、同提交来源与安装收据。 |

## UI 改动逐项核对

1. 核对正式桌面、Android 与共享 iOS 原型的组件、状态、回调、协议、宿主和预览；找到所有消费者再修改，不能只改视觉夹具。
2. 按实际布局检查正文内边距、可见性、折叠位置、禁用条件和命中区域。测量按当前帧更新，模型增删、重命名和布局切换后不沿用旧坐标。几何观察仅在测试注册后开启，生产首屏须在没有观察者时可构造。
3. 对动作移位同时验证折叠时不可见、展开后能触发，以及默认直连、本地、远程、保护态、忙碌态。长名称优先省略，操作按钮仍可达。
4. 对通知、动画、滚动和设置导航，核对虚拟时钟、事件循环、焦点及控件生命周期；不以放宽坐标、固定等待或删除断言消除失败。
5. 核对进程边界。Activity 重建应附加已有内核；进程冷启动应从持久订阅和设置重新生成配置；磁贴应直接恢复已生成的 runtime。只写 runtime 的测试夹具不能代表三条路径都持久有效。
6. 最后检查全部测试入口及 CI 的选择条件，区分本地静态检查、源码回归、原生设备集成和人工专项。旧 Android 输入法专项保留显式入口，不混进日常网络验收。

## 推送门槛

- 先收齐同轮全部 job 的失败日志和必要附件，分清产品错误、测试前置错误及下游缺产物，再集中修复；不提前推送取消尚未结束的平台。
- 本地资源受限时只执行 Rust 格式、actionlint、Slint 正式入口、文档链接、diff 及不下载依赖的轻量 Python 回归；Rust 源码测试、Cargo/Gradle/Go 构建及模拟器交由 Actions。静态检查不等于 Rust/Java 编译通过。
- 下一提交仍须取得完整同提交 CI；只在安装包及所有必需 job 验收通过后进入发布。工作流、来源与设备边界见[测试覆盖](ci.md#测试覆盖)和[安装验收](install-validation.md)。
