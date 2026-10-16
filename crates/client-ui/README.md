# Slint 桌面界面

采用 A「简洁原生」风格，十个页面与各类确认、编辑、详情弹窗共用 `ui/app.slint`。支持深色、浅色、跟随系统、字号缩放及六种强调色；外观设置自动保存。

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
