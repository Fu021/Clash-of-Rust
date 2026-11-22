# 原生 IP 与平台检测

`services.json` 为 182 项检测目录。出口 IP、GitHub 由 `crates/client-core/src/detection/ip/mod.rs` 实现，其余项目由 `crates/client-core/src/detection/regions/platforms.rs` 分派，包含原生平台检测和跳过认证的项目。OpenAI 已于 2026 年 4 月 26 日[停止 Sora 网页和应用服务](https://help.openai.com/en/articles/20001152-what-to-know-about-the-sora-discontinuation)，因此检测目录不再包含 Sora。

检测逻辑移植自 [RegionRestrictionCheck](https://github.com/1-stream/RegionRestrictionCheck)，固定上游提交 `ef841d0a2910a44a6ca565e3a03de0f2282d83b3`。原作者及版权属于上游贡献者。上游派生检测逻辑和保留的机场代码数据沿用 **AGPL-3.0-only**，完整许可证见 [LICENSE](LICENSE)。上游原始参考源码可在上述固定提交中获取，仓库不再保留旧 Shell 实现和迁移生成工具。

原生平台请求强制经过当前 mihomo 混合端口，支持匿名流程中的临时会话、HTTP 响应头判断、请求超时和响应大小限制。跨源跳转只沿用 User-Agent、Accept-Language 和 Accept-Encoding，Cookie 限于原始源，仍需携带请求体时停止跳转。同源跳转沿用原方法和请求头，301/302 的 POST 与 303 按现有规则转为无请求体的 GET。修正上游 EroGameSpace 和 Zee5 中的变量拼写。JSON 格式错误、连接失败和浏览器验证均不当作可用。取消通过丢弃异步请求完成，不创建检测子进程或临时响应文件。

用户主动执行 IP 检测或相应单项重测时，除直接查询出口的项目外，各服务另外通过内核请求目录所列地址的同源 `/cdn-cgi/trace`，沿用协议、域名和端口。只接受成功响应、匹配域名和有效公网地址，不跟随重定向。报告显示同域探针出口，受挑战、站点不提供该接口、超时或字段不符合要求时标为未确认。单独重测后各项测量时间可能不同。站点会看到本次探针请求的出口地址，不查询额外的 IP 属性供应商。

同域出口按 IANA [IPv4 特殊用途表](https://www.iana.org/assignments/iana-ipv4-special-registry)和 [IPv6 特殊用途表](https://www.iana.org/assignments/iana-ipv6-special-registry)做静态范围筛选。IPv4 排除非全球可达、弃用和组播范围，保留 `192.0.0.9` 与 `192.0.0.10` 等登记为全球可达的地址。IPv6 在 `2000::/3` 内排除文档、转换及协议保留范围，保留 `2001::/23` 下登记为全球可达的具体例外。Teredo 和 6to4 在登记表中的全球可达性为 `N/A`，报告将其视为未确认。IPv4 映射地址和 `64:ff9b::/96` 转换地址按内嵌 IPv4 判定。该筛选只验证地址范围，实时路由和前缀分配状态未纳入判定。

旧版的 `cookies.txt` 来自上述固定修订的 `cookies`，沿用上游许可证。当前源码和构建已移除该样本，以及上游 `check.sh` 中 HBO Max、NLZIET 和 Eurosport RO 探针的固定 JWT 与对应认证请求。

以下 31 项跳过平台认证检测。检测目录仅保留原有网址的源，认证可用性显示为「未确认」，耗时显示为「无响应测量」，原始报告标记 `authentication_check_skipped: true`。仍独立记录同域探针出口。

- Disney+、ESPN+、Neon TV、HBO Max、NLZIET、Eurosport RO 使用的 Cookie 样本或固定 JWT 已移除。
- Optus Sports 原请求路径含账号标识，Hulu US 原请求含登录字段和 Cookie。
- Catchplay、TrueID 使用固定 Basic 认证。EPIX、Joyn、Setanta Sports、TVer、TVING、Wavve、DMM 使用固定 API 密钥、令牌或其他认证字段。离线无法确认这些固定值适合分发或发送。
- encoreTVB、Sky DE、Maori TV、SPOTV NOW 使用固定播放策略密钥。它们属于客户端访问字段，其公开分发范围未获离线证实。
- Netflix Preferred CDN、7plus、Naver TV、TV360 分别使用固定访问令牌、签名 URL、签名密钥或用途未确认的访问参数。
- KPlus、ofiii、Now E、ViuTV 的播放或设备配置请求包含固定设备或用户标识，无法离线确认其为匿名标识。
- Channel 5 的固定请求授权参数和 4GTV 的不透明固定 POST 载荷已移除。两项均缺少经确认可安全使用的匿名请求。

内容编号、客户端类型等公开参数沿用匿名检测。Prime Video、Discovery+、Discovery+ UK 和 Telasa 使用每次检测新生成的临时设备标识；Discovery+ 只使用本次响应返回的临时会话值。Instagram Music 移除固定会话及跟踪参数，YouTube Premium 不再发送浏览器 Cookie；这些项目与 Discovery+ 无法识别响应时显示检测失败。Mora 以当次时间生成请求参数，MathsSpot 移除固定跟踪标识。这些请求的真实第三方接口兼容性未在离线回归中验证。

LiTV 和 Paravi 每次检测生成新的匿名标识，保留原有内容编号。U-NEXT 移除固定用户和实验字段，保留播放查询。MEGOGO 移除固定会话字段并生成当次毫秒时间戳。FOX 移除固定播放会话 ID，保留公开内容路径和客户端参数。相关目录地址同步移除固定字段。

FOX 在最终响应为 HTTP `200` 时检查样本。MPEG-TS 只读三个 188 字节包，校验同步字节、传输错误标志及 adaptation field 长度。离线夹具是本仓库自行构造的合成样本，包含带 CRC 的 PAT 和 PMT，以及带 PCR、PTS 的 MPEG-1 Layer I 静音载荷，可随项目再分发。HLS 前缀上限为 8 KiB，触顶时只使用已换行的完整行，并要求媒体或子清单标签后有可解析、无凭据、明确指向资源的 HTTP(S) URI。仅有裸标识符的相对地址，或前缀内找不到完整条目的清单，保留未确认。检测只核验样本文档或分片前缀，不请求清单中的资源。匿名请求与第三方实际返回的分片格式未做实网验证。

跨源跳转仅保留显式设置的 User-Agent、Accept-Language 和 Accept-Encoding，凭据、Cookie 和保留方法的请求体不跨源重发。如果目标依赖其他自定义头（如 AnimeFesta 的 `X-Requested-With`），或要求跨源重发 POST（如 VideoMarket 遇到 `307` 或 `308`），检测结果可能变化。目标域和兼容性尚未经实网确认。

LiTV 和 U-NEXT 按已知服务响应码判断地区。U-NEXT 返回 `475`、Paravi 返回明确的授权要求时，只确认接口可达。MEGOGO 依据接口返回的业务成功码确认可用。空响应、通用拒绝以及 MEGOGO 的 `502` 保留未确认或请求失败。

平台无响应测量时 `millis` 为 `null`，有响应时为向下取整的毫秒数，首个响应不足一毫秒时为数值 `0`。后续步骤失败仍保留已测得的耗时。旧报告中的数值 `0` 保持显示为 `0 ms`，历史哨兵值与实测值只有数值、缺少区分依据。

`cor ip-check` 的 JSON v1 总计和分类 `non_red` 保持历史口径，计入确认可用、部分可用、仅网页可达、信息已识别和结果未确认。CLI 文本、Android 和桌面按实际状态显示计数，信息已识别与结果未确认各自计数。桌面和 Android 分类卡显示该分类的总项目数及各非零状态的数量，跳过认证的项目计入结果未确认。

桌面和 Android 按界面语言显示内置状态摘要、原生地区检测详情前缀、GitHub 的固定 HTTP 详情及 Cloudflare 出口详情。同域出口的末尾片段须与该项域名和出口观测精确匹配，才在英文详情中翻译。详情中的上游原文和 JSON 中的原始证据保留。

运行时不需要 Python、Git、Bash、curl 或 jq，不携带 MSYS、GNU 工具或 OpenSSL 动态库。地区名称使用内置 CLDR 数据，网页可达与账号/API/播放可用分别表示；上游接口可能变化，移植不等于所有服务均已实网验证。

Rust 界面与原创通用代码保留 GPL-3.0-only；上游派生检测逻辑不改标为 GPL-only。组合发行的完整对应源码位于 https://github.com/Fu021/Clash-of-Rust ，应使用与安装包匹配的版本，组合发行遵守 GPLv3/AGPLv3 第 13 条。
