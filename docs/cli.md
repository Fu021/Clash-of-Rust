# CLI 使用说明

`cor` 是 Clash of Rust 的无界面命令行客户端，适用于 Windows、Linux 和 macOS。它复用桌面端的内核管理、配置事务、规则、检测和系统集成，不依赖 Slint、托盘或显示服务器。

## 安装与快速开始

桌面安装包包含 `cor`，无需单独下载内核与 Geo 数据。

| 系统 | 命令位置 |
| --- | --- |
| Ubuntu 22.04+/Debian | 安装 DEB 后直接运行 `cor`（`/usr/bin/cor`） |
| Windows | 安装目录中的 `cor.exe`，例如 PowerShell：`& 'C:\Program Files\Clash of Rust\cor.exe' help`；按实际安装目录调整，或自行加入 PATH |
| macOS | `/Applications/Clash of Rust.app/Contents/MacOS/cor`；也可自行为此文件建立 PATH 中的符号链接 |

```sh
cor help
cor help profile add
cor profile add 我的订阅 'https://example.com/subscription'
cor profile list
cor profile use 我的订阅
cor start
cor node list
cor node list Proxy --sort latency
cor node select Proxy 节点名称
cor proxy system
cor status
cor stop
```

节点和策略组名称以实际订阅为准；带空格的名称应使用引号。订阅可用完整 ID 或唯一名称定位，重名时须使用 ID。CLI 首次启动沿用内置“默认直连”。

GUI 已运行时，CLI 通过带随机密钥的本机接口操作同一内核。GUI 未运行时，需要后台的命令会自动启动 `cor run --no-core`；查询、修改配置不会自行开启系统代理。需要网络的命令会启动内核并恢复已保存的代理模式。`status`、`stop`、`help`、`version`、补全生成不会启动后台。

每个数据目录只有一个运行层所有者；随后打开的 GUI 会接入该后台。退出 GUI 会停止内核及其独立后台。`cor stop` 会停止内核、恢复网络；没有关联窗口时独立后台同时退出；有关联窗口时保留后台，窗口可直接重新启动内核。

## 帮助、脚本与输出

所有命令都支持 `--help`；`cor help profile` 列出该组命令。帮助与解析、补全来自同一份命令表。

| 通用选项 | 作用 |
| --- | --- |
| `--json` | 单次机器可读输出 |
| `--jsonl` | 逐行 JSON，适合实时状态、日志、连接和检测进度 |
| `--timeout N` | 单次后台请求或浏览器等待的超时秒数，默认 300，最大 3600 |
| `--data-dir PATH` | 独立数据目录；也可使用 `CLASH_OF_RUST_DATA_DIR` |
| `--resources PATH` | 显式指定含 mihomo、默认配置及 Geo 清单的资源目录 |
| `--yes` / `-y` | 明确确认删除、保护、安装等操作；不代替操作系统授权 |
| `--show-secrets` | 允许输出完整订阅/配置凭据；默认隐藏结构化输出的密钥字段和订阅链接凭据 |
| `--quiet` / `-q` | 减少进度提示，结果依旧输出 |
| `--no-color` | 纯文本输出；CLI 默认不输出 ANSI 颜色 |

JSON 每行使用 `{"schema":1,"command":"命令","data":...}` 外层。`--json` 和 `--jsonl` 二选一；持续命令可输出多份采样，推荐使用 `--jsonl`。进度、授权提示和错误写入 stderr，结果写入 stdout。导出文件原子替换，Unix 下权限为 0600。日志内容是原始诊断文本，可能包含域名、IP 或第三方服务返回的信息，分享前请检查。

`--` 后的内容作为原样位置参数；例如名称以 `-` 开头时使用它。`Ctrl+C` 结束持续查询；若后台已收到修改请求，修改可能继续完成，请重新查询状态。`cor run` 收到 Ctrl+C/SIGTERM 后先恢复网络，再关闭与该后台关联的窗口并退出。

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功，或下游管道已关闭 |
| 1 | 操作/校验/权限失败 |
| 2 | 命令或参数格式错误 |
| 3 | 后台启动失败或不可达 |
| 124 | 请求超时 |
| 130 | 查询被中断 |

