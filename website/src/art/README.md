# 插画来源

本网站使用乡间铁路、道岔与蓝猫的统一主题。`raw/` 保留原始 PNG，`../assets/` 中的 WebP 由 `website/prepare_assets.py` 转换；插画只用于介绍，不作为软件截图或性能证据。

2026-11-16 为网站改版使用 OpenAI 图像生成工具制作三张横幅，参考本站已有的配色和题材：

| 原图 | 用途 |
| --- | --- |
| `raw/start.png` | 快速上手：蓝猫从小站出发 |
| `raw/manual.png` | GUI 手册：调度室中的线路手册 |
| `raw/versions.png` | 版本记录：车站更新告示 |

TUI 复用调度室夜景 `terminal`，CLI 复用线路图 `guide`，网络工具与下载沿用 `tools` 和 `station`。手机缩略图来自同一原图；首页手机与桌面保持同一完整构图。

同日使用 OpenAI 图像生成工具，为 `hero`、`start`、`manual`、`tools`、`station`、`guide`、`versions` 制作对应的 `raw/*-night.png`。参考各自日间原图，保留构图、蓝猫、铁路及三色信号灯，改为深青与海军蓝夜空、月光、暖色灯具和村庄灯光，不添加文字或新角色；`terminal` 已是夜景，继续复用。`prepare_assets.py art --art-name hero-night` 等命令生成同名完整与手机 WebP，两者均来自同一夜景原图，随网页主题切换。

原有插画及字体许可证继续保留；不得以新增素材替换第三方来源或许可证记录。
