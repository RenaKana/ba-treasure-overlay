# 数量 1 与国际服全屏切换回归

2026-10-06（Asia/Shanghai），国际服 Steam `BlueArchive.exe` 的原始 WGC 截图。
只读取窗口像素与使用 F11 切换显示模式，没有点击棋盘、换轮或消耗游戏资源。

| 文件 | 画面与人工读数 | 来源 |
|---|---|---|
| vision-pc-user-20261006-count-one-window.png | 第 1 轮，未翻 19；水枪 Finish×0、手机×1、防晒霜×2 | `.artifacts/partial-recognition/capture03-count-diagnosis/probe/wgc-one/first.png` |
| vision-pc-global-fullscreen-initial.png | 第 2 轮，全 45 格未翻；尺寸 4×2 / 4×1 / 3×1，数量 1 / 2 / 5 | `.artifacts/count-fullscreen-fix-20261006/captures/fullscreen/first.png` |
| vision-pc-global-window-initial.png | 与上一行相同盘面，F11 后原生窗口截图 | `.artifacts/count-fullscreen-fix-20261006/captures/window/first.png` |

窗口帧 1284×767，游戏内容区 `[2,45,1280,720]`；全屏帧 3840×2160，内容区覆盖全帧。
“全屏”指用户使用的 F11 模式，不据此声明真正独占全屏。

原数量失败能由原运行 EXE 在原 WGC 图上复现：手机 ×1 读成 OCR 原文 `xl`，结果未知。
截图两视图、尺度和原文证据保存在上述 count-diagnosis 目录。

全屏问题必须用有状态序列复现：原程序同盘窗口学习→全屏出现 11 格待确认，回窗口恢复；
冷启动单张全屏可以识别 45 格。测试分别覆盖这两种情况，不能用冷启动成功代替切换验收。
派生到 1280、1920、2560 的尺寸测试单独属于派生证据；小片段负例也明确为合成测试。

## 2026-10-08 第6轮数量1补充回归

`vision-pc-user-20261008-count-one-round6.png` 来自用户报告后的原始WGC窗口帧，
来源 `.artifacts/ocr-refresh-20261008/current-wgc/first.png`；第6轮，剩余45格，
人工可见件数1/4/3。未点击游戏或换轮，捕获后探针正常清理。
10月6日优化版与10月8日Δ5版对同图均读为未知/4/3；旧的10月6日三张回归图两版均通过。
失败卡的两ROI初次OCR分别为 `xl` 和空文本，分离后的实际数字像素均读为 `111`。
本次仅允许同一数量框的空文本ROI复用另一ROI实际观察到的乘号；两ROI仍必须各自
读出三份相同数字且互相一致。API失败、非空错误前缀、缺失数字或此前冲突仍不接受。

## 2026-10-08 第6轮剩余44格补充回归

`vision-pc-user-20261008-remaining44-partial.png` 来自
`.artifacts/refresh-partial-20261008/current-wgc/first.png`，原始1284×767 WGC截图。
用户已翻开第2行第2列一格，顶部可见44/45、件数1/4/3，代理未点击游戏。
原始80/60/120字号及二值化OCR均漏掉分子；40字号紧裁剪返回 `： 44 ／ 45`。
补充较小字号与全角斜线解析，仍要求分母45、分子合法且不能覆盖此前读数冲突。
修复前EXE已在同图定位到第2类物品2×2、左上角第2行第2列，但因剩余格数未知未显示推荐。
