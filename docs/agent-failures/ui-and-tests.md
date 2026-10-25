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
- 管理列表与运行配置共用有效数据，标明客户端默认与自定义来源；删除默认项须持久保存移除记录，还原只恢复默认来源。Fake-IP 回归用例覆盖三种过滤模式、同名来源合并和重启恢复，见[配置测试](../../crates/client-core/src/config/mod.rs)。

### 移动虚拟列表重叠与宽屏空间

- **症状**：Android 连接、规则、日志页的计数正常，却只看到最后一条，且记录出现在正文中间；Pad 的分页和规则操作横向铺满并占两行。
- **根因与修复**：Slint 1.18 按重复项直接父组件名 `ListView` 启用虚拟布局，封装名 `MList` 不被识别，重复项按普通元素重叠。保留原名并明确紧凑布局的内容原点；宽屏复用紧凑分页，规则提交与分页同排，侧栏按桌面分出“工具”。
- **证据与防回归**：[正式列表](../../crates/client-ui/ui/mobile/shared/controls.slint)、[页面](../../crates/client-ui/ui/mobile/shared/pages.slint)与[软件窗口回归](../../crates/client-ui/tests/mobile_lists.rs)。回归验证多行滚动范围、不同颜色行的实际位置、删至一条仍顶部对齐，以及横向变宽后的不同行高。固定数据预览曾复现“4 条规则仅显示第 4 条”，修复后按序显示；外观检查不代表设备网络已验证。
- 软件窗口的尺寸回归用 `preferred-width` / `preferred-height`，不要用固定宽高后再请求变宽；窗口约束会使宽区域的像素断言落到背景。先核对实际窗口尺寸，再验证重排，见 [CI 40108770502](https://github.com/Fu021/Clash-of-Rust/actions/runs/40108770502) 的 Linux ARM64 列表测试。

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
- Slint 软件窗口测试先核对实际布局与命中区域：布局外的固定大小组件默认居中，夹具应明确定位或复用正式布局，避免在左侧按压却把手柄放在行中间。不同探针切换前先隐藏窗口。再用小步虚拟时钟泵事件循环，处理滚动容器延迟转发与计时器绑定；直接跳时会让长按计时器在跳时后才启动。长按回归覆盖短按取消、阈值前后与跨行拖动，见 [CI 40163165989](https://github.com/Fu021/Clash-of-Rust/actions/runs/40163165989) 和[移动列表测试](../../crates/client-ui/tests/mobile_lists.rs)。
- 测试不应读取用户真实订阅、Cookie、密钥或历史数据。
- 记录“测试失败”与“测试前置失败”的区别，不能把后者写成功能回归。
