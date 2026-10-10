# 验证记录

- [工作流首轮四平台 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/37971395782)：全部通过；基础工作流已进入 main。
- [应用修复四平台 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/37973026642)：全部通过，源码提交 `30bee58f05cdacd61e250c481bb302a19f922bd8`。包含 Release profile 的 Clippy、全部 target 测试、真实 mihomo 集成、平台原生接口测试和原始编译产物导出。
- Linux x64 默认 Rust 测试 102 通过、16 忽略；另显式执行六个真实内核用例、三个原生库用例和一个原生托盘用例，均通过。原生 Linux TUN 安装测试归发布工作流。
- Python 构建与产物工具共 28 项：Linux 27 通过、1 个 Windows 专用项跳过；该项在 Windows CI 中另行执行。181 个平台函数的生成代码一致性检查通过。
- [修改前云端内存测试](https://github.com/Fu021/Clash-of-Rust/actions/runs/37971832437)与[修改后云端内存测试](https://github.com/Fu021/Clash-of-Rust/actions/runs/37972894968)：全部通过；截图确认 `node-` 全匹配搜索生效，新界面有全局分页。工具链、输入、参数、内核和 Geo 文件哈希均相同。
- 修改后基准的应用提交为 `a17796b165f5d78454308d8acc2ea4ba410c2647`；随后 `30bee58` 只修正回归测试的 Clippy 写法，应用源码与基准程序没有变化。后续内存报告提交只更新文档与原始记录。
- [最新 PR 四平台 CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/37975568795)：全部通过，包含已同步到主分支的工作流修复；应用源码没有改变。
- [工作流修复后的 main CI](https://github.com/Fu021/Clash-of-Rust/actions/runs/37975500643)：全部通过；CI 仅 main push 和 PR 触发，内存任务支持短 SHA。
- [Publish Release 完整流程验证](https://github.com/Fu021/Clash-of-Rust/actions/runs/37976055650)：使用已验证的 main / 0.4.12 原始编译产物，`check_only=true`。四平台最终安装包、安装后的程序、Windows Defender、Ubuntu 最新版 GUI/TUN 兼容及八个附件的来源/校验验证全部通过。没有创建或修改 Release。
- 首次发布流程验证停在最新版 Ubuntu 的 Azure APT 镜像下载，未进入兼容测试；取消后提取日志定位，并改用官方 HTTPS 镜像、限制重试与等待。修复后上述完整重跑成功。

应用修复在独立分支 `codex/memory-fixes`，尚未合并 main。新版安装包测试将在合并后使用新版 main CI 产物进行；上述发布流程验证用于确认工作流改造，不能视为已经测试了 0.4.13-dev.1 的最终安装包。

配置事务覆盖进程中断后的恢复，不宣称文件系统突然断电时的全局原子性。平台检测的常量借用、正文缓冲和共享客户端有回归测试；本次 API/GUI 基准没有量化互联网平台检测的内存收益。
