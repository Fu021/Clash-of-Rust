# 私网缓存说明与交接

[返回 CI 与缓存](ci.md)。本页供维护者和后续 agent 按需查阅；贡献者不需要服务器账号或缓存凭据。

## 日常更新

缓存目录、数据类别、保留策略或服务协议发生变化时，同步修改仓库中的 [README 模板](../../.github/cache-server/README.txt)。`main` 推送的 CI 在缓存清单阶段，通过现有 Tailscale 连接和认证接口同步模板到缓存服务器的 `state/README.txt`；服务器根目录的 `README.txt` 指向该文件。内容由 Actions 写入并回读校验，不要在服务器或 agent 所在机器上直接编辑 README。

同步前确认提交仍为远端 `main` 的当前提交；服务端按 Actions 运行序号拒绝较旧内容。PR、手动发布和内存基准不写入 README，避免未合并或旧版本覆盖当前说明。每次可信 `main` 推送都会尝试同步，即使模板没有变化；模板本身不得包含令牌、个人信息或私网地址。

查看 CI 的 **Actions cache budget → Synchronize the private cache README from main**：成功日志包含 `synchronized and verified`。未配置连接、连接失败或同步失败时日志和 job summary 会提示，缓存仍属于可选加速，不阻塞应用编译；不得把有提示的运行当作 README 已同步。修复后可重新运行该 CI 的缓存预算 job，或随下一次 `main` 推送重试。查找 job 的 `databaseId` 时使用 `gh run view <运行号> --json jobs`，不要从浏览器 URL 猜测 ID。

该同步使用仓库变量 `LOCAL_CACHE_URL`、`TS_OAUTH_CLIENT_ID`、`TS_AUDIENCE` 和机密 `LOCAL_CACHE_TOKEN`，沿用现有缓存权限。外部 fork 不得索取或使用维护者凭据。服务只接受固定的 README 路径、认证请求、最多 64 KiB 的 UTF-8 内容以及有效的 SHA256；中断、错误校验值和旧运行不覆盖现有内容。

## 首次启用和服务升级

已有服务需要**一次性**部署支持 `/v1/readme` 的 `scripts/action_cache.py`；仅修改模板无需再次部署。以下操作由缓存服务器维护者在该主机上执行，先确认没有正在上传缓存的 Actions；它们安装服务和设置链接，README 内容仍由 Actions 写入。部署前保存旧程序和原 README，勿输出配置或令牌：

1. 在任意可信位置检出已审核、已通过 CI 的 `main`，确认 `scripts/action_cache.py` 为目标版本。找到现有服务实际使用的缓存根目录和 systemd user unit；不要假设 agent 的工作目录与缓存主机相同。
2. 停止 `github-action-cache` 用户服务，备份原 `bin/action_cache.py`，将目标脚本以所有者可执行、其他用户不可访问的权限安装到原位置，再启动服务。保持原 `config/server.json`、缓存数据和索引；不要扩大服务可写范围或开放 SSH/命令执行接口。
3. 从当前 `main` 的 CI 重新运行缓存预算 job，确认同步和回读通过，且缓存根目录下的 `state/README.txt` 已出现。首次启用前的旧服务返回 404，属于尚未部署，不能当成同步成功。
4. 在缓存根目录为原有 `README.txt` 保留一个私有备份，然后让 `README.txt` 相对链接到 `state/README.txt`；确认读取根目录链接得到工作流生成的内容。不要复制模板、令牌或旧 README 到公开位置。

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

不要覆盖既有备份。已建立正确链接的服务器跳过该链接步骤。

新主机还须由维护者创建私有的 `config/server.json` 和受限的 systemd user unit，并配置同一 tailnet、Actions 变量和机密。配置字段为 `bind`、`port`、`token_sha256`、`budget_gib` 和 `min_free_gib`；服务端只存访问令牌的 SHA256，Actions 的 `LOCAL_CACHE_TOKEN` 保存对应原文，均不得写入模板或日志。仓库提供[现有隔离策略的 unit 模板](../../.github/cache-server/github-action-cache.service)，以 `%h/github_action_cache` 为根；迁移到其他根目录时须调整所有绑定路径并确认隔离仍有效。启动参数用 `serve --root <缓存根目录>` 显式指定路径，不依赖脚本的历史默认目录。

服务仅绑定 loopback 或 Tailscale 地址；systemd 应只允许 `data/`、`state/`、`logs/` 写入，脚本与配置只读，不能暴露其他用户目录。`state/README.txt` 在该范围内原子替换，根链接只需安装一次。主机迁移时先部署服务及链接，再用当前 `main` 的 CI 验证；后续模板更新只走 Actions。旧服务没有执行命令或自更新接口，因此首次服务部署不能由旧接口代劳。

首次建立空服务时，以服务用户创建私有的 `bin/`、`config/`、`data/`、`state/`、`logs/`，安装 `action_cache.py` 和经检查的 unit；配置文件设为仅所有者可读，程序仅所有者可执行。将 unit 安装到该用户的 systemd 目录后执行 `systemctl --user daemon-reload` 和 `systemctl --user enable --now github-action-cache`。新根目录可预先建立相对链接 `README.txt -> state/README.txt`，首次同步前链接暂时无目标；验证服务存活和 Actions 回读后才视为部署完成。

若服务升级失败，停止服务并恢复事先备份的程序，再启动并确认原缓存可用；保留原数据、索引和 README 备份。不要通过仓库外文件编辑、重建缓存或降低鉴权来绕过同步失败。维护者应核对 service 日志中的错误类型，不公开配置、认证头或敏感路径。
