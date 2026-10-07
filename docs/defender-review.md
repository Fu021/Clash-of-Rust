# Microsoft Defender 检测复核

0.4.2 在用户电脑上被检测为 `Behavior:Win32/Impact.A!ml`，涉及安装目录中的主程序以及 NSIS 临时目录中的 `clash-shutdown.exe`。这是检测记录，尚不能据此确认具体触发操作或微软最终的分析结论。

另有用户报告：从 GitHub Release 下载 0.4.2 安装包时出现 `Trojan:Win32/Sabsik.FLA!ml`。这发生在安装程序运行之前，不能直接归因于本机已经执行了退出脚本。应分别提交原始安装包与具体被检测文件，并让微软分析安装包整体和内置文件；不能只因为检测名称包含 `!ml` 就认定安全。[微软对相关 Sabsik 检测的说明](https://www.microsoft.com/en-us/wdsi/threats/malware-encyclopedia-description?Name=Trojan%3AWin32%2FSabsik.FL.A%21ml) 没有公布具体技术细节，提交时应使用用户记录中的完整名称。

旧安装流程中的 `clash-shutdown.exe` 是主程序的同一份二进制副本。0.4.3 已移除副本执行和隐藏 PowerShell 强制终止进程的备用逻辑，改由安装程序发送退出事件，让客户端自行清理、退出。

## 提交复核

在 [Microsoft Security Intelligence](https://www.microsoft.com/en-us/wdsi/filesubmission) 选择 **Software developer**，安全产品选择实际使用的 Microsoft Defender Antivirus，分类选择 **Incorrectly detected as malware/malicious**。填写检测名称和检测时的安全情报版本，上传被检测的具体文件并说明复现过程。提交后等待微软的最终分析结果。

微软建议优先提交具体被检测文件，大量文件或整个安装包可能延迟分析。复核前保留原文件的 SHA-256、版本与原始检测信息。不要将源码检查或安装测试通过当作微软已经认可该文件的依据。

## 0.4.2 原始样本

- 版本：0.4.2。
- 主程序 SHA-256：`14b0b268f724260d93b2a102ce2c13ac7c59b6945bfe4f07c493e0cfdeef14df`。
- 安装包 SHA-256：`8d44f8a0458b628951a4ae0a8c1bdb17f55bc9c1312d47ee8513e790cc745a8d`。
- 大小：23,828,992 字节。
- Authenticode：未签名。
- 已报告的安全情报版本：`1.459.576.0`。
- 公开源码与安装包：[v0.4.2](https://github.com/Fu021/Clash-of-Rust/releases/tag/v0.4.2)。

可在提交说明中使用以下文字，并补充自己的操作步骤：

> Clash-of-Rust is an open-source GPL-3.0 Rust/Iced desktop client for the mihomo proxy core. Defender detected version 0.4.2 as Behavior:Win32/Impact.A!ml during use and during reinstallation. The installer copied the same application binary to its temporary directory as clash-shutdown.exe. The application modifies system proxy settings and can enable user-requested autostart. Its old installer shutdown path requested normal exit, with a fallback that terminated only the application's own installation and core using PowerShell. Version 0.4.3 removes this fallback and the temporary executable, using cooperative exit notification instead. Please investigate whether the reported detection is a false positive. Repository and released source: https://github.com/Fu021/Clash-of-Rust.

下载时出现 Sabsik 检测的安装包，可另附以下说明：

> Another user reports Trojan:Win32/Sabsik.FLA!ml while downloading the 0.4.2 installer from our GitHub Release, before running it. Installer SHA-256: 8d44f8a0458b628951a4ae0a8c1bdb17f55bc9c1312d47ee8513e790cc745a8d. It is an unsigned NSIS installer bundling the Rust application, mihomo proxy core, Geo data and a Git-for-Windows/MSYS runtime for the open-source RegionRestrictionCheck scripts. Please analyze the original installer and identify which embedded component or characteristic causes the classification. The detection name is transcribed from the user's report; we have not confirmed a final false-positive determination.

签名能帮助验证发布者和文件完整性，但不能保证消除行为检测。微软说明见 [Software developer FAQ](https://learn.microsoft.com/en-us/defender-xdr/developer-faq)。

## 本机扫描记录（2026-10-07）

使用本机 Defender 平台 `4.18.26080.4-0`、安全情报 `1.459.576.0`，对原始 0.4.2 安装包、0.4.3 安装包和 0.4.3 主程序执行自定义扫描，输出均为未检出威胁，返回码为 0。

扫描使用 `MpCmdRun -Scan -ScanType 3 -File ... -DisableRemediation`，只分析文件，不执行程序、不删除原始样本。该扫描模式会忽略文件排除项并扫描压缩包，详情见 [微软命令行文档](https://learn.microsoft.com/en-us/defender-endpoint/command-line-arguments-microsoft-defender-antivirus)。

**扫描时本机实时保护为关闭状态；这里记录的是显式文件扫描结果，未完成实时运行行为或其他用户的下载环境复测。** 原始 0.4.2 本机也未复现 Sabsik，因此不能依据这一对比声称 0.4.3 已解决所有告警。真实运行验证应在开启防护、更新安全情报后进行；如果仍被检测，应保留隔离和检测记录供微软分析。
