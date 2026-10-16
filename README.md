# MemReduct RS

参考 [Henry++ / Mem Reduct](https://github.com/henrypp/memreduct) 独立实现的轻量 Rust Windows 内存监控与回收工具。非上游官方版本。GPL-3.0-only。

## 下载安装

**Windows 10/11 x64 · v0.3.0**：[下载安装程序](https://github.com/sadness-sa/memreduct-rs/releases/download/v0.3.0/MemReduct-RS-0.3.0-x64-Setup.exe)，或进入 [Releases](https://github.com/sadness-sa/memreduct-rs/releases)。不再要求用户解压便携包。

从桌面或开始菜单启动后，Windows 会自动弹出管理员权限请求（UAC）；同意后才能运行 GUI 和清理。不会关闭或绕过 UAC。安装目录为 Program Files；可从 Windows「已安装的应用」卸载。安装/更新前请从旧版托盘菜单退出，用户配置与日志在卸载后保留。

## 界面与使用

- 无系统标题栏的一体化原生界面、自绘最小化/关闭、可拖动顶部区域；设置页直接在主窗口切换。
- 实时占用曲线、物理可用内存、待机缓存和本程序工作集。
- 托盘直接显示占用数字（例如 85 表示 85%），达到阈值变色；应用使用简洁透明芯片图标。
- 新安装的 GUI/CLI 默认「智能选策」，根据实时计数决定跳过、后台工作集、缓存或组合回收；结果显示选择理由。
- 点击「优化设置」调整阈值、持续时间、冷却、回差、定时间隔和进程排除列表。
- 自动模式默认关闭；默认智能选策，也可切换为轻量、均衡或压力回收；不允许全局深度清理。默认在 Windows 报告全屏 D3D 游戏或演示时暂停自动优化（并非覆盖所有无边框窗口）。
- Ctrl+Alt+M 触发当前手动策略；关闭窗口隐藏到托盘，托盘菜单可清理、显示和退出。
- 升级保留旧配置。点击「智能选策」会同时切换手动与自动选策方式，不会自动打开自动优化开关。

### 为什么之前报权限错误或变化为零

`SeProfileSingleProcessPrivilege` 无法添加到普通用户令牌；需要提升后的管理员令牌。新版 GUI 用嵌入清单自动请求管理员权限，清理前也会检查权限；缺少用户权限分配或受组织策略限制时，仍会明确报告原因。

Windows 的可用内存包含待机缓存，单纯清理待机页往往不会使可用内存显著增加。新版分别显示物理可用、待机缓存、系统提交量及后台进程工作集变化。进程工作集之和可能重复统计共享页，不能当作精确释放的物理内存。完全失败时只报告失败，不再把 +0.0 MiB 当作清理结果展示。

### 后台进程选择

均衡回收跳过本程序、前台应用及其进程树/同名进程、系统关键进程、其他会话及小工作集进程，默认排除 OBS。通过 150 ms CPU 采样跳过活动明显（CPU 时间增长超过 10 ms）的进程，操作前再次检查当前前台进程。无权访问的进程会记录数量；不会加载驱动、结束进程或分配大块内存挤压系统。

这只是对短暂采样的启发式选择，不等同于识别所有游戏或繁忙程序；建议将游戏、直播、数据库等常用程序加入排除列表。强力模式执行全局工作集清理，不使用这些排除规则，每次明确确认。

| 策略 | 操作 |
| --- | --- |
| 智能选策 smart | 自动决定具体操作，内存充足时跳过；不会选择全局深度清理 |
| 后台回收 background | 仅回收筛选后的后台进程工作集 |
| 系统缓存 cache | 仅收缩系统文件缓存工作集 |
| 待机回收 standby | 仅清理全部待机页 |
| 压力回收 recovery | 筛选后台工作集 + 系统缓存 + 全部待机页 |
| 轻量缓存 conservative | 低优先级待机页 |
| 均衡回收 balanced | 空闲后台工作集、系统文件缓存、低优先级待机页 |
| 深度清理 aggressive | 全局工作集、系统文件缓存、已修改页、全部待机页；可能增加缺页和磁盘 IO |

回收工作集不会修复泄漏或解除应用已提交的内存，后续使用可能重新加载。不能保证降低长期内存用量、提高帧率或消除卡顿。详细对照方法见 [验收记录](docs/VALIDATION.md)。

### 智能选策规则

| 当前状态（从上到下判断） | 处理方式 |
| --- | --- |
| 物理总量不可查询 | 跳过，不凭空选择策略 |
| 占用 ≥94%，或可用 ≤ max(物理总量的 4%, 512 MiB) | 压力回收 |
| 占用达到配置阈值，或可用 ≤ max(物理总量的 10%, 1 GiB) | 均衡回收 |
| 占用 <70% | 无需回收，保留工作集和缓存 |
| 待机缓存 ≥ max(物理总量的 12.5%, 512 MiB) | 待机回收 |
| 系统缓存 ≥ max(物理总量的 20%, 1 GiB) | 系统缓存回收 |
| 其余占用偏高情况 | 后台工作集回收 |

选择是可解释的规则判断，不是通过模型推理。系统提交量达到上限 90% 时会额外提示：工作集回收不能解除应用分配。持续占用、冷却和全屏暂停仍控制自动触发；定时间隔到期也不会强迫智能模式执行无必要的清理。

## 配置与 CLI

配置：`%LOCALAPPDATA%\MemReductRs\config.toml`；日志在配置旁，约 1 MiB 轮转。也支持在可写文件夹运行 `--portable`，使用 EXE 旁的 `memreduct-rs.toml`。GUI 普通启动需要 UAC，CLI `status` 可普通权限运行；CLI `clean` 需要管理员终端。

```powershell
.\memreduct-cli.exe status --json
.\memreduct-cli.exe clean --profile balanced --json
.\memreduct-cli.exe clean --profile aggressive --yes
```

CLI 默认 smart，`--yes` 只确认强力清理，不提升系统权限。退出码：0 成功，1 执行/配置错误，2 参数错误，3 部分/全部清理项失败或测量失败。每项结果包含耗时和后台工作集统计。无遥测、无后台联网。

## 构建与发布

需要 Rust 1.85+、Windows MSVC 工具链及 Windows SDK（包含 rc.exe）。在 VS Developer PowerShell 中运行：

```powershell
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
.\scripts\build-installer.ps1
```

安装脚本使用 NSIS；缺少 NSIS 时 CI/脚本通过 Chocolatey 安装。GitHub Actions 会在 Linux/Windows 运行测试，在 Windows 上生成两个 EXE、验证管理员清单、启动退出 GUI、构建安装程序并执行安装/卸载冒烟检查。检查全部通过后自动发布本版本 Setup.exe、源码包和 SHA-256。

编译器与 CI 检查不能替代用户真实桌面的权限提示、功能和性能验证。详细实现、参考项目及未完成项见 [架构说明](docs/ARCHITECTURE.md) 和 [验收说明](docs/VALIDATION.md)。
