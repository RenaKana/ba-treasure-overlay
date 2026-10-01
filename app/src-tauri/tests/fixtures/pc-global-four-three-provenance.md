# International native PC client 4:3 fixture

- Captured 2026-10-01 from the real Steam international client, Traditional Chinese treasure board; this is not a stretched 16:9 image.
- Source: `compatibility_probe/artifacts/20261001T031604Z_905f058b/latest.png`.
- Fixture: `pc-global-zh-hant-four-three-initial.png` (original WGC PNG, 1284 × 1007).
- SHA-256: `FADD7F8D190E92858DE1C4C604DFFFA79EDCC3759F1C5E7BDA814A456DBDC003`.
- Observed client content: `[2,45,1280,960]` in capture pixels. Window title bar and capture borders remain in the fixture.
- Visible round 1, remaining 45/45, all 45 covers unopened; card shapes 3×2 / 3×1 / 2×1; quantities 2 / 5 / 2; no Finish banners.

Comparing with the real `pc-global-zh-hant-initial.png` 16:9 capture, the board and round/remaining HUD keep the same width and scale and shift down 120 px. Cards keep the same size and shift down 240 px. The 240 px increase in content height therefore has two observed anchors: centered board/HUD, bottom cards. Expected board is `[607.3333,355.6667,624,346.6667]` in WGC pixels. The shared layout mapping retains actual content bounds and uses a 1280×720 virtual region at y=165 for center and y=285 for bottom.

Tests use the original capture for Windows OCR, card/board recognition and exact geometry. Derived resize/space tests crop only the measured content, uniformly resize it, and add plain margins; those are fixture replay checks, not additional native resolutions. An occluded-board derivative checks that anchors alone cannot create a positive board detection. No real 4:3 completed-board/Finish fixture was available at this point; existing 16:9 completed/Finish tests remain the regression evidence for those paths. Native overlay alignment, focus and live refresh are separate acceptance work.
