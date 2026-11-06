# Slint 界面

采用 A「简洁原生」风格，十个页面与各类确认、编辑、详情弹窗共用 `ui/app.slint`。支持深色、浅色、跟随系统、字号缩放及六种强调色；外观设置自动保存。

- `ui/state.slint`：界面数据与操作协议，不包含网络、文件或授权逻辑。
- `src/models.rs`：复用列表模型，按变更更新行；桌面投影按页构建，隐藏后释放列表。
- `apps/desktop/src/application`：应用状态与校验、可取消任务、桌面绑定；原生事件只由 `bindings.rs` 接触 Slint/winit。
- `ui/pages`、`ui/components`、`ui/dialogs`：正式程序与 Slint Viewer 预览使用同一套组件。

默认窗口 **950×700**，最小 **800×450**；正文与按钮 15px，辅助信息 14px，控件至少 40px。间距集中在 `Theme`：卡片内边距 16px、卡片间距 12px、内容间距 8px、设置分组间距 20px，弹窗内边距 24px、分区间距 16px。字体放大时控件随内容增高，超出窗口的内容滚动显示。

订阅和节点使用紧凑卡片，纵向内边距分别为 10px，卡片间距仍为 12px；桌面以内容区宽度和字号计算列数，订阅卡片参考最小宽度 300px、节点 280px，宽窗口继续增加列数，大字号减少列数。布局只重排现有分页模型，不重新获取网络数据；同一页内展开的详情按订阅 ID 或组名/节点名保留。最后一行留空槽位，奇数尾项不会丢失或横向拉满。完整名称、订阅来源及编辑/删除放在“详情”，使用、更新和测速保持直接可达；订阅支持按名称或来源搜索，筛选和翻页仍遵循原有滚动规则。没有流量/到期元数据时不虚构进度条或状态。

