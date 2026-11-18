# Agent 开发指南

仅保留跨任务约束与入口；用户当前授权与明确要求优先。

## 文档与上下文

- 按需查 [README](README.md)、[贡献要求](.github/CONTRIBUTING.md)、[开发指南](docs/development.md)、[工作流索引](docs/workflows.md)与[复盘索引](docs/agent-failures/README.md)。
- 用 `rg` 定位关键词和调用者，按需读取；不重读未变化内容或重复输出大段日志。事实保留一处权威说明，其他文档链接引用。
- 通用文档记录行为、修复结论与验证边界，不维护本仓库 issue 编号或处理状态；待办文件可保留相关的公开 issue 线索，当前状态仍以 GitHub 为准。保留解释实现和补丁所必需的上游来源。
- 文档冲突、精炼拆分及目录职责按[结构复查](docs/structure-review.md)处理；保留有效约束、证据、来源与许可证，整体重构先提交方案供维护者批准。
- [最近复盘](docs/agent-failures/reviews.md)最多 20 条，每条只记结论、证据和验证范围；稳定经验归入主题页，已有模式只链接，不堆任务流水。

## 本地待办入口

- 开始、恢复任务及汇报 todo 进度时，先读[本地待办与持续跟进](docs/agent-todo.md)，按其中规则检查 `todo/`、持续推进、交接和清理；用户中途提问或补充需求时简要回应并继续，除非明确要求停下或所有当前获准任务都无法推进。汇报须包含当前任务进度、后续安排及需要维护者决定的事项。
- `todo/` 是当前工作区的本地队列，由 Git 忽略。只保存需求、公开证据、验证边界与接续条件，凭据、真实订阅、用户现场原始日志及本机会话信息不得进入任务文件。

## 开发约束

- 修改前读调用链、测试与 diff，保留用户未提交内容，不改无关文件或撤销他人改动；职责、生命周期与性能按[开发指南](docs/development.md)执行，平台 FFI、`unsafe` 和特权入口遵守[持续维护要求](docs/development.md#ffi-与特权边界维护)。
- 共享业务与平台/GUI 分层；GUI/CLI 共用后台与内核。界面、协议、宿主、业务、持久化和预览保持贯通，不重复后台任务，不覆盖草稿或原订阅。
- 控制器仅监听本机，随机密钥不泄露；网络操作沿用平台授权与恢复入口。真实代理/TUN 测试只在可丢弃环境执行，不改变用户当前网络。
- 使用仓库期间按[占用锁协议](docs/development.md#仓库占用锁)持有共享锁；仓库维护由专用 agent 负责，不纳入开发任务和 loop。

## 资源受限环境

- 仅有 1 核、约 1 GiB 内存或资源明显不足时，不做本地 Cargo/Gradle/Go 构建、Clippy、测试、打包，不运行下载依赖或生成安装包的脚本；授权后使用现有 Actions。
- 本地仅静态检查及基本 UI 预览，优先 `python3 scripts/preview-slint.py --check`；按[代表尺寸](docs/loop-workflow.md#多尺寸-ui-检查)串行检查相关页面的一种主题，不生成全量页面/对话框/主题矩阵。
- 预览 `--output` 写到 `/tmp` 等工作区外，不改动用户已有文件。

## 任务收尾与失败复盘

- 功能、Bug 修复及公开发布前执行[循环审查](docs/loop-workflow.md)，依赖审计、升级与替换按该文的维护者授权和兼容约束执行。用户说“loop 工作流”即包含逐轮修复、验证、复查和统一收尾，直至无新增可落实问题；如实记录未验证项与阻塞。
- 收尾复查 diff、命令结果、未验证项与[同类风险](docs/agent-failures/README.md)，更新[最近复盘](docs/agent-failures/reviews.md)；新模式归主题，已有模式不重复。
- 精简文档后执行 `git diff --check`、`git status --short`，确认提交范围符合任务要求。

## Actions、提交与发布

- 修复与新功能直接在 `main` 完成，确认远端状态再推送 `origin/main`；纯文档留本地随下次功能更新推送，不单独触发 CI/打包。用户指定保留分支、暂不推送等要求优先。
- 推送前复查 diff、验证与工作区。禁止强推 `main`。分叉、冲突、凭据或权限异常时保留现场并说明阻塞。
- 按[改动范围](docs/development.md#按改动选择验证)选择验证，推送后跟进最终提交的相关 CI；失败则定位并修复。网站独立改版不派发客户端兼容、内存或不发布核验；客户端改动按受影响平台验证，公开发布前必须完成同提交 Android、Ubuntu 兼容及发布门禁。正式版发布流程完成同提交验证后，不再重复不发布打包。不以相关验证运行中收尾，按[历史耗时](docs/workflows.md#actions-跟进节奏)低频查询：CI 约 10 分钟首查、之后 5 分钟；打包约 7 分钟首查、之后 2～3 分钟，排队或长步骤延长。runner 未分配导致相关验证失败时自动重试失败 job，无需再次确认。
- 最终合并前按[脚本说明](scripts/README.md)核对全仓调用关系，合并重复入口、删除弃用脚本并同步调用者。
- [Publish Release](docs/workflows/release.md)默认用 `check_only=true`，复用同提交成功 CI 与依赖快照，核验附件及 Release 未变；流程结束前不推进来源分支。公开发布、签名、额外工作流及版本调整须用户明确授权。
- 常驻 **CI、Publish Release、Memory Benchmark**；维护者另已授权手动 [Android](docs/workflows/android-compatibility.md)、[Ubuntu](docs/workflows/ubuntu-compatibility.md)、[Debian](docs/workflows/debian-compatibility.md) 兼容验证及 [COSMIC 启动诊断](docs/workflows/cosmic-startup.md)。兼容验证复用同提交成功 CI 产物，Android 和 Ubuntu 是发布门禁。Android 截图和诊断为可选扩展。Linux 每个受验发行版使用独立入口和固定版本。新增其他含临时工作流仍须维护者同意。源码与候选包归 CI；发布复用验收结果，仅保留发行签名后的必要验证与公开发布。
- `.github/` 变更及外部 fork 每次运行须维护者审核；遵守隔离、网络和 Secrets 限制，不擅自取消他人运行、删除历史或附件。弃用工作流须删除文件并由维护者在 GitHub 禁用。
- 工作流的跨运行缓存使用 [GitHub Actions Cache](docs/workflows/cache-coverage.md)，源码、工具与镜像来自公开上游。同提交的快照、候选包与验收报告通过 Actions artifact 分发。

## 文档与提交内容

- 行为、命令或构建变化同步权威文档，README 只保留概览与入口；保留必需资源、第三方来源/许可证及共享业务与独立宿主。
- 不提交产物、缓存、历史截图、排障日志、个人 AI 配置或真实订阅/Cookie/令牌/私钥；示例用虚构数据，公开日志脱敏。工具持久安装，不顺手升级无关依赖或改锁文件。
- 汇报改动与风险，区分“通过 / 失败 / 未执行 / 仅静态检查”；其他提交 CI 不算本次验证。提供必要文件或预览，未授权发布保持待办。
