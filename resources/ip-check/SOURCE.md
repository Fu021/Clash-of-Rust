# 原生 IP 与平台检测

`services.json` 为 183 项检测目录：出口 IP、GitHub 由 `src/ip_check.rs` 实现；其余 181 项由 `src/region_check/generated.rs` 中的原生 Rust 函数实现。

运行时不需要 Python、Git、Bash、curl 或 jq。Python 工具仅用于开发、资源维护与打包。上游来源及许可证见 `vendor/region-restriction-check/SOURCE.md`。
