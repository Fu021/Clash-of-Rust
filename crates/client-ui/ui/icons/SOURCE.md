# 外部图标来源

`layout-grid.svg`、`list.svg` 与网络工具导航图标来自 [Lucide](https://github.com/lucide-icons/lucide)，固定提交 `53fab88ff9d1b48f4704b5b5b560d6b4d560bf4c`，按原样保存：

- [宫格](https://github.com/lucide-icons/lucide/blob/53fab88ff9d1b48f4704b5b5b560d6b4d560bf4c/icons/layout-grid.svg)
- [列表](https://github.com/lucide-icons/lucide/blob/53fab88ff9d1b48f4704b5b5b560d6b4d560bf4c/icons/list.svg)
- 网络工具使用同提交 `icons/` 中的 `globe`、`network`、`timer`、`split`、`route`、`shield-check`、`chart-no-axes-combined`、`radio-tower`、`radar`、`waypoints`、`download`；其中 `network.svg` 本地保存为 `network-dns.svg`，避免覆盖现有主导航图标。

以上图标使用 ISC 许可证，完整声明随安装包的 `resources/THIRD-PARTY-NOTICES.txt` 分发；Android 为 assets 中同名文件。Slint 在构建时嵌入 SVG，运行时按主题着色，无在线图标请求。
