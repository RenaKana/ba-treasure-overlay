# 国际服与日服 PC 兼容验证

日期：2026-10-01（Asia/Shanghai）。以下记录保留合并前的独立验证结果。独立工作树基于本地最新已提交版本
`a7c9d72d58a041477817882645306160d241e54b`，分支 `codex/pc-client-compat`。
该次独立验证没有引入 `D:\Tool\BA扫雷` 分辨率适配工作区的未提交修改。
2026-10-01 已将这里的窗口选择、测试和资料合入该主工作区；合并后的检查见
[桌面验收记录](desktop-acceptance.md#分辨率与-pc-客户端改动合并2026-10-01)。
下文的原始捕获、诊断和独立测试包路径相对于来源工作区
`C:\Users\admin\.codex\worktrees\29a6\BA扫雷`，原生验收对应下文的独立 EXE。

## 改动

窗口自动选择支持实测的国际服 `Blue Archive` 和日服 `ブルーアーカイブ`。
英文名称采用完整标题匹配，避免把启动阶段的
`BlueArchive_JP_Gamelauncher` 误选为游戏；日服启动器稳定后的标题为
`ブルアカ`，同样不自动选择。保留 MuMu 和用户手动选择行为。

未更改活动预设、OCR、棋盘坐标、图案识别或求解算法。
两个原生游戏首次采集的客户区均为 1280×720，带标题栏 WGC 帧均为 1284×767。
这证明当前窗口尺寸可用于下列验证，不代表任意分辨率已通过。

## 实测范围

| 项目 | 国际服 Steam PC | 日服 Yostar PC |
| --- | --- | --- |
| 实际进程 | Steam/steamapps/common/BlueArchive/BlueArchive.exe | YostarGames/BlueArchive_JP/BlueArchive.exe |
| WGC 捕获 | 3 帧，2 次相邻变化，0 黑帧，0 错误 | 3 帧，2 次相邻变化，0 黑帧，0 错误 |
| 画面 | 繁体中文活动第 1 回合初始棋盘 | 标题画面，当前无扫雷活动 |
| OCR 与识别回放 | 45 格未翻；尺寸 3×2 / 3×1 / 2×1；剩余 2 / 5 / 2；轮号 1；三张参考均建立 | 正确拒识：present=false、board=null、无物品或位置约束 |
| 实际 WASM | 2,662,809,982 种布局，100,000 次抽样；8 组概率矩阵范围、长度及面积守恒通过 | 无棋盘，不求解 |

国际服现有活动与国服相同（用户确认）。本轮没有验证其他活动、局部翻出、
完成物品或换轮，没有操作游戏翻格或消耗资源。日服没有可用活动，不能用其
非棋盘捕获或国际服结果宣称日服活动识别已验收。

国际服静态 fixture 与测试契约见
[`pc-clients-provenance.md`](../src-tauri/tests/fixtures/pc-clients-provenance.md)。
日服标题画面的原始截图仅留在本地忽略目录，不纳入回归素材。

原始 WGC 记录位于仓库根目录：

- `compatibility_probe/artifacts/20260930T162053Z_a7f2b556/`（国际服）
- `compatibility_probe/artifacts/20260930T162717Z_f8e25ffe/`（日服）
- `.artifacts/pc-client/global-baseline.json`、`global-to-wasm.json`、`jp-nonboard.json`

## 检查

- `pnpm test:desktop`：22/22，通过实际国际服、日服与启动器标题的选择回归。
- `cargo test --release --locked --manifest-path app/src-tauri/Cargo.toml --test pc_clients pc_global_zh_hant_initial -- --exact`：1/1，通过真实国际服截图的 OCR 与 45 格识别。
- 基线 OCR 子集：9/9；源码重新生成 WASM，TypeScript / Vite 生产构建通过。
- `logic.ts` 定向 ESLint 有 14 项存量错误；对 `a7c9d72` 的同一文件检查得到相同 14 项，仅行号因新增代码偏移。没有修改无关命名与格式。

截图回放不等于原生叠加、真实输入穿透、独占全屏或连续翻格性能验收。

## 独立原生测试版

交付目录：仓库根目录 `release/BA-PC-Compatibility-20261001/`。
EXE SHA-256：`040A955E679C13E631B4BABF10734FA3662D49E88C67BF61226501FC53F302FE`。

构建时仅为此测试包传入
`TAURI_CONFIG={"identifier":"local.ba.treasure.overlay.pc-compat"}`，然后执行
`cargo build --release --locked --features custom-protocol --manifest-path app/src-tauri/Cargo.toml`。
前端来自上述 `pnpm build`。标识覆盖未写入产品的 `tauri.conf.json`；此包使用
`%APPDATA%\local.ba.treasure.overlay.pc-compat`，与原助手配置隔离。

已经从交付目录启动该 EXE，原生界面默认选中 `ブルーアーカイブ`，连接并手动刷新
日服 1284×767 非活动画面，预览正常、45 格待确认、未发布概率。随后在同一助手
停止日服捕获，手动选择 `Blue Archive` 并连接国际服，读取正确物品及 45 格棋盘，
收到概率绘制回执。原生窗口元数据确认概率层、刷新窗可见，国际服保持前台。

国际服 `.artifacts/pc-client/global-native-diagnostics.json` 记录同一初始棋盘
3 次刷新到绘制为 880 / 800 / 864 ms。样本未逐次标注窗口模式，不能当作纯窗口或
纯全屏性能统计，也不代表连续翻格性能。首次由控制窗触发到绘制 11094 ms 包含
控制窗检查与切回游戏的等待，单独保留。

用户随后确认：“数字对齐，刷新正常不抢焦点，且测试全屏模式也正常”。这覆盖本包
国际服窗口与用户切换的全屏模式的可见性、对齐和小按钮刷新；未证明 true-exclusive
呈现模式，也未测试棋盘区域的真实点击穿透或连续翻格。没有为此次检查消耗道具。
日服当前没有该活动，活动 HUD、棋盘识别与叠加对齐仍待有真实画面后验收。

## 4:3 后续候选（2026-10-01）

`release/BA-Treasure-Overlay-4x3-highlight-performance-20261001/` 已在真实国际服窗口验证 16:9→4:3 自动恢复、手动一次刷新、窗口移动及提示隐藏/恢复。4:3 游戏内容为 1280×960，WGC 捕获为 1284×1007；当前初始棋盘数字、全部 45 格及原生叠加边界正确。棋盘/HUD 居中、物品卡片贴底，两组锚点保持原有图案尺度。

此候选另外加入默认关闭的最高概率粗框/填色开关，并减少重复预览和窗口更新开销。同一 16:9 静止棋盘的助手进程树 CPU 小样本下降约 34%。完整测试、采样口径、候选哈希及尚未覆盖的 4:3 Finish/残局和最终高亮像素人工确认见 [桌面验收记录](desktop-acceptance.md#国际服-43最高概率高亮与-cpu-优化2026-10-01)。日服活动验证状态不变。
