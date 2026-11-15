# 候选网站

`site-next/` 是独立设计候选，尚未替换现有 `website/`，也未接入公开部署。英文位于输出根目录，中文位于 `zh/`；首页、工具箱、使用指南、下载和版本记录共十页。

## 预览与维护

```powershell
python site-next/build.py
python -m http.server 4311 --bind 127.0.0.1 --directory site-next/dist
```

打开 `http://127.0.0.1:4311/`，中文版为 `/zh/`。编辑 `src/`，不要直接编辑 `dist/`。首页的低内存数据保留客户端与内核统计边界；界面导览使用仓库正式组件的演示截图，不是实时录屏或真实网络性能测试。

手机分享可以导出含全部页面、样式、脚本、字体和插画的单文件：

```powershell
python site-next/export_preview.py
```

结果为 `site-next/preview.html`，无需相邻资源；GitHub 实时核对仍需要联网，断网使用明确标注日期的版本快照。单文件支持双语与站内导航，是预览产物，不提交。

## 每次发布都同步

正式版与预发布版均须更新 `src/content/releases.json`。`releases` 提供版本、通道、公开发布日期、英文和中文标题、更新亮点、已知限制；`downloads` 提供两个通道的标签、发布时间和九种安装包的真实字节数；`checked` 为核对日期。数据来自对应 GitHub Release，并与 `docs/releases/` 的说明核对，不把构建日期当发布日期，不将预发布功能写成正式版已支持。

首页仅显示两个当前通道的简短亮点，完整记录在 `releases.html`。版本记录与离线下载快照共用此文件；浏览器仍会核对 GitHub 的最新完整发行。禁止在 JS 或页面中另外维护一份版本快照。历史只覆盖已整理的版本，完整历史链接 GitHub。

发布门槛见[发布工作流](../docs/workflows/release.md#publish-release)。执行 `python site-next/test_build.py`（同时构建）、`node --check site-next/src/site.js`，检查双语版本页、通道、九种包与 SHA256、离线状态、站内导航和手机布局。公开部署须另行授权。

## 内容与设计

- 首页讲清用途、常用功能与版本亮点，避免技术参数、长脚注和逐句硬译；详细限制放工具与指南页，必要的演示标识和功能版本边界保留。
- `src/content/tools.json` 是十一项工具的中英介绍、命令、参数及限制来源，须与 `docs/network-tools.md` 和实际 CLI 核对。HTTP 检查使用已有代理；系统 Ping 与路由追踪不能笼统称为走代理。
- 插画使用乡间铁路、道岔和蓝猫的独立主题；原始图位于 `src/art/raw/`，网页图在 `src/assets/`。插画不是应用截图，截图注明版本和演示数据。
- 字体使用 National Park、Departure Mono 和 Noto Sans SC，许可证随 `src/assets/fonts/` 保留。来源分别为 https://github.com/benhoepner/National-Park 、https://departuremono.com/ 、https://github.com/notofonts/noto-cjk 。中文正文使用系统字体，标题使用真实 Medium 子集，避免伪加粗。
- 修改中文标题、按钮或版本内容后执行 `python site-next/prepare_assets.py fonts` 更新字形子集。源字体放 `.cache/fonts/`，路径见脚本；首次准备需要 Pillow、fontTools 和 brotli，普通静态构建只需 Python 标准库。

`dist/`、`.cache/`、单文件预览和实时接口检查文件均不提交。构建过程不发布、不安装客户端、不修改用户网络。
