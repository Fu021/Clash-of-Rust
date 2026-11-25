# 产品网站

`website/` 是独立静态产品网站，正式地址为 https://clash-of-rust.fu021.shop/ 。默认中文，英文位于 `en/`，通过语言按钮切换；不按浏览器语言或旧偏好自动跳转。首页、快速上手、GUI/TUI/CLI 三本手册、网络工具、下载与版本记录共八种页面，双语十六页，不连接本机控制器，也不改变代理或 TUN。

## 预览与维护

```powershell
python3 -B website/build.py
python3 -m http.server 4311 --bind 127.0.0.1 --directory website/dist
```

打开 `http://127.0.0.1:4311/`，英文版为 `/en/`。编辑 `src/`，不要直接编辑 `dist/`。旧 `/zh/` 链接通过部署重定向保持可用。首页内存数据保留客户端与内核统计边界，不把演示图片作为性能证据。

手机分享可以导出含全部页面、样式、脚本、字体和插画的单文件：

```powershell
python3 -B website/export_preview.py
```

结果为 `website/preview.html`，无需相邻资源；实时版本查询仍需要联网，断网保留正式版下载列表。单文件支持双语与站内导航，是预览产物，不提交。

右上角主题按钮按“自动 → 日间 → 夜间”循环。自动模式使用设备所在时区的当地时间，07:00～18:59 为日间，其余为夜间，恢复前台、到达边界或当地时间变化后重新判断；不请求定位。手动选择保存在本地并跨页面和标签页同步，禁用存储时仍可在当前页切换。`theme.js` 在样式之前初始化配色，`site.js` 同步按钮与昼夜插画；插画解码后短暂淡入，等待上限为 1.2 秒，不遮挡正文。三份字体预加载，慢网下允许保留系统字体，避免加载后再次换字重排。没有 JavaScript 时沿用系统配色，导航和正文继续可用。

## 每次发布都同步

每次公开正式版后执行独立 [loop-website](../docs/loop-website.md)，对照实际发行和权威文档循环修正内容、三类操作入口、双语及手机布局。客户端发布前准备对应说明，公开后再核对实际日期和附件；不重复客户端构建或发布验收。网站内容、版本记录和下载入口仅提供正式版，预发布不单独更新网站。

正式版更新 `src/content/releases.json`。`series` 指定当前主次版本系列（如 `0.7`），`releases` 提供该系列正式版的版本、公开发布日期、双语标题、亮点与必要限制，`history` 保留更早正式版的版本、日期、双语 `summary` 和 GitHub 入口；摘要依据原始 Release，未记录的变更明确说明，不猜测补齐。进入新系列时将旧系列移入历史，不能丢失旧正式版。`downloads.stable` 提供正式版标签、发布时间、九种标准安装包及本版已有 RPM 的真实字节数。`checked` 为核对日期。与实际 Release 和 `docs/releases/` 核对，并以该正式版的安装器和命令行为校对 PATH 与 TUI 启动步骤。

首页显示最近两个正式版的简短亮点，完整记录在 `releases.html`。版本记录与离线下载快照共用发布数据文件。下载页通过 GitHub `/releases/latest` 查询正式版，要求九种标准安装包及各自 SHA256 均有效。可选 RPM 仅在包和摘要都存在时展示，其他附件可从 GitHub Releases 查看。正常加载时隐藏查询状态，断网或标准附件不完整时沿用正式版快照，并提供正式版入口。旧 `#preview` 链接也显示正式版。版本快照统一保存在 `src/content/releases.json`。

