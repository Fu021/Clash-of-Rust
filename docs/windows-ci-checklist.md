# Windows CI 测试清单

Windows x64 与 ARM64 使用原生 Release 构建，分别运行格式检查、Clippy、编译和 Rust 常规测试。构建工具、产物协议及 PE 图标检查保留。

原生基础检查通过真实内核验证系统代理写入与 HTTP 转发、TUN 接管无显式代理的请求、关闭代理/TUN 后停止内核与恢复设置。仅允许在一次性的 Actions runner 上运行，使用 runner 的管理员权限；人工 UAC 确认仍需本地验收。

Publish Release 复用成功 CI 二进制，安装最终 NSIS 包并核对版本、资源和文件校验值，运行同一套基础代理/TUN 检查后卸载，确认保留无关用户文件；安装包须通过 Defender 扫描。旧权限、迁移、失败恢复及专项 GUI 场景不再自动执行，相关脚本保留用于按需排查。

`windows-ci-report-{arch}` 记录原生基础检查结果与耗时；编译失败以步骤日志为准。来源提交、Cargo.lock、平台、架构及逐文件校验清单不匹配时禁止打包。详见 [工作流说明](workflows.md)。
