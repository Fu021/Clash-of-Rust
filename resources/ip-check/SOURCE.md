# 原生 IP 与平台检测

`services.json` 为 183 项检测目录：出口 IP、GitHub 由 `crates/client-core/src/detection/ip/mod.rs` 实现；其余 181 项由 `crates/client-core/src/detection/regions/platforms.rs` 中的原生 Rust 函数实现，后续直接维护 Rust。

检测逻辑移植自 [RegionRestrictionCheck](https://github.com/1-stream/RegionRestrictionCheck)，固定上游提交 `ef841d0a2910a44a6ca565e3a03de0f2282d83b3`。原作者及版权属于上游贡献者；上游派生检测逻辑，以及保留的公开检测 Cookie 与机场代码数据，保留 **AGPL-3.0-only**，完整许可证见 [LICENSE](LICENSE)。上游原始参考源码可在上述固定提交中获取，仓库不再保留旧 Shell 实现和迁移生成工具。

原生适配保留强制经过当前 mihomo 混合端口、多步认证和 Cookie、HTTP 响应头判断、请求超时和响应大小限制；认证头和 Cookie 不跨主机重定向转发。修正上游 7plus、EroGameSpace 和 Zee5 中的变量拼写；JSON 格式错误、连接失败和浏览器验证均不当作可用。取消通过丢弃异步请求完成，不创建检测子进程或临时响应文件。

运行时不需要 Python、Git、Bash、curl 或 jq，不携带 MSYS、GNU 工具或 OpenSSL 动态库。`cookies.txt` 为上游公开检测数据，不读取用户浏览器或账号 Cookie。地区名称使用内置 CLDR 数据，网页可达与账号/API/播放可用分别表示；上游接口和示例凭据可能过期，移植不等于所有服务均已实网验证。

Rust 界面与原创通用代码保留 GPL-3.0-only；上游派生检测逻辑不改标为 GPL-only。组合发行的完整对应源码位于 https://github.com/Fu021/Clash-of-Rust ，应使用与安装包匹配的版本，组合发行遵守 GPLv3/AGPLv3 第 13 条。
