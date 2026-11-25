# Clash of Rust · Agent 指南

Clash of Rust 是基于 Mihomo 的代理客户端。在 Windows、macOS 和 Linux 上，桌面窗口、终端界面和 `cor` 共用订阅、设置和后台。用桌面客户端所属的账户运行命令。如果用户运行了独立实例，每条命令都使用该实例的 `--data-dir PATH`。

## 先核对已安装的版本

运行 `cor --version`、`cor help all` 和 `cor help <命令>`，确认本机版本支持的用法。在交互终端运行 `cor` 或 `cor ui` 可打开终端界面。自动化时调用明确的子命令，单次结果用 `--json`，持续输出用 `--jsonl`。同时检查退出码与相关结果字段。

## 先排查，再改动

```sh
cor status --json
cor proxy --json
cor profile list --json
cor diagnose --offline --json
```

这些命令查看当前实例，不会启动内核。先确认内核是否运行、当前订阅及系统代理或 TUN 状态。需要主动联网探测时，可用 `cor network check https://example.com --json` 查看 DNS 结果和本次请求实际经过的线路。要保存本地诊断，可运行 `cor diagnose --offline --output cor-diagnose.json`，选用一个尚未占用的输出路径。

## 按用户要求修改

通过 CLI 修改订阅、规则和设置。订阅重名时使用 ID。遇到 Claude 保护锁定时，把当前保护状态告知用户。切换系统代理或 TUN 前确认用户想用的模式，操作系统授权弹窗由用户处理。部分联网命令会启动内核并恢复已保存的代理模式。修改命令中途超时或取消后，先确认实际状态再重试。

保留默认凭据脱敏。分享报告前检查其中的网址、域名、IP 和本机标识。订阅链接及 `control.json` 应留在本机。

## 项目入口

- [GitHub 仓库](https://github.com/Fu021/Clash-of-Rust)
- [文档导航](https://github.com/Fu021/Clash-of-Rust/blob/main/docs/README.md)
- [CLI 命令参考](https://github.com/Fu021/Clash-of-Rust/blob/main/docs/cli.md)
- [Issue 表单](https://github.com/Fu021/Clash-of-Rust/issues/new/choose)
