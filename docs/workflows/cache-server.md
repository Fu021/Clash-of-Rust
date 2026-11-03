# 私网缓存说明与交接

[返回 CI 与缓存](ci.md)。本页供维护者和后续 agent 按需查阅；贡献者不需要服务器账号或缓存凭据。

## 换服务器接手

先检查当前仓库分支、提交、未提交文件、运行环境和 [AGENTS](../../AGENTS.md)，再按任务读取本文与[缓存盘点](cache-maintenance.md)。不要假设新 agent 主机就是缓存主机，也不要沿用先前 agent 的 `/tmp` 脚本、私有路径或环境变量。仓库内的缓存客户端、工作流和 README 模板可以在任意检出中修改；日常清单可从已授权的 Actions 查看。

服务程序、systemd unit 和磁盘数据只存在于实际缓存主机：修改源码并通过 CI 不等于服务已部署。需要升级或迁移时，在维护者授权的主机与服务用户上执行下文步骤；只有 GitHub 访问权而无主机维护权限时记录阻塞，不开新远程命令接口、不索取缓存令牌或绕过隔离。README 内容始终由 Actions 同步。

| 要修改的内容 | 仓库入口 |
| --- | --- |
| GitHub 缓存预算、保留或清理次序 | `scripts/workflow_storage.py` 和 `scripts/test-build-tools.py`；核对三条工作流的预算任务。 |
| 私网缓存协议、分片或服务 | `scripts/action_cache.py`、`scripts/cache_shards.py`、`.github/actions/local-cache/` 和对应测试；检查旧服务兼容与部署。 |
| 服务器目录、数据类别、保留策略或协议说明 | 修改 [.github/cache-server/README.txt](../../.github/cache-server/README.txt)，由可信 `main` CI 同步；不要直接编辑服务器 README。 |

服务主机上可由所属用户执行 `systemctl --user show github-action-cache -p FragmentPath -p ExecStart -p ReadWritePaths` 确认实际 unit、根目录与权限；服务名可能不同，输出仅在本地查看并在共享前脱敏。服务主机没有仓库时，通过已授权的维护渠道提供已审核提交及文件 SHA256，核对后再部署，不复制配置、令牌或临时缓存。

## 日常更新

缓存目录、数据类别、保留策略或服务协议发生变化时，同步修改仓库中的 [README 模板](../../.github/cache-server/README.txt)。`main` 推送的 CI 在缓存清单阶段，通过现有 Tailscale 连接和认证接口同步模板到缓存服务器的 `state/README.txt`；服务器根目录的 `README.txt` 指向该文件。内容由 Actions 写入并回读校验，不要在服务器或 agent 所在机器上直接编辑 README。

同步前确认提交仍为远端 `main` 的当前提交；服务端按 Actions 运行序号拒绝较旧内容。PR、手动发布和内存基准不写入 README，避免未合并或旧版本覆盖当前说明。每次可信 `main` 推送都会尝试同步，即使模板没有变化；模板本身不得包含令牌、个人信息或私网地址。

