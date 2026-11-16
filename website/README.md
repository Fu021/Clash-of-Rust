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

结果为 `website/preview.html`，无需相邻资源；GitHub 实时核对仍需要联网，断网使用明确标注日期的版本快照。单文件支持双语与站内导航，是预览产物，不提交。

右上角主题按钮按“自动 → 日间 → 夜间”循环。自动模式使用设备所在时区的当地时间，07:00～18:59 为日间，其余为夜间，恢复前台、到达边界或当地时间变化后重新判断；不请求定位。手动选择保存在本地并跨页面和标签页同步，禁用存储时仍可在当前页切换。`theme.js` 在样式之前初始化配色，`site.js` 同步按钮与昼夜插画；没有 JavaScript 时沿用系统配色，导航和正文继续可用。

## 每次发布都同步

每次公开正式版或预发布版后执行独立 [loop-website](../docs/loop-website.md)，对照实际发行和权威文档循环修正内容、三类操作入口、双语及手机布局。客户端发布前准备对应说明，公开后再核对实际日期和附件；不重复客户端构建或发布验收。

正式版与预发布版均须更新 `src/content/releases.json`。`series` 指定当前主次版本系列（如 `0.7`），只整理 `0.7.x`；进入新系列时精简旧系列，完整历史始终链接 GitHub。`releases` 提供版本、通道、公开发布日期、双语标题、亮点与必要限制；`downloads` 提供两个通道的标签、发布时间和九种安装包的真实字节数；`checked` 为核对日期。与实际 Release 和 `docs/releases/` 核对，不把构建日期当发布日期，不将预发布或未发布源码能力写成正式版功能。

首页仅显示两个当前通道的简短亮点，完整记录在 `releases.html`。版本记录与离线下载快照共用此文件；浏览器仍会核对 GitHub 的最新完整发行。禁止在 JS 或页面中另外维护一份版本快照。历史只覆盖已整理的版本，完整历史链接 GitHub。

