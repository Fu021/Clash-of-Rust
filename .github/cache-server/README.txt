Request-ID: cache-safety-complete-mirrors-and-repo-lock-2026-11-03-v3

请服务端 agent 完成以下配合事项；仅完成实现与失败路径验收后声明能力。
请在你维护的文档记录此 Request-ID 的处理结果，避免重复执行。
工作流每次推送前会依据反馈及验收证据移除已解决事项；无待协作请求时留空或删除。
请对照旧 Request-ID 的验收记录：已完成项目只反馈证据，不重复实施。

原审阅五项问题的处理状态：
- 问题 1 [P1] 新片提前淘汰旧引用：服务端待修复。工作流已增加能力门禁，
  未具备安全提交、引用保护及完整保留能力前不上传 Rust 分片，防止继续触发。
- 问题 2 [P2] 同 key 缺片不修复：客户端已改为每片重新 lookup 后才复用，
  缺片会重新归档上传；GitHub 精确命中及保存前也核对索引引用的 metadata。
  仍需服务端保证 lookup 只返回存在的分片，并允许相同 key 的缺片修复。
- 问题 3 [P2] 无效或过期上传淘汰缓存：服务端待修复。工作流未取得
  validated-cache-commit-v1 时不上传归档，保持读取及正常构建回退。
- 问题 4 [P2] 文件替换与数据库提交不一致：服务端待修复，共用上述门禁。
  请按下文实现不可变物理文件与原子逻辑映射，不能只追加能力声明。
- 问题 5 [P2] Rust 源码 key 漏 SVG：客户端已将 apps/crates/vendor 等目录
  的 .svg 纳入源码 key；图标修改使精确 key 失效，依赖兼容前缀不变。

客户端的缺片检查与 SVG key 已修复，不请求服务端重复实现客户端工作。
本轮还补齐 Linux 下载缓存、桌面资源路径及开发模式编译缓存的私网镜像，
并新增完整保留门禁。资源受限主机仅做静态检查，回归交最终提交 CI；
请按文件头 Source-Commit 核对实际结果，不能借用其他提交。
服务部署、真实归档恢复及五项问题的端到端验收须另提供实际证据。

1. GET /health 增加 capabilities 字符串数组。
   validated-cache-commit-v1：
   暂存预留容量，内容、长度、SHA256 和运行序号通过后才提交。
   拒绝、过期或失败上传不得删除其他有效缓存。
   使用不可变物理文件名，先发布文件，再事务提交逻辑映射，最后回收物理孤儿。
   验收：预算紧张时校验失败及过期序号不丢旧缓存；同 key 数据库写入、
   提交失败或提交前进程中断时，旧映射及内容仍可按旧校验值恢复；
   提交成功后中断时，新映射及内容仍一致可用。

2. rust-index-references-v1：
   所有已发布 rust/<目标>/index 的非空引用必须受保护。
   新分片上传、中断或超时不能先淘汰旧索引引用的文件。
   提交新索引时原子校验分片组、key、id、大小、SHA256 及文件存在性，
   提交后只回收不属于保留 key 或引用的物理孤儿；容量不足须拒绝新写入。
   验收：上传部分新片后中断，旧索引全部仍可取；缺片时拒绝发布索引。
   保持 /v1/lookup、/v1/blob 及 rust-shards-v1 兼容，逻辑 id 仍为 SHA256(key)，
   不可变物理文件名独立于逻辑 id；相同 key 的缺片须允许重新上传修复。

   预期响应：
   {"schema":1,"status":"ready","capabilities":["validated-cache-commit-v1","rust-index-references-v1","complete-cache-retention-v1"]}

   普通归档要求安全提交及完整保留两项，Rust 要求全部三项；
   能力不全时跳过保存，继续 GitHub 优先读取、私网回退与正常构建。
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

5. 新增完整保留能力 complete-cache-retention-v1：
   所有工作流统一 GitHub 缓存优先、私网回退；GitHub 命中仍补齐私网缺失副本。
   私网须保留所有缓存类别的完整可复用内容及已成功写入的 key，
   不沿用 GitHub 9 GiB 预算、每组一个版本、LRU 或 GitHub 侧淘汰结果删除这些 key。
   同 key 可安全更新；物理去重不能破坏保留条目或已发布索引的分片引用。
   运行序号保护同一 key 的新内容，不能因同组另一 key 较新而拒绝缺失 key 的首次补齐。
   空间不足时拒绝新写入并明确报告，保持所有已有内容；保留 key 的清理由维护者另行授权。
   仅回收不属于保留 key/引用的物理孤儿和失败暂存文件。

   覆盖现有 Rust（含内存基准 release/dev）、Cargo/Go 源下载、
   Android Go 编译结果、Gradle、Skia、Geo/内核资源及 Windows NSIS，
   并接入本轮新增的 Linux 类别（arch 为 x64 或 arm64）：
   - tools/linux-python-<arch>：按冻结 SHA256 校验的独立 Python 归档。
   - tools/linux-cloud-<arch>：按冻结 SHA256 校验的原始 Ubuntu 云镜像。
   - dependencies/linux-apt-<arch>：按签名软件源索引重新校验的虚拟机 DEB 下载。
   普通归档沿用现有协议和长度/SHA256 校验，服务端不需要解包。
   虚拟机系统盘、已安装状态、测试报告和发布安装包仍不属于缓存。

   验收：两种架构的各类上传均可 lookup/blob；同组不同 key 均保留；
   GitHub 一侧清除后仍可从私网恢复；容量不足的上传失败后旧 key 与 Rust 引用不变。
   /v1/inventory 应完整列出保留 key、大小、校验值及序号，便于核对镜像缺口。
   完成实际实现和失败路径验收后才声明能力，不能只追加 health 字段。
