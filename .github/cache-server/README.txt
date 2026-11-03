Request-ID: cache-safety-and-repo-lock-2026-11-03-v2

请服务端 agent 完成以下配合事项；仅完成实现与失败路径验收后声明能力。
请在你维护的文档记录此 Request-ID 的处理结果，避免重复执行。

原审阅五项问题的处理状态：
- 问题 1 [P1] 新片提前淘汰旧引用：服务端待修复。工作流已增加能力门禁，
  未声明 rust-index-references-v1 前不再上传 Rust 分片，防止继续触发。
- 问题 2 [P2] 同 key 缺片不修复：客户端已改为每片重新 lookup 后才复用，
  缺片会重新归档上传；GitHub 精确命中及保存前也核对索引引用的 metadata。
  仍需服务端保证 lookup 只返回存在的分片，并允许相同 key 的缺片修复。
- 问题 3 [P2] 无效或过期上传淘汰缓存：服务端待修复。工作流未取得
  validated-cache-commit-v1 时不上传归档，保持读取及正常构建回退。
- 问题 4 [P2] 文件替换与数据库提交不一致：服务端待修复，共用上述门禁。
  请按下文实现不可变物理文件与原子逻辑映射，不能只追加能力声明。
- 问题 5 [P2] Rust 源码 key 漏 SVG：客户端已将 apps/crates/vendor 等目录
  的 .svg 纳入源码 key；图标修改使精确 key 失效，依赖兼容前缀不变。

客户端改动位于 scripts/action_cache.py、scripts/cache_shards.py，
回归已补入现有 scripts/test-build-tools.py，覆盖缺片、精确命中、
metadata 不一致、安全能力门禁及 SVG key。
推送前的资源受限主机仅完成静态检查，回归测试未在本地运行；
客户端 CI 请按文件头 Source-Commit 核对实际结果，不能借用其他提交。
服务部署、真实归档恢复及五项问题的端到端验收须另提供实际证据。

1. GET /health 增加 capabilities 字符串数组。
   validated-cache-commit-v1：
   暂存预留容量，内容、长度、SHA256 和运行序号通过后才决定淘汰。
   拒绝、过期或失败上传不得删除其他有效缓存。
   使用不可变物理文件名，先发布文件，再事务提交逻辑映射，最后回收旧文件。
   验收：预算紧张时校验失败及过期序号不丢旧缓存；同 key 数据库写入、
   提交失败或提交前进程中断时，旧映射及内容仍可按旧校验值恢复；
   提交成功后中断时，新映射及内容仍一致可用。

2. rust-index-references-v1：
   所有已发布 rust/<目标>/index 的非空引用必须受保护。
   新分片上传、中断或超时不能先淘汰旧索引引用的文件。
   提交新索引时原子校验分片组、key、id、大小、SHA256 及文件存在性，
   提交后才回收已无引用的版本。容量淘汰不得留下悬空引用。
   验收：上传部分新片后中断，旧索引全部仍可取；缺片时拒绝发布索引。
   保持 /v1/lookup、/v1/blob 及 rust-shards-v1 兼容，逻辑 id 仍为 SHA256(key)，
   不可变物理文件名独立于逻辑 id；相同 key 的缺片须允许重新上传修复。

   预期响应：
   {"schema":1,"status":"ready","capabilities":["validated-cache-commit-v1","rust-index-references-v1"]}

   工作流在能力不全时跳过保存，继续读取与正常构建。
   2026-11-03 报告的 Windows ARM64 已丢失分片需另行恢复并核验；
   新增声明不会自动修复既有损坏。

3. 为专用仓库维护 agent 准备占用锁：
   每个仓库使用 git rev-parse --path-format=absolute --git-common-dir
   返回目录中的 repo-usage.lock，文件内容为空，不入库。
   使用方持共享 flock，维护方持排他 flock，均覆盖整个操作期间。
   维护方非阻塞获取失败（退出码 75）时跳过，不按时间/PID 判过期。
   不删除、替换锁文件或包含它的 Git 公共目录；会话退出自动释放。
   worktree 共用 Git 公共目录；各独立 clone 分别加锁。

   使用方在实际工作 Bash 会话执行：
   cor_git_common=$(git rev-parse --path-format=absolute --git-common-dir) || exit
   umask 077
   exec 9>>"$cor_git_common/repo-usage.lock" || exit
   flock --shared --nonblock --conflict-exit-code 75 9 || exit "$?"

   维护方使用相同打开方式，将最后一行改为：
   flock --exclusive --nonblock --conflict-exit-code 75 9 || exit "$?"

   验收：一个或多个使用会话存活时维护方无法加锁；
   全部使用会话退出后维护方才可持锁；维护期间新使用会话无法加锁。

4. 请保留现有 /v1/readme 认证接口及固定 state/README.txt 写入位置，
   保持服务器根目录 README.txt 的既有交接入口可读。
   工作流只传递待协作请求，长期服务器说明由你的 AGENTS.md 等文档维护。
   请向维护者反馈此 Request-ID 各项完成情况、验证结果及需客户端调整之处，
   区分实现完成、静态检查、实际恢复验收和仍待处理事项。
