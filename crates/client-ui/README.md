# Slint 界面

采用 A「简洁原生」风格，十个页面与各类确认、编辑、详情弹窗共用 `ui/app.slint`。支持深色、浅色、跟随系统、字号缩放及六种强调色；外观设置自动保存。

- `ui/state.slint`：界面数据与操作协议，不包含网络、文件或授权逻辑。
- `src/models.rs`：复用列表模型，按变更更新行；桌面投影按页构建，隐藏后释放列表。
- `apps/desktop/src/application`：应用状态与校验、可取消任务、桌面绑定；原生事件只由 `bindings.rs` 接触 Slint/winit。
- `ui/pages`、`ui/components`、`ui/dialogs`：正式程序与 Slint Viewer 预览使用同一套组件。

默认窗口 **950×700**，最小 **800×450**；正文与按钮 15px，辅助信息 14px，控件至少 40px。间距集中在 `Theme`：卡片内边距 16px、卡片间距 12px、内容间距 8px、设置分组间距 20px，弹窗内边距 24px、分区间距 16px。字体放大时控件随内容增高，超出窗口的内容滚动显示。

正式应用使用 winit 与软件渲染器，无需 WebView、Qt 或 GPU 渲染器。系统代理、TUN、托盘、Dock、自动更新与文件选择仍由桌面平台模块负责；共享业务库不依赖 Slint，Android 使用独立原生宿主，iOS 保留占位。

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

正式程序可通过本地 Rust 工具链或 GitHub Actions 编译、Clippy 和运行源码单元测试，入口见[项目 README](../../README.md#编译与打包)。只检查外观时可使用上面的 Slint Viewer，无需编译桌面应用；截图工作流与可选 Rust 截图入口已移除。

源码单元测试保留任务取消、规则草稿、连接排序、模型复用与释放、真实指针输入和弹窗保护；无需显示服务器，不启动真实内核。节点卡片单击选择，测速按钮独立操作，测试确认按钮第一次点击便执行且不会顺带选择节点。模型测试复用正式界面库，避免重复编译组件。

## Android / iOS 界面

移动端沿用当前桌面的配色、图标、卡片与操作层次，正文 16px、辅助信息 14px、触控区至少 48px。Android 与 iOS 分别维护页面组合、导航入口、设置与系统操作协议，未来可独立增减功能：

```text
ui/mobile/
├── shared/
│   ├── state.slint       # 移动数据、操作协议与导航/草稿状态
│   ├── controls.slint    # 触控控件与滚动
│   ├── frame.slint       # 底栏、侧栏、安全区、键盘与通知
│   ├── pages.slint       # 共用业务页面
│   ├── details.slint     # 详情、编辑与确认
│   ├── settings.slint    # 共用外观、网络开关、Geo 分组
│   ├── fixtures.slint    # 仅预览使用的演示数据
│   └── preview.slint     # 仅预览使用的状态栏与键盘示意
├── android/
│   ├── app.slint         # Android 页面与导航组合
│   ├── settings.slint    # VPN、应用分流、通知速度、电池与端口
│   ├── state.slint       # Android 数据与原生操作协议
│   └── preview.slint
├── ios/
│   ├── app.slint         # iPhone / iPad 页面与导航组合
│   ├── settings.slint    # 按需连接、系统 VPN 与版本信息
│   ├── state.slint       # iOS 数据与原生操作协议
│   └── preview.slint
└── preview-data/         # 固定预览资源
```

`MobileData` 不依赖桌面 `UiData`，只复用不含平台行为的列表行类型；界面请求交由各平台宿主执行，VPN 会话与内核进程状态分别传入。移动端不显示桌面安装器、托盘或窗口尺寸。两端都保留订阅、节点、连接、规则、日志、网络诊断、IP 检测和 A÷检测。

Android 已接入 `apps/android/native`、`apps/android/app` 与 `crates/mobile-ffi`：正式窗口不导入 fixtures，业务操作进入共享会话，VPN、文件与通知进入 Android 宿主，详见 [Android 说明](../../apps/android/README.md)。iOS 保持独立界面原型和宿主占位。两端预览仍使用固定数据，只用于检查外观与导航，不能代替实际设备上的操作验证。

```sh
python3 scripts/preview-slint.py --mobile --check
python3 scripts/preview-slint.py --mobile --platform both --theme both --pairs
python3 scripts/preview-slint.py --mobile --platform android --page proxies --size 834x1194
python3 scripts/preview-slint.py --mobile --platform ios --page settings --size 1194x834
```

输出分别位于 `dist/mobile-preview/android/` 与 `dist/mobile-preview/ios/`。涵盖全部页面、深浅色、手机横屏、Pad 双方向和分屏、小屏、125% 字号、未连接、通知进度及详情/编辑/确认场景。宽度均为逻辑尺寸，布局按扣除安全区后的可用宽高决定：

- 宽度不足 640 且非短窗口：底部五个入口，详情进入独立页面；工具页收纳连接、规则、日志和检测入口。
- 中等宽度或高度不足 480：紧凑侧栏，减少横屏头部占用；节点按内容空间改为双列。
- 可用宽度至少 1024、非短窗口：完整侧栏；内容宽度达到 880 时支持列表与详情并排，设置分两列。

Android 正式窗口已接入系统安全区、键盘、权限、前台 VPN、更新与文件导入；iOS 宿主保留独立占位。不能重复扣除系统已经缩小的视口，状态栏只是预览示意。导航与草稿状态独立于布局，删除、离开未保存的编辑须确认。
