# 测试环境与稳定性

[返回复盘索引](README.md)。界面契约和交互行为见[界面复盘](ui-and-tests.md)。

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
- Slint `duration` 属性在 Rust 生成接口中使用 `i64` 毫秒，不能向 setter 传 `std::time::Duration`；后者仅用于测试时钟等 Rust API。[CI 40292027882](https://github.com/Fu021/Clash-of-Rust/actions/runs/40292027882) 的 `E0308` 来自该接口误用，检查两端参数化测试时保留即时拖动和长按拖动两种时长。
- 测试不应读取用户真实订阅、Cookie、密钥或历史数据。
- 记录“测试失败”与“测试前置失败”的区别，不能把后者写成功能回归。
- 图形授权后，带 capability 的内核进程可能拥有 root 属主的 `/proc` 项或不可读的 `exe` 链接；[安装验证](../../scripts/linux_gui.py)以进程状态中的真实/有效 UID、名称和父进程识别内核，仍验证 GUI/CLI 共享唯一进程及窗口重建后状态。[打包 40810105370](https://github.com/Fu021/Clash-of-Rust/actions/runs/40810105370)暴露了旧探针未找到内核。
- Ubuntu 20.04 用户空间容器仍使用宿主内核；目标最低系统须用旧版内核的虚拟机或真机验证。分别记录架构和内核，逐个检查发行 ELF 的 GLIBC 与链接依赖，不把 22.04 或最新版的通过结果当作 20.04 已通过。
