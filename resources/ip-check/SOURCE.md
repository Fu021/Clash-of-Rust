# IP 检测平台索引

平台名称及公开检测地址参考 [RegionRestrictionCheck](https://github.com/1-stream/RegionRestrictionCheck)，整理日期：2026-10-06。索引包含 181 个平台检测条目，另增加出口 IP 与 GitHub 项目。

平台检测已接入随安装包携带的 AGPL-3.0 上游适配脚本。固定版本、修改说明及授权见 [vendor/region-restriction-check/SOURCE.md](../../vendor/region-restriction-check/SOURCE.md)。Rust 界面与桥接部分保持 GPL-3.0-only。

181 个平台条目使用上游各自的请求、鉴权及解锁判断。出口 IP、GitHub 仍使用 Rust 检测。结果包含可用、受限、仅网页、仅自制内容等状态；地区来自原检测结果，不根据用户本地 IP 猜测。平台接口变化仍可能导致失败，检测不能保证账号登录或实际播放成功。
