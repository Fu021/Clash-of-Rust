# Android 应用列表预览

[工作流索引](../workflows.md) · [应用分流](../android-application-routing.md) · [CI](ci.md)

`Android Application Previews` 经维护者授权独立运行，只在可丢弃模拟器中打开生产 Slint 应用分流页、读取已安装应用并截图。复用成功 CI 的 `build-android-x64` 和 `dependency-snapshot`，不重新编译，不编辑或保存分流，不申请 VPN，不执行代理、流量、分流或输入法功能测试。

## 启动与来源

先等待目标提交的普通 CI 成功，再任选一个入口：

- 手动运行 `.github/workflows/android-previews.yml`，选择同一提交的分支，填写 `run_id`。
- 工作流尚未进入默认分支时，由维护者为同仓库 PR 添加 `android-previews` 标签触发；已存在时先移除再添加。此入口只响应本次加标签事件，不随之后的 push 自动运行，不对 fork 启用私网。目标提交没有成功 CI 时明确失败，不借用其他提交。

手动入口使用 GitHub 的 `workflow_dispatch`；默认分支的可用性受平台规则约束。标签入口使用普通 `pull_request: labeled`，不使用 `pull_request_target`，不扩大 fork 权限，不自动添加或移除标签。

`scripts/android_previews.py` 核对 CI 路径、成功终态、仓库、来源提交及两份输入附件的唯一性、有效期和 run/commit 身份。之后固定 checkout 该提交，下载对应 CI 附件，应用冻结依赖，核对 build 归档、版本、锁文件及 APK 摘要；设备实际 Android 版本也必须一致。

## 版本与资源

矩阵来自工具基线中已有固定摘要的 x64 系统镜像，任务名显示 Android 版本：

| Android | API |
| --- | --- |
| 8.0 | 26 |
| 8.1 | 27 |
| 9 | 28 |
| 10 | 29 |
| 11 | 30 |
| 12 | 31 |
| 12L | 32 |
| 13 | 33 |
| 14 | 34 |
| 15 | 35 |
| 16 | 36 |

Android 8.1 使用 Google 官方 AOSP `default/x86_64` 第 1 版镜像，其余版本使用 Google APIs x64 镜像；不额外编译 x86 APK。每项最长 35 分钟，最多三个模拟器 job 并行，失败不取消其他版本。仅准备 Python、Java、SDK 和模拟器，不初始化 Cargo、Go 或 Gradle。原包官方优先、私网回退，按现有持久清单提交来源；不保存安装目录或 AVD 快照，不删除私网内容。8.1 的实际启动和截图仍须本轮 Actions 验证。

## 截图与边界

`applicationListPreviewCapturesInstalledApps` 是 instrumentation 的截图入口，仅通过真实触摸导航、等待应用枚举、核对可见条数并采集证据。加载失败、只剩本应用、版本不一致、缺少有效 PNG 或意外启动 VPN 均明确报错，不能把空白截图当成完成。

每个 `android-<版本>-application-list` 附件保留七天，包含 `applications.png`、`state.json`、设备版本、instrumentation 输出和 logcat。状态记录实际枚举总数、可见数和加载状态；清单仅来自 CI 模拟器，不采集用户设备数据。

这些结果用于检查页面和应用枚举，不证明实际分流、VPN、厂商 ROM 或真机可用。本地 Slint 固定数据预览也不能替代本流程的真实枚举结果。代理/TUN 验证仍由普通 CI 执行，iOS 仍为 UI 原型。
