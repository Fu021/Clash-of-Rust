# 本地构建与预览

[工作流索引](../workflows.md)。按需读取本页，不必加载全部工作流文档。

## 本地打包

桌面打包须在与目标系统、架构一致的机器上进行。除 README 中的原生编译依赖外，Linux 打包需 `dpkg-dev`，macOS 需系统 `hdiutil`、`codesign`，Windows 的 NSIS 由脚本下载并校验。Android 所需的 Go 和 SDK 另见[Android 构建说明](../../apps/android/README.md#本地构建)。

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
