# 新增局部识别回归截图

这些都是用户提供的原始截图，未缩放或旋转。L 表示行，C 表示列，从 1 起，
矩形包含左上格到右下格之间全部格子；同一行或列只写一次。

| Fixture | 来源及真值 | 轮次 / 人工 HUD |
|---|---|---|
| vision-user-20261006-phone-partial-finish.png | 用户 capture03；后续完整 WGC 帧核验手机 L2L4C6。详见 `.artifacts/partial-recognition/run-20261006-capture03/retrospective-verification.json` | 1 / 剩余 24，件数 0、3、2 |
| vision-user-20261007-umbrella-middle.png | `codex-clipboard-c9616aa2-4393-47b2-b4ed-5ddf2c613d2f.png`；用户确认雨伞 L2C4C7 | 2 / 剩余 37，件数 1、2、3 |
| vision-user-20261007-umbrella-handle.png | `codex-clipboard-fdce61fe-8ef0-4d4a-a88e-a336f363e917.png`；用户将答案更正为雨伞 L3C4C7，L2C4C7 为另一个已完成雨伞 | 2 / 剩余 20，件数 0、1、2 |
| vision-user-20261007-bow.png | `codex-clipboard-93049a42-239a-44f3-86c2-8d1d36599415.png`；用户确认蝴蝶结 L1C8L2C9 | 3 / 剩余 44，件数 1、4、3 |
| vision-user-20261007-swim-ring.png | `codex-clipboard-5ad6f476-6711-4b67-aed0-8fd5ba9e9fb9.png`；用户确认游泳圈 L1C1L3C3 | 3 / 剩余 38，件数 1、3、2 |

2026-10-07 前景引导细化根据游泳圈失败进行开发，因此以上连续序列在本次
清单中整体归入 tuning/regression。历史冻结验证报告保持不变，不能把修改后
的同一图重新宣称为独立验证。图案的精确连续角度均无可靠真值。

HUD 来源为当帧截图人工读数，与隐藏占格真值分开。蝴蝶结截图的生产 OCR
能读出件数 1、4、3，但剩余格数为未知；离线示例使用人工 HUD，不能把局部
匹配成功算作 OCR 成功。后续完整帧只用于验收，不输入较早帧的 matcher 或缓存。

## 2026-10-08 已确认物品内新增片段

`vision-pc-user-20261008-tracked-ring-31.png` 和
`vision-pc-user-20261008-tracked-ring-30.png` 分别来自
`.artifacts/live-refresh-latency-20261008/current-wgc/first.png` 与
`after-user-wgc/first.png`，为同一第6轮原始1284×767 WGC画面，件数均1/4/3。
前图已有8件已确认占格，泳圈位于L3C4L5C6，露出索引22、40；用户随后翻开
其内部索引31（L4C5），剩余格数由31变30。代理未代点游戏或消耗资源。
用户明确要求已确认物品采用持续跟踪，内部新增格不因重新平均误差而撤销旧判断。
重新全量判定时正确泳圈矩形的前景误差升到40.21，超过原门槛38；图像未改变类别或占格。
这组素材用于有状态跟踪行为回归，不能作为新增独立隐藏格准确率样本。
