# 运行、代理与日志

[CLI 入口](../cli.md)。下表命令省略 `cor` 前缀，通用参数和退出码见入口文档。

## 运行与代理

**日常使用请用 `cor start` 在后台运行。** `cor run` 仅用于前台调试，依赖当前终端；关闭终端或断开 SSH 会结束前台运行，不应用它代替后台启动。

| 命令 | 作用 |
| --- | --- |
| `start` | **推荐：后台启动内核，关闭终端后继续运行** |
| `stop` / `restart` | 停止、重启当前内核 |
| `run [--no-core]` | **仅用于前台调试，关闭终端即停止；日常请用 `start`**。默认启动内核，`--no-core` 仅运行控制服务 |
| `status [--watch --interval 1]` | 当前版本、订阅、节点、模式、连接数及总速率 |
| `version` | CLI 版本、操作系统和 CPU 架构 |
| `mode [rule\|global\|direct]` | 查询或切换内核运行模式 |
| `proxy [system\|tun\|off]` | 查询或切换代理方式 |
| `env [--shell bash\|zsh\|fish\|powershell\|cmd] [--unset]` | 输出代理环境变量语句，不修改父终端 |

`proxy` 与 `status` 的代理方式来自后台实时快照，与桌面窗口一致；保存的启动偏好不代表当前系统代理/TUN 已经生效。单纯查询不改写该偏好，恢复网络或重新启动时仍沿用用户保存的选择。

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

Linux 普通用户通过 DEB 安装的固定授权助手开启 TUN；第一次执行 `cor proxy tun` 或开启 Claude 保护时，会在前台调用 pkexec，可使用桌面授权代理或终端认证。授权后再由后台重启内核继承网络权限，客户端本身保持普通用户身份。手动以 root 运行 `cor` 时，直接启动内核并使用已有网络权限，无需 pkexec。系统须提供 `/dev/net/tun`，容器/WSL 仍受设备和宿主网络能力限制。

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
| `logs export -o FILE` | 导出当前缓冲区，控制接口密钥会替换 |

连接排序：`--sort name|rule|chain|upload|download|upload-rate|download-rate|start|none`，文本默认升序，流量/速度/时间默认降序，`--reverse` 反转。速度由两次真实计数采样计算，首次查询会等待一个采样周期；新建、计数重置或开始时间变化的连接速度显示“尚未采样”（JSON 为 null）。

日志 `--level` 为精确等级筛选：debug/info/warning/error/silent。缓冲区沿用客户端的 500 条上限，不是磁盘历史日志库；长时间未跟随后已淘汰的日志无法补回。`logs --follow` 通过短期租约保持内核日志采集，退出后自动释放。
