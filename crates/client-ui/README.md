# Slint 桌面界面

采用 A「简洁原生」风格，十个页面与各类确认、编辑、详情弹窗共用 `ui/app.slint`。支持深色、浅色、跟随系统及字号缩放；强调色仅预留选择界面，暂不改变全局配色或保存配置。

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

根目录 `cargo run` 启动正式桌面。`cargo test --workspace` 同时运行应用状态、任务取消、分页与模型复用测试，以及十个页面和主要弹窗在默认、小窗口尺寸下的软件渲染检查；无需显示服务器，也不会启动真实内核。

`ui-preview` 为可选的隔离截图功能，官方安装包不启用。`tests/ui_preview.rs` 使用真实应用状态构建页面；`UI Screenshots` 工作流在六种原生 runner 上保存截图。设置 `CLASH_UI_HEADLESS=1` 可在无桌面环境运行相同截图入口。
