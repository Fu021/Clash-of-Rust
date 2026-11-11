# 发布文案模板

每个正式版或预发布版都从本模板准备 `docs/releases/<完整版本>.md`，替换下列占位内容并随源码提交。文案保持简短：只写用户可感知的主要变化、重要修复和必要限制，相关改动合并为一条；不写泛泛的版本概述、下载计数解释、Assets 校验文件提示，也不罗列内部字段、测试夹具、协议细节、单次耗时或细小样式调整。只填写已经实现或已验证的行为，明确尚未验收的范围；正式版与预发布版使用相同结构。提交时尚未执行的验证写为待验证，实际结果以该提交的 Actions 记录和发布收据为准。

```markdown
# <完整版本>

## 更新

- <面向用户的主要变化、修复和必要的迁移说明。>

## 验证与已知问题

- <同提交 CI、打包或平台验证的范围；尚未执行的检查和已知限制。>
```

发布程序在校验同提交的版本文案后，按 README 的 Windows、Linux、macOS、Android 顺序生成下载徽章与下载量徽章；Android ARM64 下载量徽章后注明“（多数手机和平板）”。完整下载区放在版本标题之后、更新详情之前，避免 Releases 列表截断链接。地址与计数固定到本版标签和附件，不手写安装包地址，不追加说明性套话。仅测试的 `check_only` 不创建或修改 Release。

## 徽章效果示例

以下使用已发布的 `v0.6.6` 展示实际样式，打开 Markdown 预览即可查看。准备版本文案时只复制上方代码块，不复制本节；发布程序会按本次版本生成完整下载表，预发布也使用对应的版本标签和下载量。

| 平台 | 安装包与下载次数 |
| --- | --- |
| Windows | [![Windows Setup x64](https://img.shields.io/badge/Setup-x64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe) [![Windows x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe)<br>[![Windows Setup ARM64](https://img.shields.io/badge/Setup-ARM64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe) [![Windows ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe) |
| Linux | [![Linux DEB x64](https://img.shields.io/badge/DEB-x64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb) [![Linux x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb)<br>[![Linux DEB ARM64](https://img.shields.io/badge/DEB-ARM64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb) [![Linux ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb) |
| macOS | [![macOS DMG Apple Silicon](https://img.shields.io/badge/DMG-Apple_Silicon-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg) [![macOS Apple Silicon downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg)<br>[![macOS DMG Intel x64](https://img.shields.io/badge/DMG-Intel_x64-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg) [![macOS Intel x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg) |
| Android | [![Android APK ARM64](https://img.shields.io/badge/APK-ARM64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk) [![Android ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk) （多数手机和平板）<br>[![Android APK ARMv7](https://img.shields.io/badge/APK-ARMv7-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk) [![Android ARMv7 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk)<br>[![Android APK x64](https://img.shields.io/badge/APK-x64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk) [![Android x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk) |
