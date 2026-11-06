# 运行、代理与日志

[CLI 入口](../cli.md)。下表命令省略 `cor` 前缀，通用参数和退出码见入口文档。

## 运行与代理

**日常使用请用 `cor start` 在后台运行。** `cor run` 仅用于前台调试，依赖当前终端；关闭终端或断开 SSH 会结束前台运行，不应用它代替后台启动。

| 命令 | 作用 |
| --- | --- |
| `start` | **推荐：后台启动内核，关闭终端后继续运行** |
| `stop` / `restart [--network-only]` | 停止、重启当前内核；可选择仅刷新网络 |
| `run [--no-core]` | **仅用于前台调试，关闭终端即停止；日常请用 `start`**。默认启动内核，`--no-core` 仅运行控制服务 |
| `status [--watch --interval 1]` | 当前版本、订阅、节点、模式、连接数及总速率 |
| `version` | CLI 版本、操作系统和 CPU 架构 |
| `mode [rule\|global\|direct]` | 查询或切换内核运行模式 |
| `proxy [system\|tun\|off]` | 查询或切换代理方式 |
| `env [--shell bash\|zsh\|fish\|powershell\|cmd] [--unset]` | 输出代理环境变量语句，不修改父终端 |

`proxy` 与 `status` 的代理方式来自当前用户后台的实时快照，与桌面窗口一致；保存的启动偏好不代表当前系统代理/TUN 已经生效。单纯查询不启动后台或改写代理偏好；后台状态读取最多等待 5 秒，失败时 `status` 仍输出路由证据，内核状态为不可读取，不能当成已停止。

Linux 会查询当前网络命名空间中两个示例 IPv4/IPv6 目标的路由，分别说明经过 TUN/TAP、未经过可识别的 TUN/TAP，或无法采样的具体原因。无示例路由不等于 IPv4/IPv6 不可用，其他平台也不会据此判定连通性。发现 TUN 时在 250 毫秒预算内检查同一网络命名空间的进程描述符；能读取时列出接口、PID、有效 UID 和进程名，区分当前用户与其他用户，UID 0 显示 root。持有进程不代表最初开启者；权限、进程退出及扫描预算可能导致部分结果。当前用户关闭代理后，其他实例的 TUN 仍可能影响其流量；无法读取持有方时，须由有权限的用户采集下节的离线报告，不猜测归属，也不自动停止其他用户的 TUN。

`cor restart --network-only` 只对已运行的当前用户内核请求网络刷新：重新应用当前运行配置、刷新出站及 DNS 并关闭该内核的旧连接，不重新启动内核。操作会要求确认；脚本可显式加 `--yes`。其他用户和程序的连接，包括 SSH，可能受到影响。刷新完成只表示控制器已接受操作，并不证明目标服务恢复；中途失败时部分步骤可能已经生效，先用 `cor diagnose` 和 `cor status` 检查。

```sh
eval "$(cor env --shell bash)"
# 未使用 TUN 时，为当前 Bash 终端设置 HTTP/SOCKS 代理环境变量。

eval "$(cor env --shell bash --unset)"
# 删除上一步设置的代理环境变量。

cor status --watch --jsonl
# 持续输出 JSONL 状态，适合脚本采集；按 Ctrl+C 结束查询。

cor start --data-dir /path/to/server-data
# 使用独立数据目录启动后台，替换为自己的目录。

cor proxy tun --data-dir /path/to/server-data
# 为同一数据目录的后台开启 TUN；建议纯 CLI 的服务器使用此模式。
```

Linux 普通用户通过 DEB 的固定助手开启 TUN，首次在前台使用 pkexec 完成管理员认证，SSH/非活动会话也可请求认证。授权后由原用户后台重启内核继承网络权限。`sudo cor` 先授权固定助手再回到调用者；直接 root 自有实例无需 pkexec。具体命令、目录和宿主限制见[用户与授权](linux-users.md)。

Windows 使用安装包的网络服务，macOS 使用现有网络助手；首次安装或助手升级可能触发 UAC/系统授权。无人值守前应先在交互会话完成首次授权。系统代理作用于当前用户的桌面会话；建议 Server 等纯 CLI 环境使用已授权 TUN，无法使用 TUN 时再通过 `env` 配置需要代理的程序。

