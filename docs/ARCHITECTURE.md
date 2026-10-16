# 重构设计

## 目标与边界

面向 Windows 10/11 x64，将监控、权限、清理、策略、界面拆开。优先保持常驻程序依赖少、执行路径明确、错误可见。性能优化目标是较低的驻留资源和减少无收益的重复清理，尚无实机数据证明优于原版。

## 模块

| 文件 | 职责 |
| --- | --- |
| `src/model.rs` | 内存快照、策略、逐项清理结果、可用内存变化 |
| `src/config.rs` | TOML 解析与边界校验、临时文件同步后重命名、日志轮转 |
| `src/policy.rs` | 独立状态机，输入单调时间/使用率/忙碌状态，输出触发原因 |
| `src/platform/windows.rs` | Windows API、权限 RAII、跨进程互斥锁 |
| `src/ui.rs` | Win32 窗口、计时器、托盘、快捷键、单清理线程与通道 |
| `src/cli.rs` | 命令行参数、JSON 输出、退出码 |
| `build.rs` / `app.manifest` | 原生控件主题、系统 DPI 感知、UAC 清单 |

## 执行路径

UI 每秒查询一次系统计数器，把清理请求放到容量为 1 的通道，工作线程执行完毕后返回完整报告。UI 不在消息循环里清理内存。应用退出前关闭通道并回收工作线程；交互退出在忙碌时被延后。

UI 使用线程局部 `RefCell<Option<App>>` 管理状态，消息回调使用 `try_borrow_mut`。原生确认框和托盘菜单可泵送嵌套消息，重入的操作消息被忽略，避免跨回调制造可变别名。窗口销毁消息独立处理。

GUI 用实例互斥锁约束为每会话一个实例；清理用另一把命名互斥锁约束 GUI 与 CLI 不能同时清理。锁是 Local 会话范围，不声明跨 Windows 登录会话互斥。管理员重启的新实例等待旧实例释放互斥锁后接管，取消 UAC 保留原窗口。

内存列表操作前只启用该操作需要的权限，并在作用域退出时恢复。所有返回的句柄都由所有者关闭。FFI 对 Win32 的声明来自 `windows-sys`；仅 Native API 的单个入口单独声明。

## Windows API 映射

| 能力 | 原版实现 | Rust 首版实现 |
| --- | --- | --- |
| 物理内存/使用率 | 系统查询封装 | `GlobalMemoryStatusEx` |
| 提交量/缓存 | 系统查询封装 | `GetPerformanceInfo`，页数乘系统页大小 |
| 清空工作集 | `NtSetSystemInformation` | 同入口，class=80 / command=2 |
| 刷新已修改页 | `NtSetSystemInformation` | 同入口，class=80 / command=3 |
| 全部待机页 | `NtSetSystemInformation` | 同入口，class=80 / command=4 |
| 低优先级待机页 | `NtSetSystemInformation` | 同入口，class=80 / command=5 |
| 收缩系统文件缓存 | `SystemFileCacheInformationEx` | 文档化 `SetSystemFileCacheSize(SIZE_T(-1), SIZE_T(-1), 0)` |
| 注册表缓存/合并物理页/刷新卷 | 额外 Native/卷接口 | 首版不实现 |

内存列表使用 `SeProfileSingleProcessPrivilege`，系统缓存收缩使用 `SeIncreaseQuotaPrivilege`。Native API 的 class 和 command 数值以参考提交为依据，并非 Windows 的稳定公开契约；失败按原样返回 NTSTATUS，不假定所有 Windows 构建都支持。

文档化 API 参考：

- https://learn.microsoft.com/windows/win32/api/sysinfoapi/nf-sysinfoapi-globalmemorystatusex
- https://learn.microsoft.com/windows/win32/api/psapi/nf-psapi-getperformanceinfo
- https://learn.microsoft.com/windows/win32/api/memoryapi/nf-memoryapi-setsystemfilecachesize
- https://learn.microsoft.com/windows/win32/api/securitybaseapi/nf-securitybaseapi-adjusttokenprivileges

## 自动策略

首次达到阈值记为高压起点，低于阈值立即清除此起点。持续时间满足后才允许触发。接受清理请求时记录尝试时间，清理失败也进入冷却；需要回落至阈值减回差后才重新允许高压触发。手动请求也重置自动计时。额外定时间隔以最后一次接受的请求为起点，不依赖高压重新武装，但遵守冷却和忙碌状态。

计时使用 `Instant` 的相对秒数；修改系统时间不改变计时。内存采样失败中断持续高压观察，保留已有冷却记录。重载配置和开关自动策略重置持续窗口，保留上一次尝试时间。自动任务始终使用保守模式，不读取手动强力策略。

## 当前优化措施

- 原生控件，无浏览器、WebView、异步运行时或持续动画。
- 清理线程阻塞等待请求，平时不轮询；GUI 每秒采样一次。
- 托盘提示仅在整数百分比变化时更新。
- 清理串行执行，冷却与回差减少重复操作。
- 逐项错误与后置采样错误独立记录，避免部分失败被表示为“全部成功”。
- 可用内存的差值保留符号，避免把系统变化归零后伪装成收益。
- Release 使用体积优先优化、LTO、单 codegen unit、符号剥离。

原版自身也是轻量原生程序。上述设计措施不等于“比原版更小、更快”，需按验收文档对照测量后才可下结论。
