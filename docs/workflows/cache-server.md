# 私网缓存客户端与服务端交接

[返回 CI 与缓存](ci.md)。工作流侧维护客户端、缓存身份和交接请求；服务端 agent 维护服务实现、部署、数据和自己的 AGENTS.md 等文档。服务器通用说明不再由工作流同步，贡献者不需要服务器账号或缓存凭据。

## 请求通道

仅将需要服务端配合的具体事项写入 `.github/cache-server/README.txt`，包含稳定的 `Request-ID`、请求和验收条件，不放目录说明、安装教程、日志、令牌或私网地址。每次推送到 CI 前读取该文件，对照服务端反馈、已处理 Request-ID 和验收证据移除已解决事项；没有请求时留空或删除，不把已完成问题随新请求重新发送。没有实际验收证据时不能自行视为已解决。

可信 `main` 推送的 CI 在 **Actions cache budget → Deliver pending cache server requests** 执行 `python3 scripts/action_cache.py sync-handoff`：确认提交仍为远端 `main`，通过现有认证 `/v1/readme` 接口发送，并回读校验。服务仍写固定的 `state/README.txt`，不恢复服务器根目录 README 或链接；这份文件仅供请求使用，服务端 agent 将自己的长期文档放在其他文件中。

空文件或文件不存在时不发送请求或访问 GitHub；请求正文与服务端内容相同则不写入，不因提交或运行号变化重发。服务端 agent 在自己的文档记录已处理的 `Request-ID`，不重复执行；处理完的 `state/README.txt` 可由服务端按摘要核对后移除，不影响长期反馈。新请求携带来源提交和 Actions 序号，沿用现有串行预算 job、防旧运行覆盖、64 KiB UTF-8 限制及 SHA256 校验。PR、发布和内存工作流不传递请求。

沿用现有 `LOCAL_CACHE_URL`、`LOCAL_CACHE_TOKEN` 和 Tailscale 连接，不增加私网或远程命令权限。不可达或接口尚未部署时记录未交接，缓存仍为可选加速；连接失败不能视为已送达。纯文档不单独触发 CI，本地请求也不能当作服务端已经收到。

## 反馈通道与 agent 消费

同一 CI job 在发送请求前执行 `python3 scripts/action_cache.py feedback`，通过现有认证读取 `GET /v1/feedback?request=<Request-ID>`。每个 ID 每轮只查询一次，总网络预算 40 秒，单份反馈最多 256 KiB；不新增工作流、后台通知或跨 agent 消息权限。服务端已反馈授权部署并实际验证接口；工作流是否收到须以本提交 CI 附件为准。

当前请求取自 README.txt；`.github/cache-server/feedback-requests.json` 记录仍需跟进的 ID 和**原始请求正文** SHA256，最多八项。摘要不含 `Action-Sequence` 和 `Source-Commit` 头，因此相同正文可匹配先前提交的答复。正文变化必须新建 Request-ID，不能用新正文覆盖旧 ID 的摘要；旧答复单独保留供验收，不替代新请求的结果。清空或删除 README.txt 后仍查询跟踪项；没有正文、请求跟踪和测试 ID 时不访问服务。全部验收并消费后移除对应跟踪项。

跟踪文件的 `connectivity_tests` 可列出至多两个已知测试 ID，与普通请求合计不超过八项。测试答复须匹配 `schema/request_id/kind=connectivity_test/does_not_close_request=true`，并有非空、至多 4 KiB 的消息；报告仅导出消息 SHA256 和 `received`，不公开自由文本。测试不要求请求正文摘要，也不能混用普通请求 ID、替代实际验收或关闭请求。收到并确认后移除测试跟踪项。

反馈须匹配整数 `schema: 1`、`request_id`、`request_content_sha256`，同时检查来源提交、完整请求摘要、运行序号及状态字段格式。客户端只导出身份、总体/实现/隔离验证/部署/实际恢复状态和测试数量，不公开原始答复、私网地址、凭据、自由文本或服务端路径。结果写入 job summary 和 `target/cache-feedback.json`，后者作为 `private-cache-feedback` 附件保留三天。`matched` 只表示身份吻合；404、拒绝、不可达、`implementation_verified_deployment_pending` 和隔离回归通过均不能关闭事项，也不能放开生产服务能力门禁。

