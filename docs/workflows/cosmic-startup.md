# COSMIC 启动诊断

[工作流索引](../workflows.md) · [后台自启动与托盘](../agent-failures/desktop.md#静默自启动与托盘告警) · [桌面启动诊断](../diagnostics.md#桌面启动与托盘)

维护者授权长期保留的手动工作流，用于在可丢弃的官方 Pop!_OS 24.04 live VM 中复现 COSMIC Wayland 登录、自启动入口迁移与窗口行为。不加入常规 CI 或发布必需门禁；Android、Ubuntu 兼容验证的发布要求不变。

## 启动与输入

工作流文件为 `.github/workflows/cosmic-startup-investigation.yml`，唯一输入 `run_id` 指向当前所选提交已成功的 CI。工作流与候选包必须属于同一完整提交，复用已验证的 Linux x64 DEB、摘要与收据，不重新编译。

```bash
gh workflow run cosmic-startup-investigation.yml --ref main -f run_id=<成功的CI编号>
```

仅有一个 Ubuntu 24.04 runner job，最长 60 分钟；同一时间只运行一个 COSMIC 诊断，后来的请求排队，不取消进行中的运行。资源受限的开发机不在本地启动 VM。

## 实际验证

1. 从固定官方 HTTPS 来源下载 ISO，核对固定长度和 SHA256；在报告记录来源、镜像身份、实际内核及 COSMIC 包版本。
2. 在隔离 VM 安装候选包，保持普通菜单入口为前台启动，把该菜单文件链接到测试用户的自启动目录。
3. 完整退出测试用户的管理进程并重新登录，确认首次仍前台显示，应用将旧链接迁移为权限 `0600`、带 `--background` 的普通文件，原菜单保持不变。
4. 再次完整登录，确认启动记录属于新会话，托盘已创建、窗口隐藏。
5. 从菜单手动打开，确认复用已有后台的 PID 与启动会话，窗口重新显示。

登录前后核对 systemd 用户管理进程的 `InvocationID`、合成器及真实会话环境；仅重启 display manager 不算完整登录。不会手动启动应用来冒充自启动成功。先连接串口保留启动过程，再协商 QEMU 监控接口；协议问候和能力响应各最多等待 60 秒，之后命令仍受 10 秒超时约束。隔离串口、包管理器锁及截图的判定边界见[测试环境](../agent-failures/test-environments.md)。

## 证据与结果

`cosmic-startup-investigation` 附件保留三天，包含候选包收据、ISO 身份、各阶段 JSON 与原始 PNG，以及串口、QEMU 和 guest journal。QMP 无法直接截图时，采集隔离 Xvfb 显示器并在 `captures.json` 中记录方式；缺失截图或阶段取证失败不算通过。

阶段失败时保留原错误，并尽可能记录自启动入口、已有启动记录、用户管理进程、相关用户单元和应用 journal。区分 ISO/启动、安装、登录前置失败与应用行为失败；绿色 CI、COSMIC 桌面截图和托盘对象创建均不能单独代替全部阶段验证。

截图保留完整现场；Live 安装器或其他窗口遮挡应用时，须明确说明，不能把部分画面当作完整外观验收。窗口状态与手动打开后的进程复用还须联合核对启动记录、PID 和会话身份。

镜像包版本与真实用户机器可能不同；VM 结果不代表已经验证用户的具体 COSMIC 版本、面板配置、长期登录或真实网络。

## 边界与维护

宿主脚本为 `scripts/investigate-cosmic-startup.py`，guest 脚本为 `scripts/cosmic_startup_guest.py`；离线回归在 `scripts/test-build-tools.py`。三者作为同一入口维护，不在每次验收后删除重建。

只下载官方公开镜像，不访问私网缓存或传递订阅、控制密钥、GitHub 凭据；不恢复或保存 GitHub Cache，ISO、VM 写入层和工具目录仅存在于当次 runner，短期诊断附件不作为缓存。guest 中的服务重启和用户会话退出仅作用于该隔离 VM，不修改开发机网络或用户会话。工作流本身不发布 Release、不更改更新通道。

修改镜像、驱动或断言后，重新核对所有阶段、失败取证及实际截图。镜像升级须同步官方身份、启动方式和已验证范围，不以自动跳过或放宽条件消除失败。