## 命令一览

### 运行与代理

| 命令 | 作用 |
| --- | --- |
| `run [--no-core]` | 前台运行；默认启动内核，`--no-core` 仅创建控制后台 |
| `start` / `stop` / `restart` | 启动、停止、重启当前内核 |
| `status [--watch --interval 1]` | 当前版本、订阅、节点、模式、连接数及总速率 |
| `version` | CLI 版本、操作系统和 CPU 架构 |
| `mode [rule\|global\|direct]` | 查询或切换内核运行模式 |
| `proxy [system\|tun\|off]` | 查询或切换代理方式 |
| `env [--shell bash\|zsh\|fish\|powershell\|cmd] [--unset]` | 输出代理环境变量语句，不修改父终端 |

```sh
eval "$(cor env --shell bash)"
eval "$(cor env --shell bash --unset)"
cor status --watch --jsonl
cor run --data-dir /path/to/server-data
```

Linux TUN 使用 DEB 安装的固定授权助手；第一次执行 `cor proxy tun` 或开启 Claude 保护时，会在前台调用 pkexec，可使用桌面授权代理或终端认证。授权后再由后台重启内核继承网络权限，GUI/CLI 不作为 root 长期运行。系统须提供 `/dev/net/tun`，容器/WSL 仍受设备和宿主网络能力限制。

Windows 使用安装包的网络服务，macOS 使用现有网络助手；首次安装或助手升级可能触发 UAC/系统授权。无人值守前应先在交互会话完成首次授权。系统代理作用于当前用户的桌面会话；没有桌面的服务器通常使用 `env` 或已授权 TUN。

Linux 服务器可直接安装现有 DEB 并在 SSH 中使用 `cor`，无需启动桌面窗口或显示服务器。安装包仍含桌面组件及其依赖；只运行 `cor` 不会启动这些组件。前台运行用 `cor run`，独立后台用 `cor start`；当前用户的登录自启见下方 `service` 命令。

### 订阅

| 命令 | 作用 |
| --- | --- |
| `profile list [--filter TEXT]` | 列表、当前订阅、更新时间和用量 |
| `profile add NAME URL_OR_FILE` | 下载或导入 YAML 并校验 |
| `profile import FILE --name NAME` | 文件导入；`FILE` 可为 `-`，从 stdin 读取 |
| `profile update [PROFILE] [--all]` | 更新当前/指定/全部非内置订阅 |
| `profile use PROFILE` | 切换当前订阅 |
| `profile edit PROFILE [--name NAME] [--source SOURCE]` | 修改元数据；来源变更后执行 update 下载 |
| `profile remove PROFILE --yes` | 删除；当前订阅会切回默认直连 |
| `profile export PROFILE [-o FILE]` | 输出缓存 YAML，包含凭据的完整导出须加 `--show-secrets` |
| `profile directory` | 输出订阅缓存目录 |

```sh
cat subscription.yaml | cor profile import - --name 本地订阅
cor profile export 本地订阅 --show-secrets -o backup.yaml
```

stdin 导入保存在数据目录 `imports/`，作为可再次导入的本机来源；删除订阅不会删除用户原文件。端口、密钥和运行目录仍由客户端覆盖，不直接信任 YAML 的控制接口设置。

### 节点与规则

| 命令 | 作用 |
| --- | --- |
| `node list [GROUP] [--filter TEXT --sort name\|latency --reverse]` | 列出节点/策略组；默认自然名称排序 |
| `node select GROUP NODE` | 切换节点；保护期间禁止切换 |
| `node test NODE` | 测试单个节点/策略组 |
| `node test --group GROUP` | 单策略组测速 |
| `node test --all` | 全量测速，自动策略组按共享逻辑重新选优 |
| `rule list [--profile PROFILE --filter TEXT]` | 规则文本、来源、启用状态、稳定 ID 和序号 |
| `rule add RAW [--position N --disabled]` | 添加自定义规则，默认第一条 |
| `rule edit RULE RAW` | 修改自定义规则 |
| `rule remove RULE --yes` | 删除自定义规则 |
| `rule move RULE POSITION` | 调整顺序，位置从 1 开始 |
| `rule enable RULE on\|off` | 自定义规则开关 |
| `rule shortcut [NAME] [on\|off]` | 查看或开关快捷规则 |

