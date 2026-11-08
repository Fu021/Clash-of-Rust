# 预装 VM v1：元数据与共用测试向量

[接口、认证与容量](prepared-vm-protocol.md) · [模板及测速](prepared-vm-images.md) · [机器可读向量](fixtures/prepared-vm-v1.json)

本页细化 2026-11-08 对接约定，客户端校验器已在 `scripts/prepared_vm.py` 实现，现有 `test-build-tools.py` 直接消费以下共用向量；服务端仍待实现与核对。双方不能分别改预期以迁就实现，修改契约时同步本页及向量。向量全是假数据，只测试元数据规则，不用于下载、上传或证明真实文件存在。

## 通用规则

请求为 UTF-8 JSON 对象，编码后不超过 1 MiB。以下对象字段均为必填且不接受额外字段。重复 JSON 键在解析阶段拒绝；不接受 null、浮点数、NaN/Infinity 或字符串与整数间转换。整数使用严格 JSON 整数类型，布尔值不能当整数；非容量数值上限均为 `9007199254740991`。字符串不剥离空格、不转大小写、不做 Unicode 归一化，不接受控制字符。

`sha256` 类型是 64 位小写十六进制字符串。`canonical(value)` 固定为：

```python
json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True,
           allow_nan=False).encode('ascii')
```

没有 BOM、末尾换行或其他空白。对象字段次序不影响摘要；数组顺序有意义。先按以下规则验证和排序，再计算摘要；服务端收到未排序数组时拒绝，不静默排序修复请求。十六进制摘要取上述字节的 SHA256。

## 顶层与 descriptor

| 字段 | 确切类型及规则 |
| --- | --- |
| `schema` | 严格整数 `1` |
| `group` | 字符串 `"vm/prepared"` |
| `env_key` | 字符串 `"env1-" + sha256(canonical(descriptor))`，服务端重算 |
| `descriptor` | 下表对象 |
| `image` | 下一节对象 |
| `environment` | 实际环境对象 |
| `producer` | 追踪对象 |

| descriptor 字段 | 确切类型及规则 |
| --- | --- |
| `schema` | 严格整数 `1` |
| `kind` | 字符串 `"ubuntu-compat"` |
| `distribution` | 字符串 `"ubuntu"` |
| `release` | 字符串 `"20.04"` |
| `arch` | 首版仅字符串 `"arm64"`；其他架构须后续显式放行 |
| `kernel_series` | 字符串 `"5.4"` |
| `base_sha256` | `sha256`，官方基础镜像原文件 |
| `python_sha256` | `sha256`，独立 Python 原文件 |
| `environment_lock_sha256` | `sha256`，独立环境依赖锁 |
| `prepare_recipe_sha256` | `sha256`，独立环境准备配方 |

`descriptor` 只定义通用环境身份。依赖锁及配方自身如何生成由客户端冻结逻辑负责，服务端不能凭这些摘要就宣称实际磁盘已符合锁；客户端必须逐项核对。

## image

| 字段 | 确切类型及规则 |
| --- | --- |
| `format` | 字符串 `"qcow2"` |
| `size` | 严格整数，`1..8589934592`，实际完整文件长度 |
| `virtual_size` | 严格整数，`512..34359738368`，512 的倍数，须与头部相同 |
| `sha256` | `sha256`，完整 qcow2 文件字节 |

