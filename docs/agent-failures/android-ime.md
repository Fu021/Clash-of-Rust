# Android 输入法显示与兼容性证据

[返回索引](README.md) · [Android 宿主](../../apps/android/README.md) · [专项操作](../workflows/ci.md#ci)

## 2026-11-05 多版本对照

[issue #31](https://github.com/Fu021/Clash-of-Rust/issues/31) 的 Redmi K30 / MIUI 14.0.5 在两种输入法下均有光标却无键盘。准确 Android API 级别及 MIUI 实机仍未验证；下列模拟器现象不能直接证明与该设备同根因。

`1851b2f` 的专项 `41999178442` 与采集修正 `f4defcf` 的补测 `42003318092` 合并覆盖 API 26、28、29、30、31、32、33、34、35、36。每个系统三轮正式路径；API 30+ 再三轮移除输入视图动画回调的对照。前两轮动画倍率为 1，第三轮为 0。最终取得 51 轮、346 张截图及配套状态；JUnit 成功只表示导航与采集完成。

| 系统 | 正式路径的截图结果 | 移除动画回调的对照 |
| --- | --- | --- |
| Android 8 / 9 / 10，API 26 / 28 / 29 | 三轮名称、地址及 Back 后重开均可见键盘 | 不支持该动画接口，不运行此对照 |
| Android 11，API 30 | 三轮名称可见，切换地址后三轮均无键盘 | 前两轮切换地址仍无键盘，第三轮可见 |
| Android 12 / 12L，API 31 / 32 | 名称三轮均未绘出键盘，部分状态却报告 IME 可见 | 名称三轮均可见；关闭动画的第三轮切换地址仍会消失 |
| Android 13 / 15，API 33 / 35 | 正常动画两轮名称、地址可见；关闭动画时切换地址异常 | 本轮名称、地址及重开截图可见 |
| Android 14，API 34 | 名称可见；首轮切换地址无键盘，其他时点存在状态变化 | 本轮名称、地址及重开截图可见 |
| Android 16，API 36 | 三轮名称、地址及重开截图可见 | 本轮截图可见 |

截图、输入法 dumpsys 和应用 JSON 按顺序采集，并非同一原子时点；动画或异步隐藏期间可能彼此不同。API 34/35 的六个 Back 后时点没有处于前台且有焦点的 Activity，JSON 的 Android 部分为空，截图仍保留；不能把这些未知值算作隐藏或显示成功。不能只凭 `ime_visible` 或单次几何值判定实际显示。

## 已收窄的实现风险

- `v0.6.6` 与本轮修复前的 Java helper 已含将动画回调移至输入视图的 OEM 修正；[上游 #11344](https://github.com/slint-ui/slint/issues/11344) 不能当作本次遗漏补丁。重复点击已聚焦输入框也会再次请求键盘。
- API 31/32 的保留与移除回调对照明显不同，优先调查 `WindowInsetsAnimation.Callback` 所选择的窗口合成和动画路径。API 31 的正式路径曾同时有有效输入连接、`mInputShown=true`、可见 IME surface 与空白截图；这不能归为简单没有调用 `showSoftInput()`。
- API 30 等版本在字段切换后仍会隐藏，移除动画回调并未解决全部现象。继续核对 helper 的 `requestFocus()`、`restartInput()`、显示与隐藏请求时序及系统回调；不使用无限重试或强制弹出来掩盖失败。
- 所测版本的 `MobileView.keyboard-inset` 始终为 0，即使截图有键盘。这是 Android 窗口属性未同步到移动布局的独立缺口；下述本地补丁未进入这些 APK，不能视为已验证修复。
- API 26/28 日志中有 ART 尝试解析较新动画及 Back 接口的类加载提示，但相应测试仍启动并绘出键盘。不能把这些提示直接当作输入失败或应用崩溃的根因。

## 当前修复与待验收

- [Java helper](../../vendor/slint-android-activity/java/SlintAndroidJavaHelper.java) 移除应用动画回调；字段切换的显示/隐藏请求合并至下一帧，只执行最新状态。窗口尚无焦点时等待焦点到达；已经发出的显示请求不因窗口再次获得焦点而重放。
- API 30–32 在请求 `WindowInsetsController` 显示前同步输入法焦点，保留 posted `showSoftInput()` 兼容路径，依据 [AndroidX 实现](https://github.com/androidx/androidx/blob/androidx-main/core/core/src/main/java/androidx/core/view/SoftwareKeyboardControllerCompat.java)。未加入持续重试、强制键盘或测试进程专属修复；API 26–29 继续采用输入法管理器和可见区域计算。
- [Android 窗口](../../crates/client-ui/ui/mobile/android/window.slint) 将系统键盘高度同步给共享布局，扣除已由布局处理的底部安全区，收起恢复为零。旧 API 的可见区域推断先排除稳定系统栏；横屏侧导航栏不能被当作一个占满屏高的键盘。订阅编辑器在键盘或视口变化、焦点切换时滚动显示当前字段，避免小屏或横屏仍被遮挡；普通手动滚动不触发反复拉回。
- [正式界面回归](../../apps/android/app/src/androidTest/java/org/clashofrust/android/ProxyTunTest.java) 连续切换五次字段，检验文字进入真实输入连接、稳定显示、系统高度与 Slint 布局数值一致及输入框未被键盘覆盖；再检验 Back 清零、重开、前后台与横竖屏恢复。每个系统三轮，前两轮动画倍率为 1，第三轮为 0，保留正式路径截图探测。专项仍只构建一次 APK。
- 每轮必须导出截图、状态、输入法与窗口 dumpsys；日志按本次 marker 隔离，并记录当前输入法崩溃，避免旧日志污染或把输入法进程崩溃误判成应用成功。截图、API 状态与布局断言共同验收，不能仅以 JUnit 成功认定键盘已绘出。

当前仅静态检查与离线脚本回归通过；十版本新补丁、MIUI/Redmi K30 实机及第三方输入法仍待复验。按维护者要求，当前分支尚未取得同提交 CI，不把历史对照成功计为新修复通过。

## 采集失败与验收边界

- 首轮 `41993603936` 被首帧诊断尚未就绪及镜像元数据差异阻断，不算复现。API 31 官方 `x86_64-31_r14.zip` 的下载 revision 为 14，包内 `source.properties` 为 12；分别固定下载身份与包内版本，不放宽为任意值。
- `41995793003` 首次得到 API 31 空白键盘与 API 36 可见键盘的截图；该轮关闭系统动画。后续矩阵补上正常动画、不同系统及回调开关，未将旧运行的成功代替新证据。
- 十版本首轮中 API 30 因存储权限无法导出截图，API 32 被无障碍根节点检查阻断，API 34 模拟器中途断连。前三者的缺失证据由补测补齐；模拟器断连原因尚未确定。
- `f4defcf` 只在确认是可调试模拟器后，才为权限失败重启 root adbd 并重新导出；缺必需截图或状态时失败。探测以 Activity 焦点和真实触摸导航判断就绪，常规严格输入法回归不变。Actions 的 174 项脚本回归（1 跳过）与 11 项产物回归通过；本机仅静态检查。
- 所有截图使用虚构名称与地址，仅测试 x86_64 通用模拟器。API 27、MIUI、ARM 实机、第三方输入法、完整订阅保存、真实 VPN、休眠及重启未由此专项验收；成功采集不等于 Android 8 起全面兼容。
- 故障设备采集跨进程 `adb logcat -b main -b system -v threadtime`、点击前后 `dumpsys input_method`、`dumpsys window windows` 和准确 API 信息，分享前脱敏。`cor logs` 是内核日志，不能替代输入法与系统窗口日志。
