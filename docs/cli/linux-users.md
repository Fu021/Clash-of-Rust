# Linux 用户、订阅与授权

[CLI 入口](../cli.md)。本页适用于 Linux DEB 中的 `cor`；Windows/macOS 沿用各自的用户与授权入口。

## 数据归属

每个普通用户默认使用自己的 `~/.local/share/clashofrust`，与该用户的 GUI 共用订阅、设置、后台和内核。同名订阅可以分别存在于不同用户的目录中；普通用户不能通过 `--user`、`--all-users` 或指定别人的数据目录越权访问。

直接登录 root 或进入 root shell 后，`cor` 默认使用 root 自己的数据目录。直接执行 `sudo cor ...` 时则识别 sudo 调用者，以原用户身份执行命令并复用原目录；不会另建 root 后台或将用户文件写成 root 所有。`sudo -u bob cor ...` 按 sudo 指定的实际用户执行。

root 可显式选择用户：

```sh
sudo cor --user alice profile list
sudo cor --user alice status
```

`--user` 切换整条命令的身份，修改也以目标用户权限进行。用户 YAML 始终在降权后读取；降权前清空环境，只保留终端/语言等必要项，并重设身份、HOME、固定 PATH 和目标用户会话总线。显式 `--user` 不继承管理员令牌、代理凭据、证书或数据目录环境。

自动识别 `sudo cor` 时额外保留调用者的代理/证书、显示会话和应用目录白名单，XDG 路径须为目标用户拥有的绝对路径。sudo 若已清除自定义目录变量，或需操作指定用户的自定义实例，用 `--data-dir` 显式传入；不要合并所有用户的目录。多个用户同时运行时，端口及宿主 TUN/路由仍需协调。

## root 只读总览

```sh
sudo cor profile list --all-users
sudo cor profile list --all-users --filter alice --json
```

总览显示订阅所属用户名、UID、数据目录、ID、名称、当前状态、来源与更新时间，来源凭据默认脱敏。只读取系统账户默认目录中的 JSON 索引，不读取 YAML、不启动各用户或 root 后台，也不改写文件；索引必须为所属用户拥有的普通文件，拒绝文件符号链接并限制读取大小。缺失目录跳过，损坏或不安全的索引明确报错。

此入口只支持 `profile list`，不能与 `--user` 或 `--data-dir` 混用，也不自动搜索自定义 XDG/数据目录。要操作某个用户或自定义实例，使用显式 `--user` / `--data-dir`，不要把所有用户合并到一个公共目录。root 总览不改变普通用户之间的文件权限。

## TUN 与无显示服务器

`cor` 不需要 Wayland、X11、桌面窗口或托盘。SSH 中运行 `cor start` / `cor run`；`clash-of-rust` 是桌面入口，无法连接图形服务时会提示改用 `cor`。

普通用户首次执行 `cor proxy tun` 或开启 Claude 保护，由 `pkexec` 请求管理员认证；SSH、非活动会话同样需要管理员认证，不自动免密授权。系统需安装并运行 polkit，终端需可交互认证。没有认证代理时可先仅授权固定助手，再回到普通用户命令：

```sh
sudo /opt/clash-of-rust/clash-of-rust --authorize-tun
cor proxy tun
```

也可执行 `sudo cor proxy tun`：root 只预先调用安装目录中的固定 TUN 助手，随后以原用户身份切换代理。sudo 的启动、重启、前台运行也预先准备该助手，避免恢复已保存的 TUN 时在后台索取授权；`run --no-core` 不需要此步骤。助手不接收用户内核路径或 YAML，不以 root 启动普通用户后台。直接 root 自有实例沿用 root 的网络能力。

设备与容器仍须提供 `/dev/net/tun` 和相应网络能力。仅使用 root 权限无法绕过宿主限制；系统代理也需要对应用户的桌面会话，服务器优先使用 TUN。
