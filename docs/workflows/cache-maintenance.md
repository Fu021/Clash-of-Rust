# Actions 与私网缓存盘点

[缓存服务交接](cache-server.md)说明配置、部署与 README 同步；本页供维护者及已授权的 agent 在 loop 最后一轮通过后执行盘点与清理。外部贡献者只使用自己的缓存，不索取维护者网络或凭据。

## 先区分两套缓存

| 存储 | 查看和常规清理 | 边界 |
| --- | --- | --- |
| GitHub Actions 仓库缓存 | 可信运行的 **Actions cache budget** 调用 `scripts/workflow_storage.py`，报告并按现有 9 GiB 预算清理；也可用 GitHub API 只读盘点。外部 fork 运行只读。 | 它不管理私网磁盘、Release 或 Actions 附件；不要因循环审查扩大权限或删除不相关条目。 |
| 维护者的私网缓存 | 可信 CI 的 `private-cache-inventory` 和打包的 `packaging-private-cache-inventory` 是服务端清单；服务上传时按自身预算及组版本执行淘汰。 | 服务只提供认证的缓存和 README 接口，没有远程 shell、任意文件删除或自升级接口；Actions 清理 GitHub 缓存不会清理这套磁盘。 |

清单是生成时的快照，不等于当前状态；附件保留 3 天。检查运行编号、提交、时间与任务日志，注明实际时间及数据来源。`continue-on-error` 的步骤即使 job 成功也可能跳过或失败，不能仅凭绿色 job 认定盘点或 README 同步成功。

## 不在缓存服务器时

从当前仓库确认 `git status --short`、分支、提交和远端身份，再使用已有授权的 GitHub 登录；不要要求用户提供令牌，也不要打印 Secrets、私网地址或认证头。选同提交或最近一次可信 `main` CI，确认其 **Actions cache budget** 任务的预算清理及私网清单日志。先将 `COR_CI_RUN_ID` 设为确认过的运行号，再将 `COR_CACHE_JOB_ID` 设为 `jobs[].databaseId` 中的缓存任务号：

```sh
gh run list --workflow ci.yml --branch main --limit 5
gh run view "${COR_CI_RUN_ID:?Set completed CI run ID}" --json status,conclusion,headSha,event,jobs
gh run view "${COR_CI_RUN_ID:?Set completed CI run ID}" --job "${COR_CACHE_JOB_ID:?Set cache job ID}" --log
gh api --paginate --slurp 'repos/{owner}/{repo}/actions/caches?per_page=100' \
  --jq '[.[].actions_caches[]] | {count: length, bytes: (map(.size_in_bytes) | add // 0)}'
```

私网清单可用下列命令下载；查看后删除本次临时目录，不提交原始清单。打包运行使用 `packaging-private-cache-inventory`。

```sh
COR_CACHE_REPORT_DIR="$(mktemp -d "${TMPDIR:-/tmp}/cor-cache.XXXXXXXX")"
gh run download "${COR_CI_RUN_ID:?Set completed CI run ID}" --name private-cache-inventory --dir "$COR_CACHE_REPORT_DIR"
```

读取清单时检查分类、大小、来源序号与显著过期项，摘要脱敏；它不检查服务器实际文件、分片内容或孤立文件，也不能代替实时主机检查。

完成的同提交 CI/打包已执行预算任务时，复用其清理结果，避免为了盘点重复运行工作流。纯文档改动不为此单独触发 CI。没有可用清单、权限、尚有上传或主机不可达时保留数据并报告未完成项；不能把旧快照或“job 成功”称作本轮实时清理成功，也不要求用户交出私网凭据。

## 在缓存服务器上

确认已获得维护者授权，并以实际服务用户检查 unit、`ExecStart` 和挂载权限，定位真实根目录；agent 所在主机和 `$HOME` 不一定是缓存主机。`systemctl --user` 必须在服务用户的 systemd 会话执行。不要输出 `config/server.json`、访问地址或认证信息。服务端的 `status` 会初始化目录和索引并可能迁移 schema，不是严格只读的盘点命令。

在确认的根目录内，用只读 SQLite 连接检查完整性和按组统计；先将 `COR_CACHE_ROOT` 设为实际绝对路径：

```sh
python3 -B - "${COR_CACHE_ROOT:?Set COR_CACHE_ROOT to the confirmed cache root}" <<'PY'
from pathlib import Path
import sqlite3
import sys

root = Path(sys.argv[1]).expanduser().resolve(strict=True)
index = root / "state/index.sqlite"
if index.is_symlink() or not index.is_file():
    raise SystemExit("Cache index is missing or is a symlink")
with sqlite3.connect(index.as_uri() + "?mode=ro", uri=True) as db:
    print("SQLite quick_check:", list(db.execute("PRAGMA quick_check")))
    for group, count, size in db.execute(
        "SELECT category, COUNT(*), SUM(size) FROM entries GROUP BY category ORDER BY category"
    ):
        print(group, count, size)
PY
```

再以不跟随符号链接的方式递归比较 `entries` 与 `data/` 中的实际归档，检查文件类型、大小、缺失、未索引文件、`.incoming/*.part` 的时间及日志占用；分组可有三层。不要将中断上传、孤立文件或旧日志直接当成可删：先排除活动上传、有效引用及排障需要。

### 分片与旧整包

旧 Rust 整包和新分片可能同时存在，并非都应删除。`rust/<目标>/index` 条目对应的文件虽然以 `.tar.gz` 命名，内容是 `rust-shards-v1` JSON；校验该文件的 SHA256 后，检查其 32 个分片记录：`null` 表示空分片，其余记录必须与对应组的索引、文件大小和 SHA256 一致。索引中有条目并不代表其引用的分片仍可用；缺片时保留整包并报告。存在兼容的分片索引时，缺片通常由 Cargo 补齐，不会自动改用旧整包；找不到兼容的分片索引时才可能走整包迁移回退。

删除旧整包前，还须确认它在当前与仍需支持的工具链、架构、分支及兼容版本中确实不再需要，且替代索引不旧于它。不同编译环境的缓存身份可能不同，不能仅按年龄、组名或 Actions 序号断定重复。只对候选替代分片计算完整 SHA256，避免每次无差别重读大型缓存；无明确替代时保留。

### 清理与复核

只清理由当前任务授权、经核对明确废弃且可重建的缓存；不要删除正在使用的缓存、未知文件、历史记录、附件或用户数据。确认无相关 Actions 上传后，按[服务维护步骤](cache-server.md#首次启用和服务升级)处理授权与备份，停止服务后在同一维护窗口逐条复核并删除符合条件的归档及对应索引记录。SQLite 事务与文件删除不能整体原子提交；任一步失败立即停止，先核对实际文件和索引、记录未完成项，再恢复服务。完成后复查服务、索引、文件对应关系与释放空间；不为清理扩大服务写权限或开放远程命令接口。

容量、目录、类别或协议策略变化时，更新仓库中的 README 模板，由可信 `main` Actions 同步；日常条目淘汰只需记录结论，不往模板堆文件清单。维护若涉及代码或工作流变更，重新执行受影响链路及同提交验证，再完成 loop 收尾。
