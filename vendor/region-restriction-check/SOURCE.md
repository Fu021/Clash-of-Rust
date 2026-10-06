# RegionRestrictionCheck 适配版本

上游：[1-stream/RegionRestrictionCheck](https://github.com/1-stream/RegionRestrictionCheck)。固定提交：`ef841d0a2910a44a6ca565e3a03de0f2282d83b3`。原作者及版权属于上游贡献者。本目录脚本与数据依据 **AGPL-3.0-only** 使用，完整许可证见 `LICENSE`。

`upstream/check.sh` 保留上游原文件。`check-adapted.sh` 由 `scripts/adapt-region-check.py` 生成，修改于 2026-10-06：保留全部 181 个平台函数；移除 OS 检测、自动安装依赖、自动下载、菜单和组测试入口；离线携带 cookies 与 IATA 数据；用 jq 替代仅用于 JSON 格式化的 Python；强制请求经过当前 mihomo 混合端口；单独记录响应延迟；修正 NBC 单独检测依赖 TLC 的共享变量问题。

Rust 界面及桥接代码保持 GPL-3.0-only。组合发行时遵守 GPLv3/AGPLv3 第 13 条，不将上游代码改标为 GPL。对应源码（含适配脚本、生成器与打包脚本）位于 [Clash-of-Rust 仓库](https://github.com/Fu021/Clash-of-Rust)，请使用与安装包版本匹配的提交。若修改后提供远程网络交互，也须向相应用户提供完整对应源码的获取途径。

适配时还修正了 Location 与 Crackle 响应头的大小写匹配，恢复上游完整浏览器 UA，保留各函数自己的超时时间。HTTP 响应头与退出状态单独用于识别 Cloudflare 验证和连接失败；这类结果显示为未确认，不能推断地区不可用。英文地区名称使用离线 CLDR 数据转换。ChatGPT/Sora 仅因缺少重定向而返回的 No 不足以确认不可用，桥接层保留为未确认。ChatGPT 新版首页直接返回 HTTP 200 时标为网页可达，不能据此推断账号或 API 可用；缺少重定向时也会查询该网站自身的 trace，只有返回的主机名吻合才采用地区。

请求仍经过当前 mihomo 规则；规则或自动策略组选出的出口可能不同于在一台 VPS 上直接执行脚本。上游结果是基于各平台接口的检测判断，不保证登录、订阅或实际播放成功；过期凭据、关闭的服务、接口变化可能导致失败。未知或异常输出不会作为成功处理。
