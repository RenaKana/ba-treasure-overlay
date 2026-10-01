# BA 原生捕获 / 点击穿透兼容性探针

这是可丢弃的 Windows x64 / Python 3.12 探针。它用 `windows-capture 2.0.1` 的 **Windows.Graphics.Capture** 捕获指定窗口，用标准库 `ctypes` 的 Win32/GDI 绘制细线 9×5 网格。无需 tkinter、Rust、.NET 或 Qt。网格只是兼容性测试图，**没有识别棋盘、雷区概率计算或自动点击**。

首轮目标为国服 MuMu 的寻宝窗口。MuMu 的结果属于模拟器窗口兼容性证据，日服 Windows 原生客户端仍需单独运行和验收。

## 最短运行步骤

在此目录的 PowerShell 中执行；安装限定在探针自己的虚拟环境：

```powershell
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install --only-binary=:all: -r requirements.txt
.\.venv\Scripts\python.exe probe.py --list-windows
```

本工作区若已在 `D:\Tool\BA扫雷\.venv` 准备依赖，可直接从工作区根目录运行，省略新建环境：

```powershell
.\.venv\Scripts\python.exe compatibility_probe\probe.py --list-windows
.\.venv\Scripts\python.exe compatibility_probe\probe.py --hwnd 0x123456 --seconds 45
```

从 JSON 的窗口标题确认目标，复制对应 `hwnd_hex`，然后启动：

```powershell
.\.venv\Scripts\python.exe probe.py --hwnd 0x123456 --seconds 45
```

默认等待 3 秒，期间手动切回游戏。目标处于前台时才显示网格；失焦、最小化或隐藏时网格隐藏，回到目标时恢复。网格按目标**客户区的物理像素**定位，跟随移动和缩放。默认棋盘归一化矩形是 `x=.475,y=.270,width=.485,height=.480`，只是根据样例的起点，须人工检查对齐。

```powershell
.\.venv\Scripts\python.exe probe.py --hwnd 0x123456 --capture-only --seconds 10 --start-delay 0
.\.venv\Scripts\python.exe probe.py --hwnd 0x123456 --overlay-only --seconds 45
.\.venv\Scripts\python.exe probe.py --hwnd 0x123456 --board-rect .475,.270,.485,.480 --fps 2 --max-images 2
```

MuMu 自绘顶部工具栏导致窗口/全屏归一化坐标不同的情况下，显式开启内容比例校准：

```powershell
.\.venv\Scripts\python.exe probe.py --hwnd 0x123456 --board-rect .474,.265,.486,.479 --content-aspect 16:9 --seconds 45
```

`--content-aspect` 默认为空。设为 `16:9` 时，以这个比例在客户区内等比 fit，横向居中、纵向底部对齐，再将 `--board-rect` 映射到该内容区域。例如 1920×1140 客户区对应内容 `x=0,y=60,width=1920,height=1080`；3840×2160 全屏对应 `x=0,y=0,width=3840,height=2160`。这是人工指定的 MuMu 兼容校准假设，不是自动检测游戏区域；初末内容区域、覆盖层最新内容区域和假设均写入证据。捕获仍保留完整 WGC 窗口帧。

**停止：**到达时限自动退出；也可在控制台按 `Ctrl+C`。切换模式需先停止上一实例。捕获在探针自己的子进程中；正常退出调用 native `CaptureControl.stop()`。原生启动/停止超过 3 秒宽限时，监督进程只终止自己创建的捕获子进程，并在证据里记录未完成原生清理；不会终止目标窗口或进程。

## 看结果

每次运行创建 `artifacts/<UTC时间>_<短ID>/`：

- `evidence.json`：系统和 Python 版本、初末目标客户区/窗口/显示器几何与样式、捕获汇总、覆盖层样式与显示/隐藏统计、清理结果。
- `capture_status.json`：捕获阶段、失败理由、每个已保存样本的源帧时间（100 ns）、接收时间、尺寸、像素/PNG SHA-256 和黑帧候选诊断；也用于原生异常时恢复证据。
- `first.png`、`latest.png` 和有限个 `recent_XX.png`：默认最多 6 张 PNG。`first.png` 永远保留，`latest.png` 始终覆盖为最新已保存帧，其余是近期环形槽。每个槽最终对应哪个样本和哈希在 `saved_slots` 中。

诊断和 PNG 保存频率不会超过 `--fps`（最大 5）；没有补发积压帧。WGC 本身可能更频繁地递送回调，证据分别报告 `callbacks_received` 和 `successful_sampled_frames`。静态画面可能只产生一个 WGC 帧，不能仅因变化帧为零判定失败。变化帧的分母是相邻已保存样本的比较次数。黑帧候选取不超过 64×64 的网格样本，以至少 99.5% 样本像素三个颜色通道均不超过 8 判断；这只是启发式，需查看 PNG。

控制台默认输出简短 JSON 和证据文件路径；`--print-evidence` 输出完整 JSON。退出码：`0` 表示本次程序完成且捕获模式有非黑候选帧，`1` 表示记录了异常，`2` 表示捕获没有非黑候选帧。`0` **不代表原生兼容性已验收**，特别是 overlay-only 没有捕获判定。

## 必须人工确认的门槛

分别在游戏窗口、无边框全屏，以及游戏实际提供的全屏模式运行。手动确认：PNG 内容是当前原生游戏；网格可见、位置正确；游戏普通点击仍能操作，网格不抢焦点；切换窗口和最小化会隐藏网格；退出后覆盖窗口消失。探针不会生成任何输入事件或代替这项验收。

覆盖层设置 `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOPMOST`，使用颜色键透明、`HTTRANSPARENT` 和 `MA_NOACTIVATE`。JSON 只验证这些**静态属性**，始终保留 `real_input_passthrough_tested=false`。窗口矩形等于显示器仅说明几何覆盖，不能证明 true-exclusive 全屏；`true_exclusive_fullscreen_tested` 始终为 `false`，人工验收另写记录。捕获失败时保留原始异常，可能是 WGC/Windows 支持、权限、受保护内容或全屏呈现路径问题；不会自动切换到 hook 或读内存。

探针只枚举可见有标题的顶层窗口并读取公开窗口元数据，只捕获显式选择的 `HWND`。不读取游戏内存、账号或启动配置，不注入 DLL、不安装 hook、不修改游戏窗口、不合成鼠标/键盘。截图只写入此目录内的 `artifacts`。

## 不启动 GUI 的逻辑检查

```powershell
python -m unittest discover -s tests -v
python -m py_compile probe.py capture_worker.py win32_overlay.py probe_logic.py
```

这些测试只验证几何、限速、有限保留和诊断逻辑；不会证明真实 WGC、全屏可见性或点击穿透。

API 来源：[PyPI 2.0.1](https://pypi.org/project/windows-capture/2.0.1/)，该发布 wheel 内 `windows_capture/__init__.py` 已核对支持 `window_hwnd`、`start_free_threaded()`、`CaptureControl.stop()` 和 `Frame.frame_buffer/timespan`；[上游 Python 源码](https://github.com/NiiightmareXD/windows-capture/tree/main/windows-capture-python)。
