# RegionRestrictionCheck 原生适配

上游：https://github.com/1-stream/RegionRestrictionCheck 。固定提交：`ef841d0a2910a44a6ca565e3a03de0f2282d83b3`。原作者及版权属于上游贡献者，原始检测代码和派生检测逻辑保留 **AGPL-3.0-only**，完整许可证见本目录 `LICENSE`。

0.4.4 将全部 181 个平台函数的请求参数、条件分支和结果提取逻辑迁移为 `src/region_check/generated.rs` 中的原生 Rust 函数。开发工具 `scripts/native-detector-codegen.py` 使用固定的 `check-adapted.sh` 作为迁移输入；两个 Bash 文件仅保留为源码参考，不在运行时解析或执行，也不进入安装资源。应用直接使用 reqwest、serde_json、Rust 文本处理及 HMAC-SHA1 实现检测，不携带 Bash、MSYS、curl、jq、GNU 工具或 OpenSSL 动态库。

适配保留强制经过当前 mihomo 混合端口、多步认证和 Cookie、HTTP 响应头判断、请求超时和响应大小限制。取消通过丢弃异步请求完成，不创建检测子进程或临时响应文件。修正上游 7plus、EroGameSpace 和 Zee5 中的变量拼写；认证头和 Cookie 不跨主机重定向转发。JSON 格式错误、连接失败和浏览器验证均不当作可用。

两个基础检测（出口 IP、GitHub）仍为项目自己的原生实现。地区名称使用内置 CLDR 数据，浏览器页面可达与账号/API/播放可用分别表示；目录中的 URL 不能单独作为平台解锁判定依据。上游接口和示例凭据可能过期，迁移不等于所有公开服务已实网验证。

Rust 界面与原创通用代码保留 GPL-3.0-only；上游派生检测逻辑不改标为 GPL-only。组合发行的完整对应源码位于 https://github.com/Fu021/Clash-of-Rust ，应使用与安装包匹配的版本。
