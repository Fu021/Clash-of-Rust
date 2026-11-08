# 发布文案模板

每个正式版或预发布版都从本模板准备 `docs/releases/<完整版本>.md`，替换下列占位内容并随源码提交。只填写已经实现或已验证的行为，明确尚未验收的范围；正式版与预发布版使用相同结构。提交时尚未执行的验证写为待验证，实际结果以该提交的 Actions 记录和发布收据为准。

```markdown
# <完整版本>

<一句话概括本次版本和适用范围。>

## 更新

- <面向用户的主要变化、修复和必要的迁移说明。>

## 验证与已知问题

- <同提交 CI、打包或平台验证的范围；尚未执行的检查和已知限制。>
```

发布程序在校验同提交的版本文案后，自动生成 Android、Windows、macOS 和 Linux 的下载徽章与每个安装包的下载量徽章。下载地址固定到本版标签和附件，预发布也不会转到稳定版 `latest`；不在文案中手写会过期的安装包地址。SHA256 文件随安装包上传到 Release 附件。仅测试的 `check_only` 不创建或修改 Release。

## 徽章效果示例

以下使用已发布的 `v0.6.6` 展示实际样式，打开 Markdown 预览即可查看。准备版本文案时只复制上方代码块，不复制本节；发布程序会按本次版本生成完整下载表，预发布也使用对应的版本标签和下载量。

| 平台 | 安装包与下载次数 |
| --- | --- |
| Android | [![Android APK ARM64](https://img.shields.io/badge/APK-ARM64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk) [![Android ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-arm64.apk)<br>[![Android APK ARMv7](https://img.shields.io/badge/APK-ARMv7-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk) [![Android ARMv7 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-armv7.apk)<br>[![Android APK x64](https://img.shields.io/badge/APK-x64-3DDC84?logo=android&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk) [![Android x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-android-x64.apk) |
| Windows | [![Windows Setup x64](https://img.shields.io/badge/Setup-x64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe) [![Windows x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-x64-setup.exe)<br>[![Windows Setup ARM64](https://img.shields.io/badge/Setup-ARM64-0078D4)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe) [![Windows ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-windows-arm64-setup.exe) |
| macOS | [![macOS DMG Apple Silicon](https://img.shields.io/badge/DMG-Apple_Silicon-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg) [![macOS Apple Silicon downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-arm64.dmg)<br>[![macOS DMG Intel x64](https://img.shields.io/badge/DMG-Intel_x64-555555?logo=apple&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg) [![macOS Intel x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-macos-x64.dmg) |
| Linux | [![Linux DEB x64](https://img.shields.io/badge/DEB-x64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb) [![Linux x64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-amd64.deb)<br>[![Linux DEB ARM64](https://img.shields.io/badge/DEB-ARM64-E95420?logo=linux&logoColor=white)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb) [![Linux ARM64 downloads](https://img.shields.io/github/downloads/Fu021/Clash-of-Rust/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb?label=downloads&displayAssetName=false)](https://github.com/Fu021/Clash-of-Rust/releases/download/v0.6.6/Clash-of-Rust-0.6.6-linux-arm64.deb) |
