# MemReduct RS

参考 [Henry++ / Mem Reduct](https://github.com/henrypp/memreduct) 实现的 Rust 原生 Windows 内存监控与清理工具。独立重构项目，非原作者官方版本。

当前版本 **0.1.0**，目标平台 **Windows 10 / 11 x64**。这是包含界面和真实系统调用的首版，尚未完成真实 Windows 桌面验收，不宣称已覆盖原项目全部功能或性能优于原版。

## 已实现

- 原生 Win32 中文界面，每秒刷新物理内存、可用内存、系统提交量和系统缓存。
- 托盘悬停显示内存使用率，左键恢复窗口，右键清理/退出；关闭和最小化隐藏到托盘。
- 托盘不可用时保留可见窗口；处理 Explorer 重启后的托盘重建。
- 保守、均衡、强力三种手动策略，强力操作每次明确确认。
- 自动清理：高占用持续判定、冷却、回落后再触发；可选定时触发。默认关闭。
- 全局快捷键 **Ctrl+Alt+M**，使用当前手动策略。
- 后台单工作线程执行清理；GUI/CLI 通过命名互斥锁避免同时操作内存列表。
- 按需启用并恢复权限，报告每项 Win32 错误或 NTSTATUS。
- 配置原子替换、便携模式、JSON 命令行输出、约 1 MiB 轮转日志。
- GUI 普通权限可监控，通过「管理员重启」处理 UAC；取消提升会保留原实例。

本工具通过系统接口回收缓存与工作集，不分配巨量内存“挤压”系统，不结束应用进程。

## 运行

在 [Releases](https://github.com/sadness-sa/memreduct-rs/releases) 下载 `memreduct-rs-0.1.0-windows-x64.zip`；同页提供完整源码压缩包和 SHA-256 校验文件。

解压便携包，双击 `memreduct-rs.exe`。监控不需要管理员权限，清理需要管理员权限，可点击「管理员重启」。

默认配置路径为 `%LOCALAPPDATA%\MemReductRs\config.toml`，首次启动自动生成。点击「编辑配置」，在记事本保存后点击「重载配置」。配置错误会被报告，原配置不会被默认值覆盖。

便携模式将配置和日志写到可执行文件所在目录，因此该目录须可写；两个可执行文件应放在同一目录。

```powershell
.\memreduct-rs.exe --portable
.\memreduct-rs.exe --portable --minimized

# 普通 PowerShell：查询状态
.\memreduct-cli.exe status
.\memreduct-cli.exe status --json
.\memreduct-cli.exe config --portable

# 管理员 PowerShell：执行清理
.\memreduct-cli.exe clean --portable
.\memreduct-cli.exe clean --profile balanced --json --portable
.\memreduct-cli.exe clean --profile aggressive --yes --portable
```

CLI 默认使用保守策略。`--yes` 仅用于强力模式的显式确认，不会绕过 Windows 权限。退出码：`0` 全部成功，`1` 执行或配置错误，`2` 参数错误，`3` 部分/全部清理项失败或后置测量失败。JSON 模式成功生成报告时，stdout 只输出 JSON；执行前错误写入 stderr。

## 清理策略

| 策略 | 系统操作 | 使用入口 |
| --- | --- | --- |
| 保守 conservative | 清理低优先级待机页 | 手动、自动、CLI |
| 均衡 balanced | 收缩系统文件缓存工作集 + 清理低优先级待机页 | 手动、CLI |
| 强力 aggressive | 清空工作集 + 收缩系统缓存 + 刷新已修改页 + 清理全部待机页 | 手动确认或 CLI `--yes` |

Windows 已将大量可回收待机缓存计入“可用内存”，所以清理待机页后可用内存可能几乎不变。清空工作集可能提高短时可用内存，也可能增加后续缺页、磁盘读取和游戏卡顿。结果展示**带正负号的可用内存变化**，包含其他程序的并发活动，不把它叫作精确释放量，也不保证提升速度。

自动策略固定使用保守模式，默认配置为：使用率 ≥85% 连续 10 秒后触发，任何已接受的手动/自动清理后至少冷却 300 秒；占用须回落至 ≤75% 才重新允许高占用触发。失败的清理也记录尝试时间，避免重试风暴。开启定时触发后，即使未回落也可在间隔到期后清理，仍受冷却限制。

日志在配置旁的 `config.log` / `memreduct-rs.log`，每行 JSON，包含逐项结果和 Unix 时间；保留一份 `.log.1` 旧日志。未发起网络请求，不上传遥测。

## 编译

推荐 Windows 安装 Rust stable 的 MSVC 工具链和 Visual Studio Build Tools 的 **Desktop development with C++**（包含 Windows SDK）。锁定依赖文件随源码提供。项目声明 Rust 1.85 及以上，当前验证工具链版本见 `docs/VALIDATION.md`。

```powershell
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
.\target\release\memreduct-rs.exe
```

GUI 与 CLI 分开构建：GUI 不弹控制台；CLI 可重定向或接入 PowerShell。嵌入清单启用系统 DPI 感知、Common Controls v6 和按调用者权限运行。GNU 工具链构建须安装 MinGW 的 `windres`。

Linux 交叉编译：

```bash
rustup target add x86_64-pc-windows-gnu
# 安装发行版提供的 mingw-w64（gcc、binutils、Windows 头文件与库）
cargo check --target x86_64-pc-windows-gnu --all-targets --locked
cargo build --release --target x86_64-pc-windows-gnu --locked
```

输出在 `target/x86_64-pc-windows-gnu/release/`。纯策略、配置和参数校验的测试也可在 Linux 上运行；真实内存清理只在 Windows 上执行。

## 范围与后续开发

首版未实现原项目的注册表缓存回收、物理页合并、卷缓存刷新、任务计划启动、托盘数字图标、可配置快捷键组合、多语言及安装器；也不兼容 Windows 7。窗口使用系统 DPI 缩放，尚未实现跨显示器实时 DPI 重排。ARM64 尚未验证。

开发架构和与上游的映射见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)，Windows 实机验收及性能对比方法见 [docs/VALIDATION.md](docs/VALIDATION.md)。GitHub Actions 包含 Linux 测试和 Windows MSVC 构建、测试及只读状态查询；放入自己的仓库后才会执行。

## 来源与许可证

参考上游提交 `04402de8b6a8ff9d1c5517c675d87c346655323c`（2026-10-16 检查）。保留 GPLv3 许可证文本和原作者归属，详见 [NOTICE.md](NOTICE.md)。本项目采用 **GPL-3.0-only**。
