# Windows 检测工具的源码与构建

检测所需 Bash、GNU 工具、curl、OpenSSL 及其 DLL 从 Git for Windows 的原始安装中提取，不修改二进制。`runtime/manifest.json` 记录发行版、文件校验和及源码仓库；`runtime/package-versions.txt` 记录安装时各软件包的准确版本。工具仅在检测期间运行。

- Bash、coreutils（含 md5sum）、grep、sed、gawk、findutils（xargs）、readline、ncurses、GMP、MPFR 等包的构建配方、补丁和原始源码下载地址：[Git for Windows MSYS2-packages](https://github.com/git-for-windows/MSYS2-packages)。请选择与 package-versions.txt 对应的历史版本。
- 原生 Windows curl 及依赖：[Git for Windows MINGW-packages](https://github.com/git-for-windows/MINGW-packages)。对应包的 PKGBUILD 包含源码地址与构建步骤。
- POSIX 兼容运行库：[Git for Windows msys2-runtime](https://github.com/git-for-windows/msys2-runtime)。
- jq 固定为 1.8.1：[完整源码](https://github.com/jqlang/jq/tree/jq-1.8.1)，[构建说明](https://github.com/jqlang/jq/blob/jq-1.8.1/README.md)。

Git for Windows 提供[包构建说明](https://gitforwindows.org/package-management)。在其 SDK 中切换到对应版本的包配方，使用 `makepkg --allsource` 获取包含构建配方、补丁及原始源码的源码包，再按官方文档构建。相应组件的原始许可证安装在 runtime 目录，独立工具保留各自许可证。
