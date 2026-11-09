# 配置、检测与维护

[CLI 入口](../cli.md)。下表命令省略 `cor` 前缀，通用参数和退出码见入口文档。

## 配置、诊断与检测

| 命令 | 作用 |
| --- | --- |
| `config get [KEY]` | 读取设置，凭据默认隐藏 |
| `config set KEY VALUE` | 修改单个受支持设置，不覆盖其他并发修改 |
| `config check` | 校验实际合并配置 |
| `config effective [-o FILE]` | 查看运行配置；完整内容需 `--show-secrets` |
| `diagnose [--url URL --host DOMAIN --offline -o FILE] [--all-users]` | 网络检测或离线现场采集；Linux root 用 `--all-users` 只读核对跨用户 TUN 持有进程；[报告与日志边界](runtime.md#故障反馈与诊断报告) |
| `dns DOMAIN [--system]` | 默认内核/代理解析，显式 `--system` 使用系统解析 |
| `ip-check list [--category NAME --filter TEXT]` | 查看内置检测项目与 ID |
| `ip-check [ID] [--category NAME -o FILE]` | 全部、分类或单项测试；最多同时执行 3 项 |
| `claude check [-o FILE]` | Claude 网络/地区线索及风险 |
| `claude audit [--offline -o FILE]` | 检查当前终端和已记录的项目配置；离线模式不启动内核或请求检测网站 |
| `claude browser [--open -o FILE]` | 创建本机真实浏览器采样入口并等待报告 |
| `claude protect [on\|off] [--yes]` | 可逆保护、锁定节点、TUN/DNS/规则覆盖及实时异常流量拦截；开启时关闭已有内核连接 |
| `claude watch [--interval 1]` | 只读观察 Claude 相关请求的真实线路，同时输出共享 `protection_status` 接管状态及拦截记录 |

`config set` 支持 `mixed_port`、`secret`、`dark`、`theme_mode`、`accent_color`、`text_scale`、`tray_speed`、`update_prereleases`、`dns_override`、`fake_ip`、`fake_ip_exclusions`、`fake_ip_disabled_defaults`、`ipv6`、`udp`、`ip_reputation_key`、`delay_interval_minutes`、`subscription_update_interval_minutes`、`node_sort`。值先按 JSON 解析，否则视为字符串，例如 `cor config set ipv6 false`、`cor config set theme_mode dark`。控制端口由后台自动选择并处理冲突，不提供手动设置；混合端口仍可修改。枚举取值见 `config get`；模式、订阅、规则、保护必须使用各自命令，不能绕过保护锁定。

`subscription_update_interval_minutes` 为远程订阅自动更新间隔，默认 360，允许 0–10080，0 关闭；不会更新本地 YAML，失败保留原状态，细则见[订阅自动更新](../subscriptions.md)。

关闭 `dns_override` 会同时关闭 `fake_ip`，运行配置改用 `redir-host`，其他订阅 DNS 字段保留。`fake_ip_exclusions` 是自定义排除项的 JSON 字符串数组，最多 64 项，例如 `cor config set fake_ip_exclusions '["+.example.com"]'`；DNS 覆盖内置兼容项不占自定义额度。`fake_ip_disabled_defaults` 记录被移除的内置项，只接受内置域名，例如 `cor config set fake_ip_disabled_defaults '["+.tailscale.com"]'`；设置为 `[]` 可还原全部内置项，自定义列表保留。这两个数组均保存到设置，重启后继续生效；禁用客户端内置项不会删除订阅自身的同名过滤规则。桌面提供来源标注与一键还原，见[界面说明](../../crates/client-ui/README.md)。

共享运行配置在订阅未指定 `dns.ipv6-timeout` 时，在 A 查询结束后额外等待慢 AAAA 最多 1000 毫秒；显式值不覆盖，原订阅不变。该等待不保证 IPv6 网络或节点可达，排查和验证边界见[IPv6 节点域名解析](../agent-failures/network-recovery.md#ipv6-节点域名解析)。

检测完成的退出码为 0，可用性与风险应读取报告字段，不能只看退出码。检测报告表示真实观察结果；单项检测失败记为 Failed，不将网络失败当作“可用”。浏览器采样需要用户打开本机入口，在没有真实浏览器的 SSH 会话中不会伪造 WebRTC/指纹结果。报告不读取 Claude 账户 Cookie、令牌或登录内容。项目审计使用 CLI 所在的当前工作目录和终端环境，不套用 GUI/后台启动时的环境。

`diagnose` 不会启动后台或恢复 TUN；后台未运行时仍检查系统 DNS、无代理环境变量的系统路由 HTTP，以及 Linux 的示例目标路由。内核运行时再检查其 DNS、本地代理和 IPv4/IPv6 出口。`--host` 和 `--url` 可指定故障服务的域名与 HTTP(S) 地址；返回 401、403 等也只是 HTTP 状态，不表示该服务已可登录或正常使用。报告只列出常见代理环境变量是否存在，不显示其值。两个路径的结果、域名及路由可能不同；故障排查和手动刷新边界见[联网恢复](../agent-failures/network-recovery.md#长时间运行后的服务不可达)。

出口查询请求 JSON 格式，并兼容纯文本 IPv4/IPv6；只有成功 HTTP 状态和有效 IP 才报告出口地址。错误页、缺少或无效 `ip` 字段、超大响应均报告检测失败，不把解析错误当作 IPv4/IPv6 网络不可用的证据。0.6.6 正式版的 CLI 曾请求纯文本接口后按 JSON 解析，可能出现 `trailing characters`；该现象须与真实连接、TLS 或目标服务错误分开排查。

保护开启会强制 TUN、UDP、加密 DNS 及广泛的 Claude 关联规则，可能影响共享服务；关闭按现有保护恢复机制还原此前配置。操作系统授权仍需用户同意。保护状态持久保存，重启后按原有机制恢复；节点、模式和网络覆盖设置在保护期间保持锁定。

## 维护与后台自启

| 命令 | 作用 |
| --- | --- |
| `geo status` / `geo update` | 查看或校验更新 Geo 数据 |
| `update check` | 按已保存的更新通道检查；默认仅正式版 |
| `update download -o FILE` | 下载并校验当前平台安装包，不安装 |
| `update install --yes` | 由运行层下载、校验并交给系统安装，成功交接后退出所有者；有窗口时关闭并在安装后重新打开窗口，无窗口时安装后用 `cor start` 恢复后台 |
| `update status` | 不启动后台，读取本账户和数据目录中的最近安装结果；区分准备、安装中、成功与失败 |
| `autostart [on\|off]` | 普通桌面程序登录自启 |
| `service install --yes` | 为当前用户配置 CLI 后台登录自启 |
| `service remove --yes` / `service status` | 删除或查询 CLI 后台登录自启 |
| `notify [MESSAGE]` | 与 Claude 拦截共用系统通知接口，默认“喵!” |
| `completion bash\|zsh\|fish\|powershell` | 输出上下文补全脚本；命令、选项、`config get/set` 键和常见枚举值由本地同步补全，不启动后台 |

Geo 状态区分数据版本、数据准备时间和本机上次成功更新时间；旧版清单没有本机记录时显示“暂无记录”。

Geo 更新后后台因备份缺失无法启动时，启动恢复仅保留通过整组校验的当前数据，并保存原事务、清单及残余备份；不把此前更新记为成功。未通过校验时保留现场并报错，详见[Geo 恢复与启动](../agent-failures/downloads.md#geo-恢复与启动)。资源缺失报错会列出实际程序及资源查找目录，可核对完整安装位置或 `--resources PATH`，不要直接删除恢复记录。

后台请求超时或中断会取消尚未提交的订阅、Geo 和客户端更新下载；安装交接一旦开始，系统安装程序可能已经启动。慢网可用 `--timeout N` 延长请求等待，详见[下载任务](../agent-failures/downloads.md)。

CLI 后台使用 Linux systemd user unit、macOS LaunchAgent、Windows 当前用户登录任务（无窗口入口启动 `cor`，系统受限时沿用原有注册表回退）。`service install` 配置下次登录启动，当前会话用 `cor start`；`service remove` 只取消自启，停止当前后台用 `cor stop`。不是 Linux 系统级 root 服务，也不自动启用 linger。配置时记录可执行文件和数据目录，移动程序后应重新安装服务。建议桌面自启与 CLI 自启选择一种。卸载或更换安装目录前，对自定义 `--data-dir` 先执行 `cor stop` 和 `cor service remove --yes`；Windows 安装包也会清理默认目录的后台与当前安装的 CLI 自启项。

完整安装更新需要正式安装目录：Linux DEB、Windows 安装程序或 macOS `.app`。便携目录可下载后手动安装。更新前退出其他长时间运行的 CLI 查询进程，避免 Windows 可执行文件被占用。首次系统授权仍可能需要图形会话；无人值守环境可使用 `update download`。

`update install` 返回“安装助手已启动”只说明已交接，不等于安装完成。使用同一账户及原有 `--data-dir` 参数执行 `cor update status` 查看结果；状态保存在数据目录 `update-install.json`，临时安装目录清理后仍可查询。安装失败会给出原因，准备或安装中不冒充成功。诊断报告的 `update_installation` 同步保留该结果。

使用 `cor config set update_prereleases true` 将预发布版纳入检查，默认 `false`。关闭时所有用户都以线上最新正式版为目标，即使本地为 `0.999.0` 等更高版本，也可检查、下载并确认返回正式版；版本相同时不重复提示。开启后才纳入预发布版，且只提供比本地更高的版本。GUI 与 CLI 共用此设置，细则见[更新通道](../workflows/versions.md#版本与更新通道)。

设置保存在当前实例的数据目录 `settings.json`。`--data-dir`、`CLASH_OF_RUST_DATA_DIR`、普通用户默认目录和 root 自有目录是不同实例，不能假设它们共享更新通道或其他设置。桌面自动更新重启时会显式继承原数据目录；若更新后仍看到旧设置，先核对 `cor diagnose --offline --output cor-diagnose.json` 中的 `data_directory`，再比较同一目录的 `settings.json`。

检查更新遇到网络错误或附件暂未完整时，会重新获取发布信息，最多尝试 4 次，总等待不超过约 75 秒；仍缺少当前平台安装包或校验文件时会明确报错。

更新下载显示速度、已下载大小及匹配本次连接的实际节点链，回退直连会明确标注；控制接口无法确认出站时显示未确认，不用首页所选节点代替实际线路。`update download` 的进度写入 stderr，`--quiet` 可关闭。

```sh
cor completion bash > ~/.cor-completion.bash
# 生成 Bash 命令补全脚本并保存到当前用户目录。

source ~/.cor-completion.bash
# 在当前 Bash 终端启用补全；后续可加入 ~/.bashrc。

cor config set theme_mode <Tab>
# 补全 设置键；选定键后继续补全 system/light/dark 等已知值。

cor --json config get dns_override
# 以 JSON 读取 DNS 覆盖开关，便于脚本处理。

cor --jsonl connection list --watch --sort download-rate
# 持续按下载速度从高到低输出连接采样，每行一份 JSON。

cor ip-check --category AI -o ai-report.json
# 检测 AI 分类，完成后将完整报告保存到 ai-report.json。

cor claude audit --offline -o claude-projects.json
# 离线检查终端和已记录项目的 Claude 配置，保存报告，不启动内核或访问检测网站。
```