Linux 服务器可直接安装现有 DEB 并在 SSH 中使用 `cor`，无需启动桌面窗口或显示服务器。安装包仍含桌面组件及其依赖；只运行 `cor` 不会启动这些组件。日常使用 `cor start` 在后台运行，关闭 SSH 终端后仍可使用；仅需前台调试时才用 `cor run`。当前用户的登录自启见[后台自启](maintenance.md#维护与后台自启)。

## 连接与日志

| 命令 | 作用 |
| --- | --- |
| `connection list [--watch --interval 1]` | 连接域名、线路、流量、双向速度 |
| `connection show ID` | 完整字段；目标 IP 缺失时按需经内核/加密代理解析补充 |
| `connection close ID` | 关闭一条连接 |
| `connection close --all --yes` | 关闭全部连接 |
| `logs [--follow --level warning --filter TEXT --limit 100]` | 不做终端宽度截断，按等级/文本过滤，可实时跟随 |
| `logs clear --yes` | 清空有界缓冲区 |
| `logs export -o FILE` | 导出当前最多 500 条缓冲日志，控制接口密钥会替换 |

连接排序：`--sort name|rule|chain|upload|download|upload-rate|download-rate|start|none`，文本默认升序，流量/速度/时间默认降序，`--reverse` 反转。速度由两次真实计数采样计算，首次查询会等待一个采样周期；新建、计数重置或开始时间变化的连接速度显示“尚未采样”（JSON 为 null）。

日志 `--level` 为精确等级筛选：debug/info/warning/error/silent，不代表该级别及以上。`logs` 默认显示最近 100 条，`logs --limit 500` 和 `logs export -o FILE` 可取得当前缓冲的全部记录；缓冲最多 500 条、单条最多 1000 字符，不是磁盘历史日志库，已淘汰或截断的内容无法补回。`logs --follow` 通过短期租约保持内核日志采集，退出后自动释放；可将 `--jsonl logs --follow --limit 500` 的输出重定向到权限受限的文件，保存后续取得的日志，长时间采集须自行限制时长或文件大小。

通过 `cor start` 启动的 CLI 后台将自身 stderr 写入数据目录中的 `daemon.log`，Linux root 默认路径为 `/root/.local/share/clashofrust/daemon.log`；自定义 XDG、`CLASH_OF_RUST_DATA_DIR` 或 `--data-dir` 时以实际目录为准。该文件不保存完整内核日志，后台创建时超过 2 MiB 的旧文件会被移除，不提供历史轮转归档。`cor run` 前台输出和自行配置的 systemd 日志还取决于启动方式，不应把这些文件当作完整流量历史。

## 故障反馈与诊断报告

CLI 顶层操作/参数错误、结构化结果中的部分失败，以及读到的 error 级日志会提示导出报告；同一进程只提示一次，提示写入 stderr，不混入 JSON 结果。Ctrl+C 取消和管道关闭不要求提交故障报告。

```sh
cor diagnose --output cor-diagnose.json
# 网络可检测时保存系统/代理检测、可读取的内核缓冲与本地现场。

cor diagnose --offline --output cor-diagnose.json
# 后台无法启动或需先保留 Geo 故障现场时，不连接后台、不发网络探测。
```

使用发生故障的用户身份，并沿用原来的 `--user`、`--data-dir`、`--resources`；`sudo cor` 的用户选择见[用户与授权](linux-users.md)。报告保存到应用数据目录之外，父目录须已存在。离线采集不会创建应用数据目录、初始化设置、启动后台、恢复 Geo 事务或改变网络；仅报告文件会写入。

报告包含版本、时间、程序与数据/资源路径、路由/TUN 持有进程、`daemon.log` 末尾最多 64 KiB、Geo 清单/事务、备份与恢复目录文件名、大小、时间和采集失败原因。在线导出另尝试读取内核缓冲最多 64 KiB；单项网络探测 15 秒、后台连接与日志查询各 5 秒超时，失败不丢弃其他已完成项目。目录清单和 JSON 分别有数量/大小上限，截断会明确标注；不读取大型 Geo 文件正文或替代完整校验。

日志替换可从本地设置、控制端点和订阅索引读取的凭据，并移除 URL 的认证、路径、查询及片段；这些原始配置不会输出。必要的凭据记录无法读取时不附后台日志。自动脱敏不能覆盖所有第三方内容，分享前检查域名、IP、路径和日志正文。请将报告、原始错误、具体操作与发生时间通过 issue 提供；不会自动上传报告或发送 agent 消息。`--output` 保存成功只证明报告已生成，不代表网络、缓存或故障恢复成功。