设置页“网络”提供 DNS 覆盖、Fake-IP 开关和独立排除项页面。列表合并同名域名并标注自定义/DNS 覆盖来源；删除内置项后显示移除数量及“一键还原 DNS 覆盖项”，还原保留自定义项。关闭 DNS 覆盖时关闭 Fake-IP，但保留列表和移除记录，订阅原有过滤仍生效。额度、持久化和 CLI 格式统一见[配置说明](../../docs/cli/maintenance.md#配置诊断与检测)。

滚动容器按分页、筛选或详情身份变化回到顶部；同一内容的后台刷新保留阅读位置，内容缩短时限制偏移，避免空白正文。桌面使用 `PageScroll`，移动端使用 `MScroll` / `ListView`，不要用变化中的条目总数或检测进度作为内容身份。移动虚拟列表的封装必须保留 `ListView` 名称，Slint 1.18 按重复项直接父组件的名称启用行布局与虚拟化；改名会使多条记录重叠。

桌面规则页通过左侧手柄直接拖动排序，移动端长按后拖动。拖动时整条规则浮起并跟随鼠标或手指，保留按下位置，原位淡化并高亮目标；靠近列表上下边缘时连续滚动，源规则滚出屏幕仍可继续，松手才提交排序。支持键盘上下键和 Esc 取消。移动结果按规则 ID 定位到完整草稿，搜索或来源筛选不会把可见行号误当作实际位置。右箭头打开完整规则、来源、策略、启停状态及编辑、删除、跨页移动等操作，修改仍需应用。规则页顶部操作随列表滚动，移动连接与日志页的搜索和筛选同样随内容滚动；工具栏不随列表数据或键盘可用高度变化而重建，以保留输入焦点；键盘展开时规则和连接尾栏随内容滚动，给搜索结果留出空间。

正式应用使用 winit 与软件渲染器。系统代理、TUN、托盘、Dock、更新与文件选择由桌面平台负责；共享业务不依赖 Slint，Android 使用原生宿主，iOS 保留占位。

节点名称、策略组名称和当前策略在列表、首页、详情和操作中均保持订阅原文，包括前后空格、国家代码、旗帜及普通 emoji；不删除前缀、不推断地区，也不自动补旗帜。桌面和 Android 使用随包的 [Noto Color Emoji](../../resources/fonts/SOURCE.md#noto-color-emoji) 及共享字体回退注册。桌面保留软件渲染器，局部回移 Slint 已合入的[彩色字形修复](../../vendor/slint-renderer-software/PATCHES.md)，Android 保留现有 Skia 路径。IP 检测与 Claude 保护/检测同样使用 emoji 国旗，摘要、结果、递归 DNS 和详情均传递文本，不再维护独立国旗图片或图片缓存。港澳台在报告中仍映射中国国旗并保留地区文字；这套报告映射不参与节点或策略组名称显示。

输入框在已提交文字和输入法组词文字均为空时才显示占位提示；组词尚未提交时也须隐藏提示，桌面与移动端共用这个条件。移动行标题可换行，右侧值限制宽度，完整内容仍在详情中展示。窄于 360px、字号至少 125% 或显示返回入口的手机页头将状态另起一行，避免页名被运行状态挤压。

桌面窗口进程在原生窗口建立后才报告就绪，统一处理显示、取消最小化、位置校正与激活；移动或主题变化不重复触发尺寸调整。Windows 的 `apps/desktop-ui/tests/window_reopen.rs` 使用真实界面进程检查关闭重开、屏幕外位置恢复、最小化/隐藏恢复与窗口消息响应，不启动内核或修改网络。

## 无需 Rust 编译的外观预览

安装官方 **Slint Viewer 1.18.1** 到持久目录并加入 `PATH`，例如 `~/.local/bin/slint-viewer`；也可通过 `--viewer PATH` 指定实际位置。不要把个人路径写入仓库配置。

```sh
python3 scripts/preview-slint.py --check
python3 scripts/preview-slint.py --dialogs --pairs
python3 scripts/preview-slint.py --page settings --theme both --full-settings
python3 scripts/preview-slint.py --page connections --size 800x450
python3 scripts/preview-slint.py --mobile --platform android --page rules --theme light --size 360x800 --scroll-y -360 --output /tmp/clash-rules-preview
```

输出默认在 `dist/slint-preview/`。`ui/preview.slint` 只向正式组件填入固定演示值，不接入真实操作；此工具用于外观检查。它无需 Cargo、Xvfb 或桌面会话。

官方 Viewer 1.18.1 的软件渲染器不包含本仓库的彩色字形回移；不能用它的 emoji 留白判断正式应用仍有相同缺陷。Skia 预览可核对内置字体与布局，正式软件渲染的颜色、输入框、透明度、裁切和重绘由 `tests/emoji.rs` 在 Actions 验证。

资源受限（例如 1 核 CPU、约 1 GiB 内存）时只运行 `--check`，或按[多尺寸检查](../../docs/loop-workflow.md#多尺寸-ui-检查)串行预览必要页面的一种主题；`--scroll-y` 可检查规则页滚动后的内容。使用 `--output /tmp/...` 把产物放到工作区外，并在结束前删除，不能在仓库留下 `dist/`、缓存或截图：

```sh
python3 scripts/preview-slint.py --check
python3 scripts/preview-slint.py --page settings --theme light --output /tmp/clash-slint-preview
```

## 应用与交互验证

构建、Clippy 与源码测试见[项目 README](../../README.md#编译与打包)；受限环境只用 Slint Viewer 检查外观，不恢复截图工作流或 Rust 截图入口。

源码单元测试保留任务取消、规则草稿、连接排序、模型复用与释放、真实指针输入和弹窗保护；无需显示服务器，不启动真实内核。节点卡片单击选择，测速按钮独立操作，测试确认按钮第一次点击便执行且不会顺带选择节点。模型测试复用正式界面库，避免重复编译组件。

## Android / iOS 界面

移动端沿用当前桌面的配色、图标、卡片与操作层次，正文 16px、辅助信息 14px、触控区至少 48px。Android 与 iOS 分别维护页面组合、导航入口、设置与系统操作协议，未来可独立增减功能：

移动订阅和节点同步紧凑信息层级、选中状态与“详情”入口，减少纵向留白并默认收起来源和低频操作；保留全部字段及保护锁定规则。节点名称可换行，协议/延迟与操作分行，避免窄屏大字号挤掉信息。订阅保持单列，节点沿用现有手机单列/宽屏双列逻辑，不套用桌面的动态列数算法；触控按钮仍至少 48px，字号遵循系统无障碍设置。

| 目录 | 职责 |
| --- | --- |
| `ui/mobile/shared` | `state.slint` 定义协议与草稿；`controls.slint` / `frame.slint` 管理触控、导航、安全区与键盘；`pages.slint`、`details.slint`、`settings.slint` 为共用业务界面。 |
| `ui/mobile/android` / `ui/mobile/ios` | 各自的 `app.slint`、`settings.slint`、`state.slint` 组合平台页面、设置及原生操作协议。 |
| 各平台 `preview.slint`、`shared/fixtures.slint` 和 `preview-data` | 固定演示数据、状态栏、键盘示意及预览资源；正式窗口不导入。 |

`MobileData` 不依赖桌面 `UiData`，只复用不含平台行为的列表行类型；界面请求交由各平台宿主执行，VPN 会话与内核进程状态分别传入。移动端不显示桌面安装器、托盘或窗口尺寸。两端都保留订阅、节点、连接、规则、日志、网络诊断、IP 检测和 A÷检测。

通知由宿主显式分级，不按错误文字推断：

| 等级 | 例子 | 呈现 |
| --- | --- | --- |
| 成功或信息 | 已保存、检测完成 | 顶部轻量提示，8 秒后自动关闭 |
| 可恢复失败 | Geo 更新、检查更新、测速、订阅更新失败且原状态保留 | 顶部详细错误栏，手动关闭，不阻止其他页面操作 |
| 严重故障 | 内核初始化/启动失败、意外停止、VPN 无法启动、保护恢复失败 | 屏幕中央警告弹窗，暂时阻止背景操作；普通后台结果不能覆盖 |

两类错误都直接展示详情、保留到主动关闭，提供“导出诊断”；桌面同时可复制 `cor diagnose --output cor-diagnose.json`。页面布局不移动，过长正文可滚动，关闭按钮始终可见，不用“详情”折叠。关闭、Escape 或 Android 返回优先关闭报错，保留原页面与表单草稿；Android 原生 Back 与 Slint `Key.Back` 均须处理。表单错误、外部 TUN 提醒以及日志页也有诊断入口。导出在宿主后台执行；报告范围、脱敏与失败处理以[故障反馈](../../docs/cli/runtime.md#故障反馈与诊断报告)为准。用 `preview-slint.py --notice "Geo 更新失败：连接超时，旧数据仍可用" --notice-error` 预览错误栏，改用 `--notice-critical` 预览居中弹窗；预览不能代替原生保存交互验证。

Android 已接入 `apps/android/native`、`apps/android/app` 与 `crates/mobile-ffi`：正式窗口不导入 fixtures，业务操作进入共享会话，VPN、文件与通知进入 Android 宿主，详见 [Android 说明](../../apps/android/README.md)。iOS 保持独立界面原型和宿主占位。两端预览仍使用固定数据，只用于检查外观与导航，不能代替实际设备上的操作验证。

```sh
python3 scripts/preview-slint.py --mobile --check
python3 scripts/preview-slint.py --mobile --platform both --theme both --pairs
python3 scripts/preview-slint.py --mobile --platform android --page proxies --size 834x1194
python3 scripts/preview-slint.py --mobile --platform ios --page settings --size 1194x834
```

资源充足时输出分别位于 `dist/mobile-preview/android/` 与 `dist/mobile-preview/ios/`；资源受限时使用 `--output /tmp/...` 并清理。全量预览涵盖全部页面、深浅色、手机横屏、Pad 双方向和分屏、小屏、125% 字号、未连接、通知进度及详情/编辑/确认场景。宽度均为逻辑尺寸，布局按扣除安全区后的可用宽高决定：

- 宽度不足 640 且非短窗口：底部五个入口，详情进入独立页面；工具页收纳连接、规则、日志和检测入口。
- 中等宽度或高度不足 480：紧凑侧栏，减少横屏头部占用；节点按内容空间改为双列。
- 可用宽度至少 1024、非短窗口：完整侧栏；内容宽度达到 880 时支持列表与详情并排，设置分两列。

Android 完整侧栏与手机采用相同的首页、代理、订阅、工具和设置层次，连接、规则、日志、网络诊断、IP 检测和 A÷检测均从“工具”进入，子页显示返回入口并保持工具选中。设置在完整侧栏底部、版本号上方，窄屏底栏与紧凑侧栏保留五个入口。桌面侧栏保留各独立页面与工具分组。

节点、连接和规则共用分页控件；宽内容区域使用固定触控宽度的翻页箭头和页码，规则保存操作合并到同一行右侧，避免按钮横向铺满 Pad。手机规则页将紧凑分页居中单列，下一行的放弃与应用按钮等宽展开，并与底栏留出间距；大字号同样换行，所有翻页按钮仍至少 48px。列表从顶部排列，少量记录不会垂直居中。

Android 规则页按来源筛选，长按 550ms 后拖动左侧手柄排序，靠近上下边缘会自动滚动，移回中间或松手后停止；普通滑动继续滚动列表，键盘上下键也可排序。桌面与移动端在筛选、分页或列表布局变化时取消拖动，详情均可复制完整规则原文。右箭头进入独立详情与操作页，展示原文（含 `no-resolve` 等参数），保留编辑/复制为自定义、启停、删除、跨页定位与恢复订阅顺序，所有操作先进入草稿。表单与桌面共用规则类型及重复启用规则、位置范围、数量上限校验；失败保留原草稿，逻辑规则继续使用完整文本。网络诊断的响应说明及 DNS 结果可换行显示。Fake-IP 排除项入口紧接 Fake-IP 开关，排除项、IP 结果详情/筛选和 A÷选项使用独立页面，减少主页面按钮。可用 `--mobile --platform android --page rules --detail 2 --theme light --output /tmp/...` 只预览一个表单；详情编号以 `shared/state.slint` 为准，避免运行全量矩阵。

Android 正式窗口已接入系统安全区、键盘、权限、前台 VPN、更新与文件导入；iOS 宿主保留独立占位。不能重复扣除系统已经缩小的视口，状态栏只是预览示意。导航与草稿状态独立于布局，删除、离开未保存的编辑须确认。
