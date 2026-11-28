# 本地构建与预览

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

## 本地 Rust 构建与磁盘管理

构建前同时检查会话实际可用的 CPU、内存和磁盘。Linux 容器还要核对 cgroup 的 CPU 配额和内存上限。资源充足时复用持久安装的 Rust 工具链，先在本机运行对应包的检查、测试或构建，再按[改动范围](../development.md#按改动选择验证)使用同提交 CI 和适用的兼容矩阵。资源受限时按 [Agent 开发指南](../../AGENTS.md#资源受限环境)执行。

- 首次构建、切换 Rust 版本或目标架构、连续多轮测试前后，检查产物体积和剩余空间。从 `cargo metadata --no-deps --format-version 1` 读取实际的 `target_directory`，兼顾 `CARGO_TARGET_DIR` 等设置。用 `du -sh`、`du -h --max-depth=2` 和 `df -h` 定位增长，并在任务收尾复查。
- 优先用 `cargo check -p <包名> --locked` 验证类型与依赖，按需要运行该包的测试或编译实际二进制。复用当前活跃配置的增量产物，避免无故重复构建多个架构或 profile。
- 构建产物持续增长或开始挤占后续构建空间时，停止追加构建，先确认哪些历史 profile、测试产物或独立 `CARGO_TARGET_DIR` 已经用完。确认没有并行任务使用它们并保留所需的验收产物，再按范围处理。可先用 `cargo clean --profile <名称> --dry-run` 核对范围。整目录清理和共享缓存维护遵循[占用锁协议](../development.md#仓库占用锁)。
- `target` 产物、Cargo 的依赖下载缓存与 Rust 工具链有独立的生命周期。对构建产物的清理只处理已确认的生成目录，保留工具链和仍有复用价值的下载缓存。
- 每项待办收尾时用 `df -h` 记录相关文件系统的占用和剩余空间，用 `du` 盘点实际构建目录（含独立的 `CARGO_TARGET_DIR`）及本轮临时目录。停止本次创建的模拟器或虚拟机，确认没有其他会话使用后，清理闲置的设备数据、镜像、临时文件和构建产物。保留仍需验收的证据、产物和可复用的工具链及依赖缓存。清理后用 `df -h` 和 `du` 复查，收尾汇报写明清理范围、清理前后占用及实际释放量。

## 本地打包

桌面打包须在与目标系统、架构一致的机器上进行。除 README 中的原生编译依赖外，Linux 打包需 `dpkg-dev`，macOS 需系统 `hdiutil`、`codesign`，Windows 的 NSIS 由脚本下载并校验。Android 所需的 Go 和 SDK 另见[Android 构建说明](../../apps/android/README.md#本地构建)。

资源充足且宿主支持所需工具时，可在本地构建和测试移动端，也可安装、运行 Android 模拟器验证交互。桌面与移动端仍须遵守[平台编译边界](../development.md#平台编译边界)；本地结果不代替同提交 CI 和适用的平台验收。涉及代理或 TUN 的测试在可丢弃的隔离环境中执行。

在仓库根目录执行以下命令。`resolve` 只冻结当前提交的锁文件、Android 声明与已确认工具基线，不查询或应用升级，也不修改源码依赖文件；这些文件须与当前提交一致。依赖升级与工具更新按[依赖维护](../development.md#依赖维护)执行，不能用旧快照覆盖当前声明。

```sh
python3 scripts/build_inputs.py resolve
cargo build -p clash-of-rust -p clash-desktop-ui -p clash-cli --release --locked --bins
python3 scripts/prepare-resources.py
```

再导出带校验清单的本地程序与测试程序，并打包。以下命令可用于 Bash 或 PowerShell；Windows 可用 `python` 替代 `python3`：

```sh
python3 -X utf8 -c "
import sys
from pathlib import Path
sys.path.insert(0, 'scripts')
from build_support import host_system, host_arch, cargo_messages
from workflow_artifacts import test_binaries, export_build, extract_build
messages = cargo_messages(['cargo', 'test', '-p', 'clash-of-rust', '--test', 'core_integration', '--release', '--locked', '--no-run', '--message-format=json'])
tests = test_binaries(messages)
archive = export_build(host_system(), host_arch(), tests, Path('dist/local-export'))
extract_build(archive, Path('dist/local-build'))
"
python3 scripts/build-installer.py --prebuilt dist/local-build
```

安装包与 SHA256 输出到 `dist/`，重复打包时为导出与暂存指定新目录。`--no-run` 仅编译集成测试程序，不执行代理/TUN 系统测试；本地产物清单不代表已经通过 CI，也不会创建 Release。`build-installer.py --prebuilt` 需要上述完整产物目录，不能直接传入 `target/release/`。`run-ci.py` 包含专用系统集成测试，不作为普通本地构建入口。

## UI 预览与静态资源

持久安装官方 Slint Viewer 后运行：

```text
python3 scripts/preview-slint.py --dialogs --pairs
```

脚本直接加载正式 `.slint` 组件和演示数据，不运行 Cargo 或真实内核。资源充足时默认生成 `dist/slint-preview`；资源受限时只检查语法或预览必要的一页和一种主题，并显式把输出放到工作区外：

```sh
python3 scripts/preview-slint.py --check
python3 scripts/preview-slint.py --page settings --theme light --output /tmp/clash-slint-preview
```

`--protection-state off|active|checking|error` 可预览全局保护徽标与异常原因，桌面和 `--mobile --platform android` 共用；仅注入演示状态，不连接内核或改变网络。

`--notice "订阅更新失败：连接超时" --notice-error` 预览错误通知及诊断按钮，不设置 `--notice-error` 则为正常通知。正式诊断采集与原生文件保存仍需在各端验证；iOS 预览没有可用原生导出宿主。

整体界面的三种架构、几何字标、浅深主题及警告样式见[界面设计与审阅](../ui-design.md)。`--pages-only` 跳过自动附加的移动情境矩阵，适合只预览本次选定的页面；仍须显式设置工作区外的 `--output`。维护者明确要求全量截图时串行渲染，不并行占用受限服务器。

使用仓库期间遵循[占用锁协议](../development.md#仓库占用锁)。静态检查与预览不等于编译、测试或真实交互验证。

图标、字体、国家名称及许可证属于必要资源；日常构建使用仓库成品。仅在维护资源时使用 `scripts/prepare-assets.py icons` / `countries`，不再生成国旗图集。图标源图保留于 `resources/icon-concepts`，Android 自适应图标使用生成后的 432px 前景图，第三方来源见各资源目录的说明。平台检测直接维护 Rust，必要的公开检测数据与 AGPL 来源说明统一位于 `resources/ip-check`，不再保留旧平台检测 `vendor` 或 Shell 生成工具。

## Markdown 与网页截图

`scripts/preview-page.py` 用无头 Chromium 把一个 Markdown 文档、本地 HTML 或指定网页导出为 PNG。首次在用户目录安装 Playwright、markdown-it-py 和浏览器，后续直接复用。Linux 主机还需 Chromium 系统库及能覆盖正文的字体，例如 Ubuntu 的 `fonts-noto-cjk`。

```sh
python3 -m venv ~/.local/share/clash-of-rust/preview
~/.local/share/clash-of-rust/preview/bin/python -m pip install 'playwright==1.65.0' 'markdown-it-py==4.3.0'
~/.local/share/clash-of-rust/preview/bin/python -m playwright install chromium --only-shell
```

从仓库根目录运行。网页示例使用现有网站构建入口生成的 `website/dist/`。

```sh
~/.local/share/clash-of-rust/preview/bin/python scripts/preview-page.py \
  docs/workflows/local-development.md --root . --full-page --output /tmp/cor-doc.png
python3 -B website/build.py --file-links
~/.local/share/clash-of-rust/preview/bin/python scripts/preview-page.py \
  website/dist/index.html --width 1280 --height 900 --output /tmp/cor-home.png
```

网页默认截取整页长图，`--viewport-only` 只截首屏。Markdown 默认截首屏，使用 `--full-page` 可生成长图。`--theme dark` 设置浏览器深色偏好。Markdown 支持表格、代码块和本地图片。预览本地文件时，只加载 `--root` 范围内的文件，默认范围是输入文件所在目录。打开 `https://` 或 `http://` 网页时按正常网页方式加载资源。截图输出为新文件，可直接发送给维护者审阅。
