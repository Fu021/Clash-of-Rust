# Windows 本地权限排查与验证（2026-10-10）

环境：Windows x64，当前账号的普通权限交互会话；原安装版本 0.5.0。
源码来自 `fix/windows-permissions` 的下载目录，本目录没有 Git 元数据。

## 已确认的本机原因与修复

现有 `ClashOfRust-<当前账号 SID>` 计划任务的 DACL 仅授予当前账号
`FR`（读取），管理员和 SYSTEM 才能管理任务。普通权限将 `Enabled`
写回原值即可复现 `E_ACCESSDENIED / 0x80070005`。

使用 `scripts/repair-local-autostart.ps1`，在一次 UAC 授权后核对当前账号
SID、注册表安装标记和任务执行路径，授予该账号任务完全控制权限。
保留任务原来的开启状态、执行路径、触发器和运行权限。
修复后在普通权限会话中实际关闭、开启该任务成功，并恢复为原来的开启状态。
操作前后 DACL 保存在 `dist/local-windows/autostart-repair.json`。

本机设置文件的 ACL 已授予当前账号完全控制。安装版 0.5.0 使用隔离数据
通过了前台、后台启动和旧设置升级测试，因此没有在本机复现启动保存失败。
不能将此前其他机器的保存故障归因于本机计划任务 DACL。

## 回归入口

`scripts/test-windows-local.py` 使用当前真实普通权限交互会话，创建临时数据
目录、随机端口和独立密钥，关闭系统代理及 TUN，检查真实 GUI 进程、内核
控制接口、设置和配置保存，并通过退出事件正常关闭。测试前需退出其他
Clash of Rust 实例，避免会话内单实例锁干扰。

```powershell
python scripts/test-windows-local.py bundle/local-windows/clash-of-rust.exe --transaction-recovery
cargo test --lib native_ordinary_user -- --ignored --test-threads=1
```

第二项仅创建并清理随机命名的测试任务，验证普通用户直接注册任务、
禁用、重新注册启用及删除；强制确认走 Task Scheduler，不能被 Run 回退掩盖。
Windows CI 的普通账号测试也已加入这一项。

修复版真实 GUI 已验证：前台及后台启动、旧设置升级、损坏设置与遗留
事务日志的启动回滚、内核接口就绪、离线 Geo、文件保存和正常退出。
最终全目标常规测试共 117 项通过；新增普通用户原生任务测试单独通过。
8 项真实内核集成测试及 1 项原生托盘测试也全部通过，共 127 项 Rust
测试通过；在线下载及需要专门管理员夹具的测试未运行。
`cargo fmt --all -- --check`、全目标 Clippy `-D warnings` 通过。
Python 构建工具测试 23 项（1 跳过），产物测试 5 项通过。

`bundle/local-windows/clash-of-rust.exe` 是本地调试构建，配套资源复用本机
安装版离线资源；不是发布安装包。本次未验证 ARM64，也未重新运行远程 CI。
