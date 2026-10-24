# Slint 界面与测试稳定性

[返回索引](README.md)。只读取与当前症状相关的章节；编号沿用历史复盘，便于引用。

## 4. Slint 界面契约与预览漂移

**典型症状**

- `.slint` 属性、回调、页面索引或演示 fixture 与正式入口不一致。
- 编译生成代码出现语法或类型错误，例如 “this looks like an `else if` but the `else` is missing”。
- 旧 UI 截图/预览脚本依赖已删除的测试钩子或固定坐标，出现找不到选择器、卡片或 tray 函数。

**代表证据**

- CI [39773451589](https://github.com/Fu021/Clash-of-Rust/actions/runs/39773451589)：桌面 UI target 在一次界面变更后出现大量生成代码编译错误。
- UI Screenshots [38595379460](https://github.com/Fu021/Clash-of-Rust/actions/runs/38595379460)：旧临时任务引用已不存在的 `native_present`、`remove_native_item_for_test`、`verify_native_state`。
- Memory Benchmark [38102403454](https://github.com/Fu021/Clash-of-Rust/actions/runs/38102403454)、[38058824407](https://github.com/Fu021/Clash-of-Rust/actions/runs/38058824407)、[38058444877](https://github.com/Fu021/Clash-of-Rust/actions/runs/38058444877)：旧预览脚本依赖固定坐标和分类卡片布局。

**首先检查**

- Slint 状态/回调、协议、宿主投影、正式窗口入口和预览 fixture 是否同步。
- `scripts/preview-slint.py --check` 是否能加载正式入口；预览数据是否含已删除属性。
- UI 测试是否通过正式消息路径，而不是复制状态或依赖临时截图脚本。

**防回归**

- 界面外观只使用持久安装、版本匹配的官方 Slint Viewer；不要恢复截图工作流。
- 交互测试使用稳定标识/可访问文本和状态等待，不依赖屏幕绝对坐标。
- 预览固定数据只能证明外观，不能写成真实交互、VPN 或系统集成已通过。

## 11. 测试稳定性与外部环境假设

**典型症状**

- 单元测试依赖真实订阅、默认节点或固定端口，换 target 后失败。
- fixture 文件已存在、端口绑定失败、TLS 根证书或反向 DNS 不可用。
- UI/窗口测试固定等待时间，偶发超时或把合成器崩溃算作应用失败。

**代表证据**

- CI [39618278378](https://github.com/Fu021/Clash-of-Rust/actions/runs/39618278378)：规则测试因 fixture 没有 `proxies`/`proxy-providers` 失败。
- CI [39005660080](https://github.com/Fu021/Clash-of-Rust/actions/runs/39005660080)：provider 自动组断言缺项，另有构建工具 `KeyError`。
- 临时验证 [38640723255](https://github.com/Fu021/Clash-of-Rust/actions/runs/38640723255)：重复创建 `target/claude-build` 触发 `FileExistsError`。
- Memory Benchmark [39803098220](https://github.com/Fu021/Clash-of-Rust/actions/runs/39803098220)：等待恢复窗口超时。
- 修复提交 `9960618`、`2fbd14b`、`f5374b6`、`c87afd1`、`84c3a2e`、`2efd99b`。

**首先检查**

- fixture 是否自包含、固定时钟/输入，且不继承运行环境的 `HOME`、代理、DNS 或数据目录。
- 每个测试是否独占临时目录、动态端口和明确清理路径。
- 等待条件是否可观测并与超时分开；失败时是否保留脱敏诊断。

**防回归**

- 不把固定 `sleep`、固定坐标、固定端口或系统默认路由当同步机制。
- 测试不应读取用户真实订阅、Cookie、密钥或历史数据。
- 记录“测试失败”与“测试前置失败”的区别，不能把后者写成功能回归。