文件内容及 qcow2 头部检查属于[接口约定](prepared-vm-protocol.md#qcow2-准入检查)中的另一层，元数据向量不替代它。

## environment

| 字段 | 确切类型及规则 |
| --- | --- |
| `kernel_release` | ASCII 字符串，1～128 字符；匹配 `5\.4\.[0-9][A-Za-z0-9.+_~-]*`，实际启动的内核版本 |
| `python_version` | ASCII 字符串，1～32 字符；匹配 `[0-9]+\.[0-9]+\.[0-9]+`，实际独立 Python 的版本 |
| `packages` | 1～4096 项安装包对象数组，规则如下 |
| `packages_sha256` | `sha256(canonical(packages))`，只覆盖数组，不把此字段自身算入 |
| `sources` | 2～4096 项公开原包来源对象数组，规则如下 |

安装包对象恰好为 `{name, version, architecture}`：

| 字段 | 确切类型及规则 |
| --- | --- |
| `name` | ASCII 字符串，2～128 字符；匹配 `[a-z0-9][a-z0-9+.-]+`，不带 `:arm64` 后缀 |
| `version` | ASCII 字符串，1～256 字符；匹配 `[0-9][A-Za-z0-9.+:~-]*`，保留 epoch 与 revision |
| `architecture` | 首版仅 `"arm64"` 或 `"all"` |

以上正则均匹配整个字符串，是本协议的接纳范围。包数组按 `(name, architecture)` 的 ASCII 字节升序排列；同一 `(name, architecture)` 即使版本相同也拒绝重复，不按 name 合并或只取最大版本。记录完整已安装包清单，不能只报本轮额外安装项；独立 Python 由对应字段及来源记录，不冒充 dpkg 包。

来源对象恰好为 `{url, size, sha256}`。`url` 是 1～2048 字符的 ASCII 公共 HTTPS URL，无空白、反斜线、用户名、密码、query 或 fragment；其来源还须通过现有官方来源白名单及公网校验。`size` 为正严格整数，最多 `9007199254740991`；`sha256` 为原文件摘要。按 sha256 的 ASCII 字节升序排列，摘要不重复；同内容选择一个已核验的官方源，不在此数组登记镜像别名。

来源必须包含 `base_sha256` 和 `python_sha256`，两者须不同；还要包含额外下载的全部环境原包。基础盘中原有包通过基础镜像来源追溯，不要求虚构它们的单独 DEB 下载记录。sources 自身不改变环境键；可变 URL 不能代替原文件摘要。来源解析、白名单、DNS/重定向及真实字节校验是额外验收，不由纯 JSON 向量完成。

## producer

| 字段 | 确切类型及规则 |
| --- | --- |
| `repository` | ASCII 字符串 `owner/repository`，每段 1～100 字符，匹配 `[A-Za-z0-9][A-Za-z0-9_.-]*` |
| `commit` | 40 位小写十六进制字符串，实际来源提交 |
| `run_id` | 正严格整数，最多 `9007199254740991` |
| `run_attempt` | 正严格整数，最多 `9007199254740991` |

不包含 token、宿主路径、用户名配置、额外自由文本或命令。repository 只能追踪，现有共享 Bearer 不提供按该字段授权的能力。镜像复用时返回原 producer，不以查询者的当前 run 替换。

## 响应封装

health 返回 HTTP 200，`capabilities` 数组含已实现能力；`prepared_vm` 对象包含 `architectures: ["arm64"]`、`max_image_bytes`、`max_virtual_bytes` 和 `range: false`。两个容量字段为正严格整数，不超过协议上限；虚拟容量至少覆盖 24 GiB guest。未声明或不匹配时冷回退，不进行上传。

ready 响应使用 HTTP 200，将原始请求的七个字段置于顶层，额外返回 `status: "ready"` 和 `object_id`，不再嵌套 manifest。不可变复用保留原 producer 和文件身份；实际内核、Python、包清单及来源的摘要/大小必须匹配，相同来源内容的官方 URL 别名可以不同。

201/202 及非 ready 上传查询使用顶层 `schema/group/env_key/upload_id/status`，201 另含 `expires_at`（正整数 Unix 秒）。ID 匹配 `[A-Za-z0-9_-]{1,128}`；GET 查询返回 HTTP 200，ready 时使用完整 ready 形状。错误使用 `error_code`，不依赖自由文本。blob 只返回完整内容，Content-Length 等于 image.size；客户端不跟随重定向。

## 共用向量与错误

[prepared-vm-v1.json](fixtures/prepared-vm-v1.json) 含一个 `base_request` 及 `cases`。逐条深拷贝 base，按序执行 `operations`；`path` 是字段名/数组下标组成的数组。`set` 替换已有数组项或设置对象字段，`remove` 删除已有字段或数组项。不修改原 base，不隐含重算任何摘要。

`wire` 只用于解析层边界：`duplicate_top_key` 在 JSON 编码时重复指定顶层字段，不能先解析成字典而丢掉重复；`append` 将给定文本附在完整 JSON 后。其余 case 直接序列化变换后的请求。`expected.valid` 是元数据校验结果；成功须同时比对明确给出的环境键和包清单摘要。`expected.error` 是拒绝时的固定错误码。

格式、字段、排序、包摘要不符等返回 400 / `invalid_metadata`；合法 descriptor 重算后与 env_key 不符返回 409 / `identity_conflict`；未支持架构返回 422 / `unsupported_image`，超出单文件/虚拟容量上限返回 413 / `unsupported_image`。类型检查先于数值比较与摘要核对。服务额度不足的 507、上传槽忙的 503 和文件内容错误另做状态/存储回归。

向量中的来源域名只用于离线元数据测试，不请求网络。双方还须各自验证 1 MiB/4096 项边界、重复嵌套键、无效 UTF-8/非有限数、来源策略及上传生命周期；不得以该向量集通过代替这些验收。
