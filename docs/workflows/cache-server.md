# 私网原始下载缓存边界

[返回 CI](ci.md)。本侧负责工作流、原包身份、回退和日志；服务端 agent 负责服务实现、部署、数据和服务器文档。本仓库旧 `Store`/`Handler` 与分片辅助函数仅保留历史协议回归，不是部署来源或当前工作流入口。

## 协作规则

不修改或上传 `.github/cache-server/README.txt`，不调用 `/v1/readme`、`/v1/feedback`，不另建 agent 消息通道。需要维护者介入时，只向用户说明必要问题、证据和验收条件。旧 README 和反馈不构成待自动执行的请求。

工作流不删除私网缓存，也不要求服务保留所有历史构建对象。私网只保留可独立下载并验证的原始工具/依赖文件，具体按[可保留白名单](cache-retention.md)实施；清理由服务端进行，保护在途事务与其他使用方。

<a id="客户端与服务端的写入约定"></a>
<a id="可选的上游代拉下载"></a>

## 现有接口的原包用法

客户端读取 `/health`，导入要求同时声明 `validated-cache-commit-v1`、`complete-cache-retention-v1` 和 `upstream-fetch-v1`。前两项仍表示校验后原子提交、拒绝不破坏有效内容；完整保留约束限于当前接纳的原包，不要求保留退役的编译缓存。

恢复使用认证 `GET /v1/lookup?group=upstream/files&key=sha256-<原文件摘要>` 与 `GET /v1/blob?group=upstream/files&id=<逻辑id>`。不使用前缀回退，不上传归档，不访问 Rust 索引。逻辑 key 按原文件内容区分，服务端不可变物理文件名与逻辑 id 分离。一个 key 对应一个原始普通文件；服务端下载并验证长度和 SHA256 后，按成员 `0`、mode 420 封装 gzip tar，恢复时分别验证外层归档与原文件。

## 持久清单补拉

2026-11-05 的服务端协议新增 `upstream-import-queue-v1`。三条工作流均通过公共 `upstream-report` 和 `scripts/upstream_cache.py` 接入；在已有三项原包能力之外检测此能力，优先使用认证 `POST /v1/import/manifest`：

```json
{
  "schema": 1,
  "group": "upstream/files",
  "sequence": 1234001,
  "retry_failed": false,
  "files": [{
    "url": "https://github.com/example/tool/releases/download/v1/tool.tar.gz",
    "size": 123456,
    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  }]
}
```

客户端先将完整观测清单和全部批次请求持久写入 `target/upstream-downloads.json`，再提交；每批最多 1024 项、编码后 1 MiB，按 SHA256 去重。`sequence` 固定取本次 run id × 1000 加 attempt；断连只重发相同请求，不能换序号绕过失败。返回持久 `manifest_id` 后，服务器继续处理，即使 job 已退出或服务重启。客户端优先提交所有批次，再查询，避免前面大文件阻塞后面的任务接收。

应答核对 `schema/group/sequence/manifest_id/total/counts` 和逐项 `key/sequence/job_id`。POST 只返回前 100 项时，使用 `GET /v1/import/manifest?id=<manifest_id>&offset=0&limit=100`，沿 `next_offset` 保存其余分页，拒绝重复项、错身份和异常分页。只保留白名单字段、固定错误码和请求关联号，不导出私网地址、认证配置、runner 绝对路径或原始错误正文。检查点复用已接收批次，只追加新观测文件；收尾重新查询已有清单。

| 报告状态 | 验收含义 |
| --- | --- |
| `not_submitted` | 未确认持久接收；清单、拒绝码或断连证据保留，不能当作服务端已知任务 |
| `pending` | 已接收，但仍 queued/running/retry_wait，或尚未完成全部回执核对 |
| `ready` | 单项 worker 已完成校验入库；整批还须下面的完整验收 |
| `failed` / `superseded` | 终态失败或被较新序号取代；保留固定错误码，不自动重置重试周期 |

**只有清单 `status=ready`、`counts.ready=total`，并核对所有分页逐项 ready，才将整批记为 ready。** HTTP 200/202、`queued`、`complete=true` 都不能单独代表成功；已确认接收但分页检查失败时，保留 manifest id 和 pending/错误，不重报未接收。ready 也不能替代实际 lookup/blob 下载、双层校验和平台恢复验收。

服务端单 worker，单次最多 30 分钟、最多 6 次，按 5/30/120/600/1800 秒退避。客户端正常提交、查询始终 `retry_failed=false`；不会自动重启已耗尽任务。没有持久队列能力时兼容旧单项 `POST /v1/import`，附 `Prefer: respond-async`；只有旧三项能力也满足时才启用。报告明确标记 `legacy-import`，不伪造持久接收；兼容项逐条记录 present/queued/ready/unavailable，与新清单验收区分。

## 服务与验收边界

- 服务端负责队列和 worker 持久性、单并发、重启恢复及任务元数据容量；客户端不直接操作数据库或队列锁。503/queue_full 整批未接收，保留原清单，不截断文件数或自动清理历史。
- 公开原文件由官方 URL 下载，验证大小与 SHA256 后再发布；失败、过期序号、磁盘不足和数据库错误保留已有可用映射。相同内容的并发导入不得互相删除或退化。
- 独立校验官方主机、HTTPS、每次重定向与实际目标地址，禁止私网/回环/链路本地/凭据 URL、非标准端口及 DNS 重绑定；不得向上游转发私网或 GitHub 令牌。GitHub 资产 API 只访问公开资源。
- 原包归入新命名空间；旧数据先按白名单核对、必要时重新代拉或迁移，待使用旧策略的运行结束再由服务端清理。其他仓库、备份、状态库和运维文件不属于本次缓存淘汰范围。
- 如需让原生 setup、SDK、Maven、Go 的全部下载也能回退，应与本侧通过用户约定原包清单及包管理器接入方式；现有目录快照和“有官方链接”都不能作为已完成覆盖的证据。

服务端 agent 已报告隔离回归、小原包冷下载及重启持久性通过；本侧不将转述当作完整 Actions 验收。下一轮须分别核对客户端整批提交/回执、长队列最终入库、来源与在线 inventory、官方不可达时真实恢复。大文件队列、重启/代理空闲与所有链路故障是否消失仍须实际证据，不能用能力声明、绿色构建或 queued 代替。
