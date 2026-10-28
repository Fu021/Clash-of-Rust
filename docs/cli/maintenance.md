# 配置、检测与维护

[CLI 入口](../cli.md)。下表命令省略 `cor` 前缀，通用参数和退出码见入口文档。

## 配置、诊断与检测

| 命令 | 作用 |
| --- | --- |
| `config get [KEY]` | 读取设置，凭据默认隐藏 |
| `config set KEY VALUE` | 修改单个受支持设置，不覆盖其他并发修改 |
| `config check` | 校验实际合并配置 |
| `config effective [-o FILE]` | 查看运行配置；完整内容需 `--show-secrets` |
| `diagnose [--url URL --host DOMAIN]` | HTTP、IPv4/IPv6 出口、系统与内核 DNS |
| `dns DOMAIN [--system]` | 默认内核/代理解析，显式 `--system` 使用系统解析 |
| `ip-check list [--category NAME --filter TEXT]` | 查看内置检测项目与 ID |
| `ip-check [ID] [--category NAME -o FILE]` | 全部、分类或单项测试；最多同时执行 3 项 |
| `claude check [-o FILE]` | Claude 网络/地区线索及风险 |
| `claude audit [--offline -o FILE]` | 检查当前终端和已记录的项目配置；离线模式不启动内核或请求检测网站 |
| `claude browser [--open -o FILE]` | 创建本机真实浏览器采样入口并等待报告 |
| `claude protect [on\|off] [--yes]` | 可逆保护、锁定节点、TUN/DNS/规则覆盖及实时异常流量拦截；开启时关闭已有内核连接 |
| `claude watch [--interval 1]` | 只读观察 Claude 相关请求的真实线路；保护开启时同时查看拦截状态 |

`config set` 支持 `mixed_port`、`secret`、`dark`、`theme_mode`、`accent_color`、`text_scale`、`tray_speed`、`update_prereleases`、`dns_override`、`fake_ip`、`fake_ip_exclusions`、`fake_ip_disabled_defaults`、`ipv6`、`udp`、`ip_reputation_key`、`delay_interval_minutes`、`node_sort`。值先按 JSON 解析，否则视为字符串，例如 `cor config set ipv6 false`、`cor config set theme_mode dark`。控制端口由后台自动选择并处理冲突，不提供手动设置；混合端口仍可修改。枚举取值见 `config get`；模式、订阅、规则、保护必须使用各自命令，不能绕过保护锁定。

关闭 `dns_override` 会同时关闭 `fake_ip`，运行配置改用 `redir-host`，其他订阅 DNS 字段保留。`fake_ip_exclusions` 是自定义排除项的 JSON 字符串数组，最多 64 项，例如 `cor config set fake_ip_exclusions '["+.example.com"]'`；DNS 覆盖内置兼容项不占自定义额度。`fake_ip_disabled_defaults` 记录被移除的内置项，只接受内置域名，例如 `cor config set fake_ip_disabled_defaults '["+.tailscale.com"]'`；设置为 `[]` 可还原全部内置项，自定义列表保留。这两个数组均保存到设置，重启后继续生效；禁用客户端内置项不会删除订阅自身的同名过滤规则。桌面提供来源标注与一键还原，见[界面说明](../../crates/client-ui/README.md)。

检测完成的退出码为 0，可用性与风险应读取报告字段，不能只看退出码。检测报告表示真实观察结果；单项检测失败记为 Failed，不将网络失败当作“可用”。浏览器采样需要用户打开本机入口，在没有真实浏览器的 SSH 会话中不会伪造 WebRTC/指纹结果。报告不读取 Claude 账户 Cookie、令牌或登录内容。项目审计使用 CLI 所在的当前工作目录和终端环境，不套用 GUI/后台启动时的环境。

保护开启会强制 TUN、UDP、加密 DNS 及广泛的 Claude 关联规则，可能影响共享服务；关闭按现有保护恢复机制还原此前配置。操作系统授权仍需用户同意。保护状态持久保存，重启后按原有机制恢复；节点、模式和网络覆盖设置在保护期间保持锁定。

## 维护与后台自启

| 命令 | 作用 |
| --- | --- |
| `geo status` / `geo update` | 查看或校验更新 Geo 数据 |
| `update check` | 按已保存的更新通道检查；默认仅正式版 |
| `update download -o FILE` | 下载并校验当前平台安装包，不安装 |
| `update install --yes` | 由运行层下载、校验并交给系统安装，成功交接后退出所有者；有窗口时关闭并在安装后重新打开窗口，无窗口时安装后用 `cor start` 恢复后台 |
| `autostart [on\|off]` | 普通桌面程序登录自启 |
| `service install --yes` | 为当前用户配置 CLI 后台登录自启 |
| `service remove --yes` / `service status` | 删除或查询 CLI 后台登录自启 |
| `notify [MESSAGE]` | 与 Claude 拦截共用系统通知接口，默认“喵!” |
| `completion bash\|zsh\|fish\|powershell` | 输出 shell 补全脚本 |

CLI 后台使用 Linux systemd user unit、macOS LaunchAgent、Windows 当前用户登录任务（无窗口入口启动 `cor`，系统受限时沿用原有注册表回退）。`service install` 配置下次登录启动，当前会话用 `cor start`；`service remove` 只取消自启，停止当前后台用 `cor stop`。不是 Linux 系统级 root 服务，也不自动启用 linger。配置时记录可执行文件和数据目录，移动程序后应重新安装服务。建议桌面自启与 CLI 自启选择一种。卸载或更换安装目录前，对自定义 `--data-dir` 先执行 `cor stop` 和 `cor service remove --yes`；Windows 安装包也会清理默认目录的后台与当前安装的 CLI 自启项。

完整安装更新需要正式安装目录：Linux DEB、Windows 安装程序或 macOS `.app`。便携目录可下载后手动安装。更新前退出其他长时间运行的 CLI 查询进程，避免 Windows 可执行文件被占用。首次系统授权仍可能需要图形会话；无人值守环境可使用 `update download`。

使用 `cor config set update_prereleases true` 将预发布版纳入检查，默认 `false`。关闭时所有用户都以线上最新正式版为目标，即使本地为 `0.999.0` 等更高版本，也可检查、下载并确认返回正式版；版本相同时不重复提示。开启后才纳入预发布版，且只提供比本地更高的版本。GUI 与 CLI 共用此设置，细则见[更新通道](../workflows/versions.md#版本与更新通道)。

检查更新遇到网络错误或附件暂未完整时，会重新获取发布信息，最多尝试 4 次，总等待不超过约 75 秒；仍缺少当前平台安装包或校验文件时会明确报错。

更新下载显示速度、已下载大小及匹配本次连接的实际节点链，回退直连会明确标注；控制接口无法确认出站时显示未确认，不用首页所选节点代替实际线路。`update download` 的进度写入 stderr，`--quiet` 可关闭。

```sh
cor completion bash > ~/.cor-completion.bash
# 生成 Bash 命令补全脚本并保存到当前用户目录。

source ~/.cor-completion.bash
# 在当前 Bash 终端启用补全；后续可加入 ~/.bashrc。

cor --json config get dns_override
# 以 JSON 读取 DNS 覆盖开关，便于脚本处理。

cor --jsonl connection list --watch --sort download-rate
# 持续按下载速度从高到低输出连接采样，每行一份 JSON。

cor ip-check --category AI -o ai-report.json
# 检测 AI 分类，完成后将完整报告保存到 ai-report.json。

cor claude audit --offline -o claude-projects.json
# 离线检查终端和已记录项目的 Claude 配置，保存报告，不启动内核或访问检测网站。
```