发布门槛见[发布工作流](../docs/workflows/release.md#publish-release)。执行 `python3 -B website/test_build.py`、`node --check website/src/site.js` 和 `node --check website/src/theme.js`，检查双语版本页与历史摘要、正式版下载、九种包与 SHA256、离线状态和站内导航。静态测试在临时目录构建和导出，不覆盖已有 `dist/` 或 `preview.html`；手动构建仍使用开头的命令。

真实浏览器与多尺寸长图检查交给现有 CI 的 `Website validation`，仅在网站或 CI 工作流变化时安装锁定的 Playwright 并运行；不在资源受限主机运行。该任务独立于客户端编译，使用只读权限，浏览器缓存仅存本次 runner。检查脚本自行构建后，串行核对十六个页面的 360×640 手机、800×1280 平板与 1280×800 桌面布局、首页连续缩放和 80%～200% CSS 缩放，以及中文默认、语言切换、三类手册和旧深链、折叠菜单与目录、图片放大、轮播及暂停、减少动态效果与无 JavaScript 回退。手机启用移动设备与触摸仿真，用触摸操作菜单、目录、截图和轮播。下载测试模拟 GitHub 正常、离线、缺校验文件和非法链接，浏览器不访问外网。每页保留中英文桌面/手机长图，中文另存平板长图，报告为短期 `website-browser-report` 附件；总预算五分钟，失败不自动整轮重跑。设备仿真及 CSS 缩放回归不代替真机与系统缩放验收。`package.json` 和锁文件只服务于测试，构建与 Cloudflare 部署仍只需 Python 标准库。

浏览器回归还覆盖手机版版本记录的当前页标记、两种时区的昼夜边界、手动主题跨页与语言／标签页同步、存储不可用和无脚本深色回退；分别保存日间与夜间截图，检查插画和深浅字标。

页脚访客数统计功能启用以来的独立浏览器。同一站点的页面、语言和标签页共用 IndexedDB 中的随机标识。Worker 只保存该标识的摘要，并用 SQLite 唯一约束去重。换浏览器、换设备或清除站点数据会重新计数。浏览器存储不可用时只查询总数，接口离线、静态预览或禁用 JavaScript 时隐藏计数。测试同一浏览器同时打开中英文页面、刷新和接口失败。需要连同计数接口预览时，先构建网站，再在仓库根目录运行 `npx wrangler dev`，使用本地的计数存储。

首页内存区保留底部留白，使用与终端区相同的像素过渡连接下一节。“测试条件与来源”和手机菜单共用展开、收起动效，支持键盘、连续切换及动态启用减少动态效果；无脚本时保留原生折叠操作。浏览器回归另核对中英文电脑/手机的间距、展开/收起及无脚本回退，下载回退覆盖接口离线、缺少校验、非法链接、草稿及预发布响应；旧 `#preview` 不恢复通道切换。

## 内容与设计

- 首页讲用途、常用功能和版本亮点，避免长脚注和逐句硬译；不展示“正在核对”“已核对”等内部查询进度，不在正文解释维护流程、配图策略或重复版本号。必要的截图版本、演示标识和功能边界保留，技术细节放手册或链接。首页插画使用正常文档流的全宽图片，与其他页面一致；不以绝对定位、尺寸上限或容器查询坐标限制图片宽度。
- 阅读层级与操作入口按 [loop-website](../docs/loop-website.md#阅读层级与操作入口)维护：`start.html` 提供三条首次使用路径，`guide.html`、`tui.html`、`cli.html` 分别为完整手册。CLI 不附在另外两类教程末尾；旧指南 `#gui` / `#tui` / `#cli`、TUI 章节与功能锚点保留可理解的入口。无 JavaScript 时正文、折叠菜单和目录仍可用。已发布与未发布能力分别标注。
- 中英文同步修改入口、章节、命令、参数、默认值、平台限制、版本亮点和配图。静态检查核对章节、教程命令及版本字段，不能代替人工语义审阅。每轮 loop 对照 `docs/` 的权威行为，发现不同步先修复再提交。
- `src/content/tools.json` 是网络工具的中英介绍、命令、参数及限制来源，须与 `docs/network-tools.md` 和实际 CLI 核对。新增工具注明最低可用版本；HTTP 检查使用已有代理，系统 Ping 与路由追踪不能笼统称为走代理。
- 插画使用乡间铁路、道岔和蓝猫主题；原图在 `src/art/raw/`，网页图在 `src/assets/`，新增来源见[插画说明](src/art/README.md)。每个内容页均有插画，图片外框、按钮与卡片采用统一圆角。动效使用短时透明度、位移和颜色过渡，无常驻逐帧任务。
- 首页导览可见时每 2.6 秒切换桌面软件画面，选择画面后暂停，也可手动暂停/继续；焦点位于导览内、图片放大、切到其他标签页、导览离屏或系统减少动态效果时停用计时，悬停不暂停。切换预留共同高度，首页不混入手机竖屏图；无脚本保留第一帧。CLI 与 TUI 展示紧接导览所在区块，后接功能列表；终端内容按宽度换行，不要求横向拖拉。
- 字标在构建时直接复制 README 的 `resources/icons/wordmark-light.svg`；深色背景使用对应 `wordmark-dark.svg`，不以近似字体替换。首页桌面和手机使用同一张完整插画与比例，不另换手机裁图；代理、直连、拦截三种出口标签均保留，手机紧凑排列在画面上方，渐隐遮罩只作用于插画，不裁切标签。其余横幅使用同源缩略图。手机导航及章节目录可折叠，图片可放大，命令和表格在局部滚动。
- `src/content/screenshots.json` 记录 GUI 截图的真实版本、来源提交、尺寸及平台；`terminal.json` 保留实际 PTY 帧。每类画面跨页复用，图注显示其真实版本，不随下载快照改标签。当前 GUI 和 TUI 画面以正式版 `0.7.2`、提交 `141e1cb` 为来源；GUI 使用正式 Slint 组件的演示数据，TUI 使用实际 `cor ui --demo` 捕获。两者只验证展示，不代替联网或设备验收；不手写屏幕内容冒充实测。
- 字体使用 National Park、Departure Mono 和 Noto Sans SC，许可证随 `src/assets/fonts/` 保留。来源分别为 https://github.com/benhoepner/National-Park 、https://departuremono.com/ 、https://github.com/notofonts/noto-cjk 。中文正文使用系统字体，标题使用真实 Medium 子集，避免伪加粗。
- 修改中文标题、按钮或版本内容后检查展示字体的字形覆盖；有新增字形时执行 `python3 website/prepare_assets.py fonts` 更新子集。源字体放 `.cache/fonts/`，路径见脚本；首次准备需要 Pillow、fontTools 和 brotli，普通静态构建只需 Python 标准库。已有字形充分覆盖时保留字体文件，不为文案调整重复下载工具。

更新配图时用 `prepare_assets.py shots --gui-previews DIR --android-preview DIR --terminal-previews DIR --terminal-commit SHA`；GUI 目录包含同尺寸、同主题的首页、代理、订阅、规则、网络和 A÷页面，Android 目录包含应用分流截图，终端目录使用 CI 导出的 `background-tui-{home,network}.json`。保留实际版本与源提交，源码变化不等于这些截图已重新验收；有布局改动须重新生成。

独立更新终端帧用 `prepare_assets.py terminal --terminal-previews DIR --terminal-commit SHA`。新增插画可用 `prepare_assets.py art --art-name start --art-name manual --art-name versions`，避免重复转换已有大图。字体源未保留在本机时，新增中文使用系统字体回退；只有取得原字体源后才更新字形子集，不替换成其他字标。

`dist/`、`.cache/`、单文件预览和实时接口检查文件均不提交。构建过程不发布、不安装客户端、不修改用户网络。

## 部署

[Cloudflare Workers 网站](https://clash-of-rust.fu021.shop/)从仓库构建。仓库根目录执行 `python3 -B website/build.py`，再运行 `npx wrangler deploy`。现有显式传入 Worker 名称、静态目录和兼容日期的命令也使用根目录的 `wrangler.toml`，由首次迁移创建 SQLite Durable Object。推送来源分支可能触发现有自动部署，部署后可查询 `/api/visitors` 检查计数接口。

为避免无关客户端提交触发网站构建，在 Workers 项目的 **Settings → Build → Build watch paths** 配置 Include paths，加入 `website/*`、`wrangler.toml`、`resources/icons/wordmark-light.svg`、`resources/icons/wordmark-dark.svg`，Exclude paths 留空。这覆盖网站源码、构建脚本、部署配置和共享字标。该配置在 Cloudflare 账户中保存；保存后分别用网站输入变化和无关文件变化验证。按 [Cloudflare 官方说明](https://developers.cloudflare.com/workers/ci-cd/builds/build-watch-paths/)，空 push、至少 20 个提交或至少 3000 个文件变化仍会直接构建。

构建复制 `_headers`、`_redirects`、`robots.txt` 和 `sitemap.xml`，canonical 与双语 alternate 均使用正式域名。旧 `/features` 跳至中文指南，`/zh/*` 跳至对应中文根路径，`/download` 继续有效。间距样式转换为外部规则以兼容 CSP；搜索引擎收录不作保证。
