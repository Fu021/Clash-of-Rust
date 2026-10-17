# Slint 桌面界面

采用 A「简洁原生」风格，十一个页面与各类确认、编辑、详情弹窗共用 `ui/app.slint`。支持深色、浅色、跟随系统、字号缩放及六种强调色；外观设置自动保存。

- `ui/state.slint`：界面数据与操作协议，不包含网络、文件或授权逻辑。
- `src/models.rs`：复用列表模型，按变更更新行；桌面投影按页构建，隐藏后释放列表。
- `apps/desktop/src/application`：应用状态与校验、可取消任务、桌面绑定；原生事件只由 `bindings.rs` 接触 Slint/winit。
- `ui/pages`、`ui/components`、`ui/dialogs`：正式程序、无 Rust 预览与截图测试使用的同一套组件。

默认窗口 **950×700**，最小 **800×450**；正文与按钮 15px，辅助信息 14px，控件至少 40px。间距集中在 `Theme`：卡片内边距 16px、卡片间距 12px、内容间距 8px、设置分组间距 20px，弹窗内边距 24px、分区间距 16px。字体放大时控件随内容增高，超出窗口的内容滚动显示。

正式应用使用 winit 与软件渲染器，无需 WebView、Qt 或 GPU 渲染器。系统代理、TUN、托盘、Dock、自动更新与文件选择仍由桌面平台模块负责；共享业务库不依赖 Slint，Android/iOS 继续预留。

## 无需 Rust 编译的外观预览

安装官方 **Slint Viewer 1.18.1** 到持久目录。当前环境为 `/home/coder/.local/bin/slint-viewer`，也可通过 `--viewer PATH` 指定。

```sh
python3 scripts/preview-slint.py --check
python3 scripts/preview-slint.py --dialogs --pairs
python3 scripts/preview-slint.py --page settings --theme both --full-settings
python3 scripts/preview-slint.py --page connections --size 800x450
```

输出在 `dist/slint-preview/`。`ui/preview.slint` 只向正式组件填入固定演示值，不接入真实操作；此工具用于外观检查。它无需 Cargo、Xvfb 或桌面会话。

## 应用与交互验证

正式程序的编译、Clippy 和测试统一由 GitHub Actions 执行。本地使用上面的 Slint Viewer 直接预览界面，不编译桌面应用；截图工作流与可选 Rust 截图入口已移除。

源码单元测试保留任务取消、规则草稿、连接排序、模型复用与释放、真实指针输入和弹窗保护；无需显示服务器，不启动真实内核。节点卡片单击选择，测速按钮独立操作，测试确认按钮第一次点击便执行且不会顺带选择节点。模型测试复用正式界面库，避免重复编译组件。

## WSL 安全网络接口

WSL 原生页追加为索引 **10**，原有页面索引不变。`Intent::WslAction` 使用 `(action, mode)`：Doctor、Status、Install 预检、Test 的 mode 为 `preview`，Install 执行、Start、Stop、默认 Rollback 为 `execute`。默认 Rollback 文案为“回退 · 保留保护”，只断开隧道、保留 guard；“恢复原网络…”发送 `(Rollback, restore-confirm)`，desktop 弹出确认后才允许设置 `restore_original_network=true`。恢复原网络会撤销保护，仅允许所有 WSL 发行版均已停止时执行。`Intent::WslInput` 使用 `(key, value)`，key 为 `backend`、`endpoint-ipv4`、`endpoint-port`、`distro`、`profile-path`。UI 不启动 helper、发行版或提权流程；私密 JSON 只传路径，管理员命令由 desktop 完整校验后提供，显示和复制不代表已获授权或已执行。

`UiData` 将隧道连接、德国 IPv4 出口、保护与功能验收分别投影；generation 为十进制 u64 字符串。缺失 strict 全 WSL 冷启动证据时必须 Blocked。Docker Desktop 独立适配前须封闭外网、保留数据，未适配不可算功能通过。操作忙碌时禁用网络动作与配置输入，侧栏仍可导航。

`ViewModels` 通过 `attach()` 绑定 `wsl_distributions`、`wsl_checks`、`wsl_logs`。`set_wsl_results(generation, distributions, checks, logs) -> bool` 拒绝旧代次；返回 false 时 desktop 也应丢弃同一快照的标量状态。同代 operation_id 与异步完成顺序由 desktop 校验。输入变化时传新代次和空列表废弃旧证据。`clear_wsl_results()` 与 `clear()` 清空行但保留代次下限；隐藏再显示可重新投影同代结果。

发行版最多 128 行，检查与日志各 256 行；发行版/检查超限使用明确的未展示提示行，日志保留最新记录。展示文本按 UTF-8 字符边界限制长度，截断加省略号。这些限制不代替脱敏，desktop/core 必须只提交安全文本。`tests/wsl/mod.rs` 复用 `ui_models` 测试入口，覆盖代次极值、证据清理、模型身份、行数上限与多字节文本边界；由 Actions 运行。

desktop 测试覆盖预检与执行参数、Test 为 preview、默认 Rollback 保留 guard、restore-confirm 取消不执行、busy 时输入和重复动作不可触发、权限不足仅展示安全管理员命令、旧代次/旧 operation_id 不回写，以及连接成功但 strict 证据缺失仍为 Blocked。CI 是否实际执行这些测试只能以对应 Actions 运行记录为准。仍需补充真实 WSL/Hyper-V、运行中发行版恢复阻止和 Docker Desktop 后端故障测试；未适配的 Docker 不计功能通过。Viewer 可用 JSON `{"preview-page":10}` 直接预览；`preview-wsl-busy` 与 `preview-wsl-supported` 可切换固定演示状态。
