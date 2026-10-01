# Round seven window-resolution regression

- Source: `compatibility_probe/artifacts/20260930T161537Z_78caa812/latest.png`, real game window captured on 2026-10-01 Asia/Shanghai (capture artifact timestamp is UTC).
- Fixture: `vision-resolution-round-seven.png`, unchanged 1960 x 1162 RGBA screenshot.
- SHA-256: `8c2496dba13a69434d5fa4d45b5077bdc3dc169fa9c0d08dd19ebc2134de0b24`.
- Observed round: 7; covered cells: 32; quantities: 1 / 2 / 5; card shapes: 4 x 2 / 3 x 1 / 2 x 1.
- Fully revealed objects: cells 11,12 (item 2); 14,23,32 (item 1); 27,28,29,30,36,37,38,39 (item 0). Other cells remain covered.
- The source window's game border is at x=2..1958, bottom=1160. Its nominal 16:9 content rectangle is [2,59.75,1956,1100.25]. Whole-window scaling scales those borders too; a fixed 2-pixel border is incorrect at 960 pixels wide.
- Visual regression derives 960/1440/1960/2560/3840 pixel wide windows dynamically with Catmull-Rom resampling. OCR tests share this same source fixture.
- The original reported failure was a 960 x 569 OpenCV INTER_CUBIC derivative in `.artifacts/resolution-compat-20261001/round7-960.png`: the old [2,29.25,956,537.75] rectangle lost card 0's 4 x 2 mask and left eight object cells uncertain; the measured [1,29.125,958,538.875] rectangle recognizes all three masks and all thirteen completed cells without relaxed recognition thresholds.
- Derived screenshot tests are replay evidence, not proof of live native window resizing or DPI changes.

## Native maximized window

- Source: `compatibility_probe/artifacts/20260930T163940Z_ae9ddafe/latest.png`, captured from the actual maximized MuMu window on 2026-10-01 Asia/Shanghai, with Android internal resolution 1920 x 1080.
- Fixture: `vision-resolution-round-seven-maximized.png`, unchanged 3840 x 2094 screenshot; SHA-256 `8df23a697679ba6aff60abfe0f28b884b486f60508aae5afc9704516e6be7a22`.
- Round, quantities, shapes and revealed cells are unchanged from the earlier round-seven capture above. No board cells were clicked for this compatibility check.
- Content geometry remains [114,56.25,3612,2031.75]. At this raster phase, the first four rows of each normalized shape patch belong to blue card artwork; the light shape panel begins at row 4. Starting component scans unconditionally at row 3 mistook artwork on cards 1 and 2 for clipped mask squares.
- The repair starts mask components only on rows containing observed light icon-panel pixels, using the existing light-anchor definition. Component tracing, size, area, grid and clipping checks are unchanged. The native screenshot now preserves all thirteen completed cells without shifting the board.
- A separate real control capture at internal 1280 x 720 (`compatibility_probe/artifacts/20260930T164035Z_f8047ed2/latest.png`) had the same outer content rectangle and passed before this fix; it is not duplicated as another binary fixture.

## Native resize with eleven covered cells

All three unchanged captures below are from the same round-seven board on 2026-10-01 Asia/Shanghai. Quantities are 0 / 0 / 2, masks are 4 x 2 / 3 x 1 / 2 x 1, and cards 0 and 1 show Finish. The independently inspected 1920 x 1080 ADB baseline is `.artifacts/resolution-compat-20261001/resume-adb.png` with its full 45-cell record in `resume-adb-inspection.json`: eleven covered cells, one empty cell (2), and thirty-three completed cells in nine objects.

| Fixture | Real source | Capture size | SHA-256 |
| --- | --- | --- | --- |
| `vision-resolution-round-seven-eleven-maximized.png` | `compatibility_probe/artifacts/20260930T174636Z_4754909d/latest.png` | 3840 x 2094 | `32db3b96cf608e93b71969e1757e9db9fe0d885f0251482155defeb036164729` |
| `vision-resolution-round-seven-eleven-small.png` | `compatibility_probe/artifacts/20260930T174824Z_11c36a41/latest.png` | 970 x 605 | `46d0bee0ffd6025216d567ba24a7fb2b4259efc18a3d7c995c2792cae5e87d71` |
| `vision-resolution-round-seven-eleven-internal720.png` | `compatibility_probe/artifacts/20260930T175300Z_fc64b501/latest.png` | 3840 x 2160, internal Android 1280 x 720 | `7b6befeedf4a564a84008d310246816682b5e02e60f5507289fdaff7444f109c` |

- In the maximized capture, the ordinary 64-sample-per-cell neutral component for cells 18/19 has bounds [3,131,123,66]. A narrow left frame/shadow spur reaches y=197, beyond the row-two footprint's existing y=195 containment tolerance; the ordinary footprint check rejects it. Cleanup is limited to failed components within the outer frame margin and the original footprint, positive-pixel, color, texture and 0.78 identity checks are retained.
- In the small capture, the third-card foreground mask connects to a cropped full-width bottom divider. Its corrupted 154 x 120 bounding box fails every rotation aspect check. A zero-padded opening is used only when the extracted component touches the crop boundary, restoring the observed sprite silhouette; ordinary interior templates keep their prior path.
- In the internal-720 capture, automatic background fitting labels cell 39 empty from its interior while the completed surfboard's thin tail remains visible at the cell edge. Full neutral silhouettes are validated before that automatic label pass. Existing covers and trusted ice labels still exclude object candidates.
- The regression checks all 45 cells and nine objects on every capture both from a fresh recognizer and after seeding the original round-seven fixture. The warm recognizer preserves the first two card fingerprints under Finish across the sequential real capture sizes. These fixture replays are separate from the final rebuilt EXE and live capture acceptance.
