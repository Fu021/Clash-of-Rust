# 订阅、节点与规则

[CLI 入口](../cli.md)。下表命令省略 `cor` 前缀，通用参数和退出码见入口文档。

## 订阅

| 命令 | 作用 |
| --- | --- |
| `profile list [--filter TEXT --all-users]` | 当前用户列表；Linux root 可只读查看[跨用户总览](linux-users.md#root-只读总览) |
| `profile add NAME URL_OR_FILE` | 下载或导入 YAML 并校验 |
| `profile import FILE --name NAME` | 文件导入；`FILE` 可为 `-`，从 stdin 读取 |
| `profile update [PROFILE] [--all]` | 更新当前/指定/全部非内置订阅 |
| `profile use PROFILE` | 切换当前订阅 |
| `profile edit PROFILE [--name NAME] [--source SOURCE]` | 修改元数据；来源变更后执行 update 下载 |
| `profile remove PROFILE --yes` | 删除；当前订阅会切回默认直连 |
| `profile export PROFILE [-o FILE]` | 输出缓存 YAML，包含凭据的完整导出须加 `--show-secrets` |
| `profile directory` | 输出订阅缓存目录 |

```sh
cat subscription.yaml | cor profile import - --name 本地订阅
# 从标准输入导入 YAML，并命名为“本地订阅”。

cor profile export 本地订阅 --show-secrets -o backup.yaml
# 导出完整订阅到 backup.yaml，包含节点凭据，请妥善保存。
```

stdin 导入保存在数据目录 `imports/`，作为可再次导入的本机来源；删除订阅不会删除用户原文件。端口、密钥和运行目录仍由客户端覆盖，不直接信任 YAML 的控制接口设置。

订阅更新的下载或校验失败时[保留原订阅和运行状态](../development.md#修改前后都要检查)。`profile update --all` 继续更新其余订阅，逐项输出结果；有失败项则返回非零退出码，成功项正常保存。

## 节点与规则

| 命令 | 作用 |
| --- | --- |
| `node list [GROUP] [--filter TEXT --sort name\|latency --reverse]` | 列出节点/策略组；默认自然名称排序 |
| `node select GROUP NODE` | 切换节点；保护期间禁止切换 |
| `node test NODE` | 测试单个节点/策略组 |
| `node test --group GROUP` | 单策略组测速 |
| `node test --all` | 全量测速，自动策略组按共享逻辑重新选优 |
| `rule list [--profile PROFILE --filter TEXT]` | 规则文本、来源、启用状态和序号；JSON 包含稳定 ID |
| `rule add RAW [--position N --disabled]` | 添加自定义规则，默认第一条 |
| `rule edit RULE RAW` | 修改自定义规则 |
| `rule remove RULE --yes` | 删除自定义规则 |
| `rule move RULE POSITION` | 调整顺序，位置从 1 开始 |
| `rule enable RULE on\|off` | 自定义规则开关 |
| `rule shortcut [NAME] [on\|off]` | 查看或开关快捷规则 |

规则命令均支持 `--profile`，默认操作当前订阅。`RULE` 可用稳定 ID 或当下列表序号；脚本建议用稳定 ID。订阅原规则只读，可调整顺序；快捷规则使用专用开关。原始 YAML 不会被重写，保存时先用 mihomo 校验，再通过事务提交。并发 CLI 修改在后台锁内基于最新状态执行。

快捷规则名称依次为 `ads`、`private-domain`、`private-ip`、`cn-domain`、`cn-ip`，对应现有五项规则，仅补充缺少的规则，不重复添加，默认全关闭。

```sh
cor rule add 'DOMAIN-SUFFIX,example.com,DIRECT' --position 1
# 在当前订阅的第 1 条添加规则，让 example.com 及其子域名直连。

cor rule shortcut ads on
# 开启广告拦截快捷规则，仅补充订阅中缺少的规则。

cor rule list --json
# 以 JSON 查看完整规则和稳定 ID，适合脚本或精确定位规则。

cor rule move c:完整UUID 1
# 将指定自定义规则移动到第 1 条；用上一命令返回的实际 ID 替换占位内容。
```

节点、连接和规则列表支持 `--limit`（默认 100，最大 10000）、`--offset`，先筛选和排序再分页。
