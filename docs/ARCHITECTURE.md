# 架构与参考实现

## v0.3 新增

- `strategy.rs`：基于物理可用量、占用、待机/系统缓存的纯函数选策。智能模式不选全局工作集或修改页刷新，低压时无操作。
- `tray.rs`：参考上游 `_app_iconcreate` 的绘字位图与 CreateIconIndirect 流程；只保留当前 HICON，更新及退出释放，按系统小图标尺寸绘制数字。
- `ui.rs`：WS_POPUP 自绘顶部，无原生 caption/border，HTCAPTION 拖动；Windows 11 DWM 圆角与旧系统区域回退；内嵌设置页及自绘策略按钮。
- `assets/icon.svg`：简洁图标的矢量源；`app.ico` 包含多个尺寸，托盘图标独立生成。

## 分层

- `platform/windows.rs`：Windows 内存查询、权限启用/恢复、NT 内存列表命令、后台工作集回收、跨进程互斥。
- `selection.rs`：前台进程树和同名进程保护、系统/用户排除规则；纯函数测试。
- `model.rs`：内存快照、逐项耗时、后台工作集统计和前后采样报告。
- `policy.rs`：单调时间驱动的持续高压、回落重置、冷却和定时状态机。
- `config.rs`：兼容旧配置的新字段默认值、校验、原子替换及日志轮转。
- `ui.rs`：原生 Win32 控件与双缓冲 GDI 卡片/曲线/按钮，60 个固定上限的历史点；托盘隐藏后降低采样频率。
- `settings.rs`：原生内嵌设置页，取消不修改原配置，保存前校验。
- `installer/` 和 `scripts/`：NSIS x64 安装器、清单与 GUI 冒烟检查、安装/卸载验证。

GUI 线程只采样和绘图。单个 worker 执行回收、CPU 双采样和清理后 500 ms 延迟测量，避免阻塞窗口。GUI/CLI 通过命名互斥锁避免并发清理；强力模式每次确认。

GUI 使用 `requireAdministrator` 清单，由 Windows 在进程启动前处理 UAC。CLI 保持 `asInvoker`，status 普通权限可运行，clean 在权限不足时先拒绝执行。没有服务、驱动、存储密码、关闭 UAC 或修改用户权限分配。SeProfileSingleProcessPrivilege、SeIncreaseQuotaPrivilege 按单项操作启用，RAII 恢复原状态。

## 参考项目与判断

| 一手来源 | 对照内容 | 本项目选择 |
| --- | --- | --- |
| [henrypp/memreduct src/main.c](https://github.com/henrypp/memreduct/blob/04402de8b6a8ff9d1c5517c675d87c346655323c/src/main.c) | NT 内存列表命令、两项令牌权限、系统缓存与工作集操作 | 保留相同系统能力，逐项报告 NTSTATUS 与权限失败 |
| [IgorMundstein/WinMemoryCleaner ComputerService.cs](https://github.com/IgorMundstein/WinMemoryCleaner/blob/main/src/Service/ComputerService.cs) | 按进程 EmptyWorkingSet 与排除列表、系统缓存回收 | 均衡模式增加受筛选的后台工作集回收，并排除前台进程树和活动进程 |
| [WinMemoryCleaner manifest](https://github.com/IgorMundstein/WinMemoryCleaner/blob/main/src/app.manifest) | requireAdministrator | GUI/CLI 分离清单，监控 CLI 无需管理员 |
| [winsiderss/phnt ntexapi.h](https://github.com/winsiderss/phnt/blob/master/ntexapi.h) | SYSTEM_MEMORY_LIST_INFORMATION 的 ABI | 可选查询待机/空闲/修改页；查询失败不伪造计数 |
| [Microsoft AdjustTokenPrivileges](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-adjusttokenprivileges) | 无法给令牌添加不存在的权限 | 启动提升 + 清理前检查；组织策略仍可阻止 |
| [Microsoft EmptyWorkingSet](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset) | 工作集收缩与句柄访问权限 | 不结束进程，不把 WS 变化当作解除内存分配 |
| [Microsoft SetSystemFileCacheSize](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-setsystemfilecachesize) | SIZE_T(-1) 缓存工作集回收 | 采用文档化 Win32 调用，不设置长期硬限制 |

以上是实现思路和系统 API 的对照，没有引入其他项目的 UI 框架、服务或二进制。原始参考项目的归属与 GPL 文本见 NOTICE.md 和 LICENSE。

## 均衡回收

1. 枚举进程，构建前台应用的祖先/后代和同名保护集合；同时保护最近一次观察到的非本程序前台应用。
2. 排除自身、系统名称保护项、系统 critical 标记、其他会话、用户排除项和小工作集。
3. 仅申请 QUERY_LIMITED_INFORMATION | SET_QUOTA；不能访问则记录并跳过，不启用 SeDebugPrivilege 扩大访问。
4. 对候选进程统一等待 150 ms，再读 CPU 时间；增长超过 10 ms 则认为活动明显并跳过。
5. 操作前再次检查前台应用，执行 EmptyWorkingSet，记录前后 WS、成功/失败/不可访问数量。
6. 回收系统文件缓存与低优先级待机页，等待 500 ms 后重新采样系统计数。

这是启发式策略，采样不能证明应用完全空闲。全局强力模式不支持进程排除。前台保护、全屏暂停不代表所有游戏均能被准确识别；配置精确进程名排除仍然有用。

## 效果边界

清理待机页不会直接增加已经包含它的可用内存。后台工作集可按需重新加载，系统提交量不会因为简单 WS 收缩就同步下降。统计保留正负差值，并显示失败和测量缺失；逐进程 WS 总和可能重复统计共享页。不能根据编译或单次数字宣称降低长期占用、修复泄漏或提升帧率。

## 未实现

原项目的注册表缓存、物理页合并、卷缓存刷新、多语言、可配置快捷键组合仍未实现。尚无任务计划自启动、ARM64 及真实多显示器逐屏 DPI 重排。界面按系统 DPI 缩放并根据屏幕高度限制初始窗口尺寸。
