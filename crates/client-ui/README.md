# Slint 界面预览

独立的 Slint 页面原型，采用选定的 A「简洁原生」风格，支持浅色、深色和跟随系统三种外观选项（默认跟随系统）。覆盖十个页面，以及删除确认、草稿保护、规则与订阅编辑、Claude 保护提示和检测详情。预览通过固定数据模拟系统主题，尚未连接桌面程序的系统主题监听。

外观中提供强调色按钮和六色选择弹窗。当前仅保留预览会话内的选择状态，不应用全局配色、不保存配置；确认、取消和恢复默认均只操作预览数据。

`ui/concepts/style-study.slint` 另外提供首页的三种设计方向：简洁原生、柔和卡片和紧凑工作台，各有浅色、深色。这些方案独立于现有十页原型，用于选择设计方向。

当前只包含 `.slint` 文件，没有注册为 Cargo 工作区成员，不改动现有 Iced 应用、Windows 修复或原有构建流程。所有网络与配置数据均为演示数据，预览不会启动内核、更改系统设置或写入真实订阅。

使用官方预编译的 **Slint Viewer 1.18.1**，无需编译 Rust 项目。工具应安装在持久目录并加入 PATH，或通过 `--viewer` 指定。当前环境使用 `/home/coder/.local/bin/slint-viewer`，实际工具与运行库位于 `/home/coder/.local/share/slint/1.18.1/`。

```sh
# 生成全部页面、弹窗的浅色和深色截图，以及浏览图集。
python3 scripts/preview-slint.py --dialogs

# 生成可直接在回复中展示的浅色/深色成对截图，设置页展开完整高度。
python3 scripts/preview-slint.py --dialogs --pairs --full-settings

# 只生成一个页面，或检查默认尺寸下的布局。
python3 scripts/preview-slint.py --page connections --theme dark
python3 scripts/preview-slint.py --page home --theme light --size 950x700

# 在有图形桌面的环境中打开预览，保存 .slint 后自动刷新。
python3 scripts/preview-slint.py --page home --theme light --live

# 只检查语法与界面属性。
python3 scripts/preview-slint.py --check

# 三种风格的浅色、深色实际渲染对照，独立输出，不覆盖现有页面图集。
python3 scripts/preview-slint.py --concepts --output dist/slint-preview/styles

# 有图形桌面时，可通过预览侧栏的方案按钮切换三个布局。
python3 scripts/preview-slint.py --concepts --theme light --live --output dist/slint-preview/styles
```

默认输出为 `dist/slint-preview/`，其中 `index.html` 可查看所有原尺寸截图，`overview-light.png` 与 `overview-dark.png` 为页面总览。截图工具直接解释并渲染 Slint 文件，不调用 Cargo；无需 Xvfb 或运行中的桌面会话。

预览中可切换页面、主题和演示节点，展开快捷规则、打开弹窗，并切换空白规则的表单和文本模式。检测按钮只推进演示状态；文件导入、保存、删除、关闭连接等操作尚未接入业务。节点与连接筛选仅提供完整字段匹配；连接排序只演示方向切换，不代表完整的字段排序实现。

后续由 `src/models` 和 `src/bindings` 接入共享 Rust 业务，保留增量列表更新、任务取消与后台生命周期。移动端入口与布局继续预留，当前不实现 Android 或 iOS。