纯文档变更按 [AGENTS](../../AGENTS.md#actions提交与发布)留在本地，随下次功能更新推送；其间服务器仍显示远端 `main` 的已提交模板。核对服务内容时以该提交为基准，不把未提交的本地模板当成已同步，也不为纯文档单独触发 CI。

查看 CI 的 **Actions cache budget → Synchronize the private cache README from main**：成功日志包含 `synchronized and verified`。未配置连接、连接失败或同步失败时日志和 job summary 会提示，缓存仍属于可选加速，不阻塞应用编译；不得把有提示的运行当作 README 已同步。修复后可重新运行该 CI 的缓存预算 job，或随下一次 `main` 推送重试。查找 job 的 `databaseId` 时使用 `gh run view <运行号> --json jobs`，不要从浏览器 URL 猜测 ID。

该同步使用仓库变量 `LOCAL_CACHE_URL`、`TS_OAUTH_CLIENT_ID`、`TS_AUDIENCE` 和机密 `LOCAL_CACHE_TOKEN`，沿用现有缓存权限。外部 fork 不得索取或使用维护者凭据。服务只接受固定的 README 路径、认证请求、最多 64 KiB 的 UTF-8 内容以及有效的 SHA256；中断、错误校验值和旧运行不覆盖现有内容。

## 首次启用和服务升级

已有服务需要**一次性**部署支持 `/v1/readme` 的 `scripts/action_cache.py`；仅修改模板无需再次部署。以下操作由缓存服务器维护者在该主机上执行，先确认相关 Actions 已结束；它们安装服务和设置链接，README 内容仍由 Actions 写入。默认保存旧程序和原 README；维护者明确要求不备份时省略备份步骤，不能假称可回滚。勿输出配置或令牌：

1. 在任意可信位置检出已审核、已通过 CI 的 `main`，确认 `scripts/action_cache.py` 为目标版本。找到现有服务实际使用的缓存根目录和 systemd user unit；不要假设 agent 的工作目录与缓存主机相同。
2. 停止 `github-action-cache` 用户服务，按授权决定备份，将目标脚本以所有者可执行、其他用户不可访问的权限安装到原位置，再启动服务。更新 unit 时先解析已有符号链接，更新实际文件并保留链接，再执行 `systemctl --user daemon-reload`。保持原 `config/server.json`、缓存数据和索引；不要扩大服务可写范围或开放 SSH/命令执行接口。
3. 从当前 `main` 的 CI 重新运行缓存预算 job，确认同步和回读通过，且缓存根目录下的 `state/README.txt` 已出现。首次启用前的旧服务返回 404，属于尚未部署，不能当成同步成功。
4. 按授权决定是否备份原 `README.txt`，然后让它相对链接到 `state/README.txt`；确认读取根目录链接得到工作流生成的内容。不要复制模板、令牌或旧 README 到公开位置。

例如，根目录位于当前维护者的 `~/github_action_cache`，服务名为 `github-action-cache`，且脚本备份名尚未使用时，在已审核的仓库目录执行：

```sh
(
  set -eu
  CACHE_ROOT="$HOME/github_action_cache"
  test -f "$CACHE_ROOT/bin/action_cache.py"
  for path in "$CACHE_ROOT/bin/action_cache.py.pre-readme" "$CACHE_ROOT/bin/action_cache.py.pending"; do
    test ! -e "$path"
    test ! -L "$path"
  done
  cp -p "$CACHE_ROOT/bin/action_cache.py" "$CACHE_ROOT/bin/action_cache.py.pre-readme"
  install -m 0700 scripts/action_cache.py "$CACHE_ROOT/bin/action_cache.py.pending"
  systemctl --user stop github-action-cache
  mv "$CACHE_ROOT/bin/action_cache.py.pending" "$CACHE_ROOT/bin/action_cache.py"
  systemctl --user start github-action-cache
  systemctl --user status github-action-cache --no-pager
)
```

确认 Actions 已生成 `state/README.txt` 后，再备份原 README 并建立链接。以下示例要求原 README 是普通文件、备份和临时链接不存在；任一检查失败时停止并调查：

```sh
(
  set -eu
  CACHE_ROOT="$HOME/github_action_cache"
  test -f "$CACHE_ROOT/state/README.txt"
  test -f "$CACHE_ROOT/README.txt"
  test ! -L "$CACHE_ROOT/README.txt"
  for path in "$CACHE_ROOT/README.pre-actions.txt" "$CACHE_ROOT/.README.next"; do
    test ! -e "$path"
    test ! -L "$path"
  done
  cp -p "$CACHE_ROOT/README.txt" "$CACHE_ROOT/README.pre-actions.txt"
  ln -s state/README.txt "$CACHE_ROOT/.README.next"
  mv -Tf "$CACHE_ROOT/.README.next" "$CACHE_ROOT/README.txt"
  readlink "$CACHE_ROOT/README.txt"
)
```

示例默认备份；维护者明确要求不备份时省略相应复制和备份名检查。不要覆盖或删除既有备份；已建立正确链接的服务器跳过该链接步骤。

新主机还须由维护者创建私有的 `config/server.json` 和受限的 systemd user unit，并配置同一 tailnet、Actions 变量和机密。配置字段为 `bind`、`port`、`token_sha256`、`budget_gib` 和 `min_free_gib`；服务端只存访问令牌的 SHA256，Actions 的 `LOCAL_CACHE_TOKEN` 保存对应原文，均不得写入模板或日志。仓库提供[现有隔离策略的 unit 模板](../../.github/cache-server/github-action-cache.service)，以 `%h/github_action_cache` 为根；迁移到其他根目录时须调整所有绑定路径并确认隔离仍有效。启动参数用 `serve --root <缓存根目录>` 显式指定路径，不依赖脚本的历史默认目录。

服务仅绑定 loopback 或 Tailscale 地址；systemd 应只允许 `data/`、`state/`、`logs/` 写入，脚本与配置只读，不能暴露其他用户目录。`state/README.txt` 在该范围内原子替换，根链接只需安装一次。主机迁移时先部署服务及链接，再用当前 `main` 的 CI 验证；后续模板更新只走 Actions。旧服务没有执行命令或自更新接口，因此首次服务部署不能由旧接口代劳。

首次建立空服务时，以服务用户创建私有的 `bin/`、`config/`、`data/`、`state/`、`logs/`，安装 `action_cache.py` 和经检查的 unit；配置文件设为仅所有者可读，程序仅所有者可执行。将 unit 安装到该用户的 systemd 目录后执行 `systemctl --user daemon-reload` 和 `systemctl --user enable --now github-action-cache`。新根目录可预先建立相对链接 `README.txt -> state/README.txt`，首次同步前链接暂时无目标；验证服务存活和 Actions 回读后才视为部署完成。

若服务升级失败，有备份时停止服务并恢复原程序；未备份时保留原数据和索引，用已审核源码修复部署。重新确认服务健康及未认证请求被拒绝，再通过 Actions 验证同步。不要直接编辑 README、重建缓存或降低鉴权来绕过失败；不公开配置、认证头或敏感路径。

## 缓存检查与清理

GitHub 与私网缓存的盘点、分片检查、安全清理和权限不足时的处理统一见[缓存维护](cache-maintenance.md)；loop 收尾也执行这一流程。
