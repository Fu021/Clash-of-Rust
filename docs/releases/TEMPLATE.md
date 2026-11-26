# 发布文案模板

每个正式版或预发布版都从本模板准备 `docs/releases/<完整版本>.md`，替换下列占位内容并随源码提交。文案保持简短：只写用户可感知的主要变化、重要修复和必要限制，相关改动合并为一条；不写泛泛的版本概述、下载计数解释、Assets 校验文件提示，也不罗列内部字段、测试夹具、协议细节、单次耗时或细小样式调整。只填写已经实现或已验证的行为，明确尚未验收的范围；正式版与预发布版使用相同结构。提交时尚未执行的验证写为待验证，实际结果以该提交的 Actions 记录和发布收据为准。

先写完整中文“更新”和“验证与已知问题”，再逐条翻译为英文；英文覆盖前文全部内容，不额外删减验证范围或已知限制。正式版的网站内容按[网站说明](../../website/README.md)同步，预发布不单独更新网站。

```markdown
# <完整版本>

## 更新

- <面向用户的主要变化、修复和必要的迁移说明。>

## 验证与已知问题

- <同提交 CI、打包或平台验证的范围；尚未执行的检查和已知限制。>

## Updates

- <逐条翻译上面的更新。>

## Verification and known issues

- <逐条翻译上面的验证范围及已知问题。>
```

发布程序在校验同提交的版本文案后，按 README 的 Windows、Linux、macOS、Android 顺序生成下载区。下载按钮使用[来源提交中的静态 SVG](badges/SOURCE.md)，下载量徽章继续由 Shields.io 提供。两枚徽章都链接到本版标签的对应安装包。Android ARM64 下载量徽章后注明“（多数手机和平板）”。完整下载区放在版本标题之后、更新详情之前，避免 Releases 列表截断链接。填写文案时只写更新和验证内容，由发布程序生成安装包地址。仅测试的 `check_only` 保持现有 Release。

Linux 的徽章随发布范围生成。预览时对照 [`linux_scope`](../workflows/release.md#兼容性发布门禁)，检查本版的 RPM 架构和 Arch x64 原生包都已出现在下载表中。

## 徽章效果示例

以下是 `v0.7.3` 的一组徽章。完整下载表见 [v0.7.3 正式版](https://github.com/Fu021/Clash-of-Rust/releases/tag/v0.7.3)。

[![Linux Arch pkg x64](badges/linux-arch-x64.svg)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.pkg.tar.zst) [![Linux Arch x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.pkg.tar.zst?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.7.3/Clash-of-Rust-0.7.3-linux-x86_64.pkg.tar.zst)