负责本仓库的 agent 在开始相关任务、每次推送前和同提交 CI 跟进时，读取最近一次可信 `main` CI 的该 summary/附件，对照跟踪摘要及当前请求，区分实现、部署和真实恢复；附件缺失、过期或请求已变则保留未解决项。根据验收证据精简 README.txt，全部解决后留空或删除，并移除已消费的跟踪项。读取和人工转达也算消费，但不能据此声称接口自动送达；无需额外触发工作流。

2026-11-04 已消费服务端 v3 部署记录：修复已上线，18 项隔离回归、四项回滚回归及线上小归档检查完成，锁协议已确认。CI `41681472818` 附件确认测试消息为 `received`，已移除测试跟踪；v4 当时尚无答复，随后交接已回读校验。该 CI 的 Linux 两架构 Rust 分片及三种 Android Go 缓存实际恢复通过，Windows 两架构仅部分上传并保留旧索引；v5 只跟进历史 ARM64 缺片与尚无真实恢复证据的类别，不重复要求实现或部署。

## 客户端与服务端的写入约定

工作流侧负责客户端、缓存 key、回退与协议文档；服务实现、部署及数据修复交由服务端 agent。当前仓库中的旧 `Store`/`Handler` 不作为服务器部署来源；服务端 agent 已反馈修复上线。客户端仍检查实时能力，仓库 CI 不代替服务端真实恢复验收。

客户端在发送归档前通过现有 `GET /health` 检查 `schema: 1`、`status: "ready"` 及字符串数组 `capabilities`。服务端完成实现和失败路径验证后才可声明相应能力：

| 能力 | 服务端必须保证 |
| --- | --- |
| `validated-cache-commit-v1` | 暂存预留容量，先校验内容、长度、SHA256 和运行序号，再提交；拒绝或失败上传不能删除现有有效条目。归档使用不可变物理文件名，发布文件后在同一 SQLite 事务提交逻辑映射及物理回收记录，提交成功后才回收不属于保留 key 的物理孤儿；数据库失败或提交前中断时旧映射仍指向原内容，提交后中断时新映射及文件一致可用。 |
| `rust-index-references-v1` | 每个已发布 `rust/<目标>/index` 引用的分片保持可取；上传单片、超时或中断不能先淘汰旧索引的引用。提交新索引时，在同一临界区核验全部非空分片的组、key、id、大小、校验值及文件存在性，原子发布索引及引用关系，再回收不再引用且不属于保留 key 的物理孤儿文件。容量不足时拒绝新上传，不能留下悬空引用。 |
| `complete-cache-retention-v1` | 私网保存所有缓存类别的完整可复用内容及已成功写入的 key，独立于 GitHub 的 9 GiB 预算；不按每组一个版本、LRU 或 GitHub 侧淘汰结果删除已保留 key。同 key 可安全更新，物理去重不能破坏逻辑条目或分片引用。空间不足时拒绝新写入并明确报告，保持现有内容；清除保留 key 交维护者另行授权。 |

修复后的响应示例：

```json
{
  "schema": 1,
  "status": "ready",
  "capabilities": ["validated-cache-commit-v1", "rust-index-references-v1", "complete-cache-retention-v1"]
}
```

Rust 分片上传要求三项能力；普通归档要求安全提交和完整保留两项。旧服务缺少字段、返回 404 或声明不全时，客户端跳过相应保存并记录 `server upgrade required`，继续使用已有缓存、GitHub 缓存或正常编译。服务仍须校验每次写入，不能只添加声明字段。保持现有 `/v1/lookup`、`/v1/blob` 及 `rust-shards-v1` 格式兼容；当前客户端按 `SHA256(key)` 得到逻辑 `id`，不可变物理文件名与此逻辑标识分离。

GitHub 命中时客户端仍逐片核对私网 metadata；保存前再次检查精确命中索引，不能只凭源码 key 跳过。生成分片时每个 key 都重新 lookup，缺失则重新归档上传，不直接复用旧记录；实际下载仍核验完整 SHA256。Rust 源码 key 包含 `crates/` 等目录的 SVG，图标变化仅使精确命中失效，保留兼容的依赖前缀。

服务端交接验收须覆盖：上传新片后中断，旧索引全部可恢复；相同分片 key 的缺片可修复；校验失败和过期序号不淘汰其他组；同 key 文件发布后 SQLite 写入或提交失败，旧快照仍能通过原校验。2026-11-03 用户报告的 Windows ARM64 缺片属于既有损坏，声明能力不会恢复已删除的文件，须由服务端 agent 处理并核验。