发布门槛见[发布工作流](../docs/workflows/release.md#publish-release)。执行 `python3 -B website/test_build.py`、`node --check website/src/site.js` 和 `node --check website/src/theme.js`，检查双语版本页、通道、九种包与 SHA256、离线状态和站内导航。静态测试在临时目录构建和导出，不覆盖已有 `dist/` 或 `preview.html`；手动构建仍使用开头的命令。

真实浏览器与多尺寸长图检查交给现有 CI 的 `Website validation`，仅在网站或 CI 工作流变化时安装锁定的 Playwright 并运行；不在资源受限主机运行。该任务独立于客户端编译，无私网或发布凭据，不保存浏览器缓存。检查脚本自行构建后，串行核对十六个页面的 360×640 手机、800×1280 平板与 1280×800 桌面布局、首页连续缩放和 80%～200% CSS 缩放，以及中文默认、语言切换、三类手册和旧深链、折叠菜单与目录、图片放大、轮播及暂停、减少动态效果与无 JavaScript 回退。手机启用移动设备与触摸仿真，用触摸操作菜单、目录、截图和轮播。下载测试模拟 GitHub 正常、离线、缺校验文件和非法链接，浏览器不访问外网。每页保留中英文桌面/手机长图，中文另存平板长图，报告为短期 `website-browser-report` 附件；总预算五分钟，失败不自动整轮重跑。设备仿真及 CSS 缩放回归不代替真机与系统缩放验收。`package.json` 和锁文件只服务于测试，构建与 Cloudflare 部署仍只需 Python 标准库。

浏览器回归还覆盖手机版版本记录的当前页标记、两种时区的昼夜边界、手动主题跨页与语言／标签页同步、存储不可用和无脚本深色回退；分别保存日间与夜间截图，检查插画和深浅字标。

## 内容与设计

- 首页讲用途、常用功能和版本亮点，避免长脚注和逐句硬译；删除插画自我说明，必要的截图版本、演示标识和功能边界保留。首页插画使用正常文档流的全宽图片，与其他页面一致；不以绝对定位、尺寸上限或容器查询坐标限制图片宽度。
- 阅读层级与操作入口按 [loop-website](../docs/loop-website.md#阅读层级与操作入口)维护：`start.html` 提供三条首次使用路径，`guide.html`、`tui.html`、`cli.html` 分别为完整手册。CLI 不附在另外两类教程末尾；旧指南 `#gui` / `#tui` / `#cli`、TUI 章节与功能锚点保留可理解的入口。无 JavaScript 时正文、折叠菜单和目录仍可用。已发布与未发布能力分别标注。
- 中英文同步修改入口、章节、命令、参数、默认值、平台限制、版本亮点和配图。静态检查核对章节、教程命令及版本字段，不能代替人工语义审阅。每轮 loop 对照 `docs/` 的权威行为，发现不同步先修复再提交。
- `src/content/tools.json` 是网络工具的中英介绍、命令、参数及限制来源，须与 `docs/network-tools.md` 和实际 CLI 核对。新增工具注明最低可用版本；HTTP 检查使用已有代理，系统 Ping 与路由追踪不能笼统称为走代理。
- 插画使用乡间铁路、道岔和蓝猫主题；原图在 `src/art/raw/`，网页图在 `src/assets/`，新增来源见[插画说明](src/art/README.md)。每个内容页均有插画，图片外框、按钮与卡片采用统一圆角。动效使用短时透明度、位移和颜色过渡，无常驻逐帧任务。
- 首页导览可见时每 3.5 秒切换软件画面，选择画面后暂停，也可手动暂停/继续；悬停、焦点位于导览内、图片放大、切到其他标签页、导览离屏或系统减少动态效果时停用计时。切换预留共同高度，避免桌面与手机画面交替引发整页跳动；无脚本保留第一帧。
- 字标在构建时直接复制 README 的 `resources/icons/wordmark-light.svg`；深色背景使用对应 `wordmark-dark.svg`，不以近似字体替换。首页桌面和手机使用同一张完整插画与比例，不另换手机裁图；代理、直连、拦截三种出口标签均保留，手机紧凑排列在画面上方，渐隐遮罩只作用于插画，不裁切标签。其余横幅使用同源缩略图。手机导航及章节目录可折叠，图片可放大，命令和表格在局部滚动。
- `src/content/screenshots.json` 记录 GUI 截图的真实版本、来源提交、尺寸及平台；`terminal.json` 保留 CI 实际 PTY 帧。每类画面跨页复用，图注显示其真实版本，不随下载快照改标签。目前七张 GUI 画面已按 `0.7.2-fix.4.network-inspection` 源码重新生成，TUI 来自同提交 Memory Benchmark `43735184801`。GUI 使用正式 Slint 组件的演示数据，仅验证外观，不能代替工具实际联网或设备验收；TUI 不手写屏幕内容冒充实测。
- 字体使用 National Park、Departure Mono 和 Noto Sans SC，许可证随 `src/assets/fonts/` 保留。来源分别为 https://github.com/benhoepner/National-Park 、https://departuremono.com/ 、https://github.com/notofonts/noto-cjk 。中文正文使用系统字体，标题使用真实 Medium 子集，避免伪加粗。
- 修改中文标题、按钮或版本内容后检查展示字体的字形覆盖；有新增字形时执行 `python3 website/prepare_assets.py fonts` 更新子集。源字体放 `.cache/fonts/`，路径见脚本；首次准备需要 Pillow、fontTools 和 brotli，普通静态构建只需 Python 标准库。已有字形充分覆盖时保留字体文件，不为文案调整重复下载工具。

更新配图时用 `prepare_assets.py shots --gui-previews DIR --android-preview DIR --terminal-previews DIR --terminal-commit SHA`；GUI 目录包含同尺寸、同主题的首页、代理、订阅、规则、网络和 A÷页面，Android 目录包含应用分流截图，终端目录使用 CI 导出的 `background-tui-{home,network}.json`。保留实际版本与源提交，源码变化不等于这些截图已重新验收；有布局改动须重新生成。

独立更新终端帧用 `prepare_assets.py terminal --terminal-previews DIR --terminal-commit SHA`。新增插画可用 `prepare_assets.py art --art-name start --art-name manual --art-name versions`，避免重复转换已有大图。字体源未保留在本机时，新增中文使用系统字体回退；只有取得原字体源后才更新字形子集，不替换成其他字标。

`dist/`、`.cache/`、单文件预览和实时接口检查文件均不提交。构建过程不发布、不安装客户端、不修改用户网络。

## 部署

[Cloudflare Workers 网站](https://clash-of-rust.fu021.shop/)从仓库构建；沿用仓库根目录、`python3 -B website/build.py` 和 `npx wrangler deploy --name clash-of-rust --assets ./website/dist --compatibility-date 2026-11-13`。推送来源分支可能触发现有自动部署；部署结果以 Cloudflare 为准。

构建复制 `_headers`、`_redirects`、`robots.txt` 和 `sitemap.xml`，canonical 与双语 alternate 均使用正式域名。旧 `/features` 跳至中文指南，`/zh/*` 跳至对应中文根路径，`/download` 继续有效。间距样式转换为外部规则以兼容 CSP；搜索引擎收录不作保证。
