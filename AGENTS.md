# 本仓库的开发约定

- 当前本地环境只有 1 核、1 GiB 内存。所有 Rust 编译、Clippy、单元测试、集成测试、内存基准和安装包构建都通过 GitHub Actions 执行；本地不运行 `cargo build/check/test/clippy/run` 或 `rustc` 构建程序。
- 本地只进行代码阅读、编辑、格式与静态语法检查。UI 截图使用 `scripts/preview-slint.py` 和持久安装的 Slint Viewer，直接预览正式 `.slint` 组件，不编译桌面程序；截图在回复中展示。
- 长期工作流只保留 CI、Publish Release、Memory Benchmark。常规 CI 保留有价值的源码单元测试；真实系统集成测试只覆盖系统代理与 TUN。弃用工作流既要移除文件，也要在 GitHub 上禁用。
- 打包复用对应提交成功 CI 的已验证产物，测试包使用 Publish Release 的 `check_only`；未获明确授权不得合并 main 或正式发布。
- 不在仓库保留本地 `target`、打包输出、历史截图、排障日志和缓存；需要 UI 预览时才生成 `dist`。工具放在持久目录，不放 `/tmp`。
- 保留必要的资源、第三方来源与许可证，以及 Android/iOS 的共享业务与宿主占位目录。
