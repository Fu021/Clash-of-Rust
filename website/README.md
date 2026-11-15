# 产品网站

`website/` 是独立静态产品网站，正式地址为 https://clash-of-rust.fu021.shop/ 。默认中文，英文位于 `en/`，通过语言按钮切换；不按浏览器语言或旧偏好自动跳转。首页、工具箱、使用指南、下载和版本记录共十页，不连接本机控制器，也不改变代理或 TUN。

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

## 每次发布都同步

正式版与预发布版均须更新 `src/content/releases.json`。`series` 指定当前主次版本系列（如 `0.7`），只整理 `0.7.x`；进入新系列时精简旧系列，完整历史始终链接 GitHub。`releases` 提供版本、通道、公开发布日期、双语标题、亮点与必要限制；`downloads` 提供两个通道的标签、发布时间和九种安装包的真实字节数；`checked` 为核对日期。与实际 Release 和 `docs/releases/` 核对，不把构建日期当发布日期，不将预发布或未发布源码能力写成正式版功能。

首页仅显示两个当前通道的简短亮点，完整记录在 `releases.html`。版本记录与离线下载快照共用此文件；浏览器仍会核对 GitHub 的最新完整发行。禁止在 JS 或页面中另外维护一份版本快照。历史只覆盖已整理的版本，完整历史链接 GitHub。

发布门槛见[发布工作流](../docs/workflows/release.md#publish-release)。执行 `python3 -B website/test_build.py`（同时构建）、`node --check website/src/site.js`，检查双语版本页、通道、九种包与 SHA256、离线状态、站内导航和手机布局。

真实浏览器与多尺寸长图检查交给现有 CI 的 `Website validation`，仅在网站或 CI 工作流变化时安装锁定的 Playwright 并运行；不在资源受限主机运行。该任务独立于客户端编译，无私网或发布凭据，不保存浏览器缓存。它串行核对十个双语页面的桌面/手机布局、首页连续缩放和 80%～200% CSS 缩放、中文默认及英文切换、GUI/TUI 教程和深链、下载通道、减少动态效果与无 JavaScript 回退。浏览器禁止外网请求，长图与报告保存为短期 `website-browser-report` 附件；总预算五分钟，失败不自动整轮重跑。CSS 缩放回归不代替真实浏览器窗口、系统缩放及 GPU 组合验收。`package.json` 和锁文件只服务于测试，构建与 Cloudflare 部署仍只需 Python 标准库。

## 内容与设计

- 首页讲用途、常用功能和版本亮点，避免长脚注和逐句硬译；删除插画自我说明，必要的截图版本、演示标识和功能边界保留。首页插画使用正常文档流的全宽图片，与其他页面一致；不以绝对定位、尺寸上限或容器查询坐标限制图片宽度。
- 使用指南分为 GUI 和 TUI，可通过按钮或 `#gui` / `#tui` 直达，CLI 示例放在对应操作旁。无 JavaScript 时两套教程仍可阅读。已发布 TUI 与源码新增页分别标注，发布时同步修订；不把原型页面数永久写死。
- 中英文同步修改入口、章节、命令、参数、默认值、平台限制、版本亮点和配图。静态检查核对章节、教程命令及版本字段，不能代替人工语义审阅。每轮 loop 对照 `docs/` 的权威行为，发现不同步先修复再提交。
- `src/content/tools.json` 是网络工具的中英介绍、命令、参数及限制来源，须与 `docs/network-tools.md` 和实际 CLI 核对。新增工具注明最低可用版本；HTTP 检查使用已有代理，系统 Ping 与路由追踪不能笼统称为走代理。
- 插画使用乡间铁路、道岔和蓝猫主题；原图在 `src/art/raw/`，网页图在 `src/assets/`。图片外框、按钮、表单与卡片使用统一圆角。动效只使用短时透明度、位移与颜色过渡，尊重 `prefers-reduced-motion`，无自动播放或常驻逐帧任务。
- `src/content/screenshots.json` 记录当前 GUI 截图版本、来源提交、尺寸及平台；`terminal.json` 保留同版本 CI 实际 PTY 帧。首页、工具与指南共用，禁止混用旧版首页或用插画冒充软件。源图片由正式 Slint 组件串行预览生成，TUI 文本按实际终端帧绘制，不能手写屏幕内容冒充实测；测试输入不是用户订阅。
- 字体使用 National Park、Departure Mono 和 Noto Sans SC，许可证随 `src/assets/fonts/` 保留。来源分别为 https://github.com/benhoepner/National-Park 、https://departuremono.com/ 、https://github.com/notofonts/noto-cjk 。中文正文使用系统字体，标题使用真实 Medium 子集，避免伪加粗。
- 修改中文标题、按钮或版本内容后检查展示字体的字形覆盖；有新增字形时执行 `python3 website/prepare_assets.py fonts` 更新子集。源字体放 `.cache/fonts/`，路径见脚本；首次准备需要 Pillow、fontTools 和 brotli，普通静态构建只需 Python 标准库。已有字形充分覆盖时保留字体文件，不为文案调整重复下载工具。

更新配图时用 `prepare_assets.py shots --gui-previews DIR --android-preview DIR --terminal-previews DIR --terminal-commit SHA`；GUI 目录包含同尺寸、同主题的首页、代理、订阅、规则、网络和 A÷页面，Android 目录包含应用分流截图，终端目录使用 CI 导出的 `background-tui-{home,network}.json`。保留实际版本与源提交，源码变化不等于这些截图已重新验收；有布局改动须重新生成。

`dist/`、`.cache/`、单文件预览和实时接口检查文件均不提交。构建过程不发布、不安装客户端、不修改用户网络。

## 部署

[Cloudflare Workers 网站](https://clash-of-rust.fu021.shop/)从仓库构建；沿用仓库根目录、`python3 -B website/build.py` 和 `npx wrangler deploy --name clash-of-rust --assets ./website/dist --compatibility-date 2026-11-13`。推送来源分支可能触发现有自动部署；部署结果以 Cloudflare 为准。

构建复制 `_headers`、`_redirects`、`robots.txt` 和 `sitemap.xml`，canonical 与双语 alternate 均使用正式域名。旧 `/features` 跳至中文指南，`/zh/*` 跳至对应中文根路径，`/download` 继续有效。间距样式转换为外部规则以兼容 CSP；搜索引擎收录不作保证。