规则命令均支持 `--profile`，默认操作当前订阅。`RULE` 可用稳定 ID 或当下列表序号；脚本建议用稳定 ID。订阅原规则只读，可调整顺序；快捷规则使用专用开关。原始 YAML 不会被重写，保存时先用 mihomo 校验，再通过事务提交。并发 CLI 修改在后台锁内基于最新状态执行。

快捷规则名称依次为 `ads`、`private-domain`、`private-ip`、`cn-domain`、`cn-ip`，对应现有五项规则，仅补充缺少的规则，不重复添加，默认全关闭。

```sh
cor rule add 'DOMAIN-SUFFIX,example.com,DIRECT' --position 1
cor rule shortcut ads on
cor rule list --json
cor rule move c:完整UUID 1
```

节点、连接和规则列表支持 `--limit`（默认 100，最大 10000）、`--offset`，先筛选和排序再分页。

### 连接与日志

| 命令 | 作用 |
| --- | --- |
| `connection list [--watch --interval 1]` | 连接域名、线路、流量、双向速度 |
| `connection show ID` | 完整字段；目标 IP 缺失时按需经内核/加密代理解析补充 |
| `connection close ID` | 关闭一条连接 |
| `connection close --all --yes` | 关闭全部连接 |
| `logs [--follow --level warning --filter TEXT --limit 100]` | 不做终端宽度截断，按等级/文本过滤，可实时跟随 |
| `logs clear --yes` | 清空有界缓冲区 |
| `logs export -o FILE` | 导出当前缓冲区，控制接口密钥会替换 |

连接排序：`--sort name|rule|chain|upload|download|upload-rate|download-rate|start|none`，文本默认升序，流量/速度/时间默认降序，`--reverse` 反转。速度由两次真实计数采样计算，首次查询会等待一个采样周期；新建、计数重置或开始时间变化的连接速度为 null。

日志 `--level` 为精确等级筛选：debug/info/warning/error/silent。缓冲区延用客户端的 500 条上限，不是磁盘历史日志库；长时间未跟随后已淘汰的日志无法补回。`logs --follow` 通过短期租约保持内核日志采集，退出后自动释放。

### 配置、诊断与检测

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
| `claude protect [on\|off] [--yes]` | 可逆保护、锁定节点、TUN/DNS/规则覆盖及实时异常流量拦截 |
| `claude watch [--interval 1]` | 只读观察 Claude 相关请求的真实线路；保护开启时同时查看拦截状态 |

`config set` 支持 `controller_port`、`mixed_port`、`secret`、`dark`、`theme_mode`、`accent_color`、`text_scale`、`tray_speed`、`dns_override`、`ipv6`、`udp`、`ip_reputation_key`、`delay_interval_minutes`、`node_sort`。值先按 JSON 解析，否则视为字符串，例如 `cor config set ipv6 false`、`cor config set theme_mode dark`。枚举取值见 `config get`；模式、订阅、规则、保护必须使用各自命令，不能绕过保护锁定。

检测完成的退出码为 0，可用性与风险应读取报告字段，不能只看退出码。检测报告表示真实观察结果；单项检测失败记为 Failed，不将网络失败当作“可用”。浏览器采样需要用户打开本机入口，在没有真实浏览器的 SSH 会话中不会伪造 WebRTC/指纹结果。报告不读取 Claude 账户 Cookie、令牌或登录内容。项目审计使用 CLI 所在的当前工作目录和终端环境，不套用 GUI/后台启动时的环境。

