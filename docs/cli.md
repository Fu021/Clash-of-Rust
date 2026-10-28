# CLI 使用说明

`cor` 是 Clash of Rust 的无界面命令行客户端，适用于 Windows、Linux 和 macOS。它复用桌面端的内核管理、配置事务、规则、检测和系统集成，不依赖 Slint、托盘或显示服务器。

## 安装与快速开始

从 0.6.2 开始，桌面安装包包含 `cor`，无需单独下载内核与 Geo 数据；0.6.1 及更早的包不包含 CLI。

| 系统 | 命令位置 |
| --- | --- |
| Ubuntu 22.04+/Debian | 安装 DEB 后直接运行 `cor`（`/usr/bin/cor`） |
| Windows | 安装目录中的 `cor.exe`，例如 PowerShell：`& 'C:\Program Files\Clash of Rust\cor.exe' help`；按实际安装目录调整，或自行加入 PATH |
| macOS | `/Applications/Clash of Rust.app/Contents/MacOS/cor`；也可自行为此文件建立 PATH 中的符号链接 |

```sh
cor help profile add
cor profile add 我的订阅 'https://example.com/subscription'
cor profile list
cor profile use 我的订阅
cor start
cor node list
cor node select Proxy 节点名称
cor proxy tun
cor status
cor stop
```

依次添加和选择订阅、后台启动、选择节点、启用 TUN、查看状态并停止。名称和组名以实际订阅为准；首次 TUN 可能需要系统授权。服务器建议使用 TUN；后台登录自启见[`service install`](cli/maintenance.md#维护与后台自启)。

节点和策略组名称以实际订阅为准；带空格的名称应使用引号。订阅可用完整 ID 或唯一名称定位，重名时须使用 ID。CLI 首次启动沿用内置“默认直连”。

GUI 已运行时，CLI 通过带随机密钥的本机接口操作同一内核。GUI 未运行时，需要后台的命令会自动创建无界面后台，无需手动运行 `cor run`；查询、修改配置不会自行开启系统代理。需要网络的命令会启动内核并恢复已保存的代理模式。`status`、`stop`、`help`、`version`、补全生成不会启动后台。

每个数据目录只有一个运行层所有者；随后打开的 GUI 会接入该后台。退出 GUI 会停止内核及其独立后台。`cor stop` 会停止内核、恢复网络；没有关联窗口时独立后台同时退出；有关联窗口时保留后台，窗口可直接重新启动内核。

## 帮助、脚本与输出

所有命令都支持 `--help`；`cor help profile` 列出该组命令。帮助与解析、补全来自同一份命令表。

默认使用中文状态说明和表格；节点列表区分策略组、节点与内置出站，空延迟显示“未测速”，测速失败显示“超时”。速度和流量按 B、KiB、MiB 等单位显示，时间使用运行机器的本地时区。较窄终端会改用分条显示，保留完整名称和 ID；可通过 `COLUMNS` 指定显示宽度。脚本请使用 `--json` / `--jsonl`，字段与原有结构保持一致。

| 通用选项 | 作用 |
| --- | --- |
| `--json` | 单次机器可读输出 |
| `--jsonl` | 逐行 JSON，适合实时状态、日志、连接和检测进度 |
| `--timeout N` | 单次后台请求或浏览器等待的超时秒数，默认 300，最大 3600 |
| `--data-dir PATH` | 独立数据目录；也可使用 `CLASH_OF_RUST_DATA_DIR` |
| `--resources PATH` | 显式指定含 mihomo、默认配置及 Geo 清单的资源目录 |
| `--user USER` | Linux root 以指定用户身份操作其订阅与后台，见[用户隔离](cli/linux-users.md) |
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

| 操作 | 查阅位置 |
| --- | --- |
| 内核、后台、代理、环境变量、连接和日志 | [运行与代理](cli/runtime.md) |
| 订阅增删更新、节点测速、规则编辑与排序 | [订阅、节点与规则](cli/profiles-and-rules.md) |
| 配置、DNS/IP/A÷检测、Geo、客户端更新和自启 | [配置、检测与维护](cli/maintenance.md) |

### 运行与代理

见[命令与授权说明](cli/runtime.md#运行与代理)。

### 订阅

见[订阅操作与失败回退](cli/profiles-and-rules.md#订阅)。

### 节点与规则

见[节点测速、规则草稿和稳定 ID](cli/profiles-and-rules.md#节点与规则)。

### 连接与日志

见[采样、筛选和日志租约](cli/runtime.md#连接与日志)。

### 配置、诊断与检测

见[配置与检测命令](cli/maintenance.md#配置诊断与检测)。

### 客户端更新

默认只检查正式版。要包含预发布版本，先开启对应通道，再检查并安装：

```sh
cor config set update_prereleases true
cor update check
cor update install --yes
```

关闭预发布通道可执行 `cor config set update_prereleases false`。设置按用户保存，GUI 与 CLI 共用；版本选择、返回正式版及安装条件见[更新与后台自启](cli/maintenance.md#维护与后台自启)。

### 维护与后台自启

见[更新、补全与自启命令](cli/maintenance.md#维护与后台自启)。

## 数据与接口

默认数据目录与 GUI 相同，也可通过 `--data-dir` 隔离不同实例。配置、规则和订阅保存复用现有事务与校验机制；`control.json` 保存一次运行有效的随机控制密钥，只监听 `127.0.0.1`，不是公开 HTTP API。不要共享此文件。Unix 下端点文件验证当前用户所有权和 0600 权限；客户端不连接外部传入的控制地址。

Linux 普通用户各管自己的目录；`sudo cor` 复用调用者身份与订阅，直接 root 使用自己的目录。root 的 `profile list --all-users` 提供跨用户只读总览，显式 `--user` 可选择用户；边界与 TUN 授权见[Linux 用户、订阅与授权](cli/linux-users.md)。

自动启动后台的错误写入数据目录 `daemon.log`；日志仅保存本次后台进程的 stderr，并在下次启动时对超过 2 MiB 的旧文件重新创建。正常内核日志保留在共享有界缓冲区中。退出时释放端点和实例锁，网络恢复失败会保留原有恢复记录供下次启动处理。

源码位于 `apps/cli`；原生运行层位于 `crates/client-runtime`；可跨桌面、Android/iOS 复用的业务仍位于 `crates/client-core`。CI 在六个桌面平台编译 console target、运行解析/脱敏/采样/协议单元测试，并在现有系统代理/TUN 测试中验证本机控制接口及 GUI 接入。发布打包直接复用这些已验证的 `cor` 产物。