保护开启会强制 TUN、UDP、加密 DNS 及广泛的 Claude 关联规则，可能影响共享服务；关闭按现有保护恢复机制还原此前配置。操作系统授权仍需用户同意。保护状态持久保存，重启后按原有机制恢复；节点、模式和网络覆盖设置在保护期间保持锁定。

### 维护与后台自启

| 命令 | 作用 |
| --- | --- |
| `geo status` / `geo update` | 查看或校验更新 Geo 数据 |
| `update check` | 仅检查正式版，忽略 prerelease |
| `update download -o FILE` | 下载并校验当前平台安装包，不安装 |
| `update install --yes` | 由运行层下载、校验并交给系统安装，成功交接后退出所有者；有窗口时关闭并在安装后重新打开窗口，无窗口时安装后用 `cor start` 恢复后台 |
| `autostart [on\|off]` | 普通桌面程序登录自启 |
| `service install --yes` | 为当前用户配置 CLI 后台登录自启 |
| `service remove --yes` / `service status` | 删除或查询 CLI 后台登录自启 |
| `notify [MESSAGE]` | 与 Claude 拦截共用系统通知接口，默认“喵!” |
| `completion bash\|zsh\|fish\|powershell` | 输出 shell 补全脚本 |

CLI 后台使用 Linux systemd user unit、macOS LaunchAgent、Windows 当前用户登录任务（无窗口入口启动 `cor`，系统受限时沿用原有注册表回退）。`service install` 配置下次登录启动，当前会话用 `cor start`；`service remove` 只取消自启，停止当前后台用 `cor stop`。不是 Linux 系统级 root 服务，也不自动启用 linger。配置时记录可执行文件和数据目录，移动程序后应重新安装服务。建议桌面自启与 CLI 自启选择一种。卸载或更换安装目录前，对自定义 `--data-dir` 先执行 `cor stop` 和 `cor service remove --yes`；Windows 安装包也会清理默认目录的后台与当前安装的 CLI 自启项。

完整安装更新需要正式安装目录：Linux DEB、Windows 安装程序或 macOS `.app`。便携目录可下载后手动安装。更新前退出其他长时间运行的 CLI 查询进程，避免 Windows 可执行文件被占用。首次系统授权仍可能需要图形会话；无人值守环境可使用 `update download`。

更新下载显示速度、已下载大小及匹配本次连接的实际节点链，回退直连会明确标注；控制接口无法确认出站时显示未确认，不用首页所选节点代替实际线路。`update download` 的进度写入 stderr，`--quiet` 可关闭。

```sh
cor completion bash > ~/.cor-completion.bash
source ~/.cor-completion.bash
cor --json config get dns_override
cor --jsonl connection list --watch --sort download-rate
cor ip-check --category AI -o ai-report.json
cor claude audit --offline -o claude-projects.json
```

## 数据与接口

默认数据目录与 GUI 相同，也可通过 `--data-dir` 隔离不同实例。配置、规则和订阅保存复用现有事务与校验机制；`control.json` 保存一次运行有效的随机控制密钥，只监听 `127.0.0.1`，不是公开 HTTP API。不要共享此文件。Unix 下端点文件验证当前用户所有权和 0600 权限；客户端不连接外部传入的控制地址。

自动启动后台的错误写入数据目录 `daemon.log`；日志仅保存本次后台进程的 stderr，并在下次启动时对超过 2 MiB 的旧文件重新创建。正常内核日志保留在共享有界缓冲区中。退出时释放端点和实例锁，网络恢复失败会保留原有恢复记录供下次启动处理。

源码位于 `apps/cli`；原生运行层位于 `crates/client-runtime`；可跨桌面、Android/iOS 复用的业务仍位于 `crates/client-core`。CI 在六个桌面平台编译 console target、运行解析/脱敏/采样/协议单元测试，并在现有系统代理/TUN 测试中验证本机控制接口及 GUI 接入。发布打包直接复用这些已验证的 `cor` 产物。
