# PC client fixture provenance

`pc-global-zh-hant-initial.png` is a static copy of the real International
(Global) Steam PC client's initial board capture. Its source capture is in
`compatibility_probe/artifacts/20260930T162053Z_a7f2b556`. The UTC timestamp in
that directory is 2026-09-30 16:20:53 UTC (2026-10-01 in China Standard Time).

The source was captured from the native game window with HWND `0x1F18B4`,
window title `Blue Archive`, and process
`C:\Program Files (x86)\Steam\steamapps\common\BlueArchive\BlueArchive.exe`.
The full WGC frame includes the title bar and measures 1284x767 pixels; the
client area measures 1280x720 pixels, and the DWM frame measures 1284x767.
The fixture SHA-256 is
`E0F5F1AE7246A81A2934FB9317034450429F484C037956D41DFB16769065F8DB`.

The `pc_global_zh_hant_initial` integration test initializes COM, runs the
Windows OCR path and checks remaining cells `45`, round `1`, and counts
`[2, 5, 2]`. It then calls `analyze_completed_snapshot` and checks 45 unknown
cells, shapes `[[3, 2], [3, 1], [2, 1]]`, three ready references, no completed
objects or candidate constraints, no Finish cards, and board pixel geometry
`[607.33, 235.67, 624, 346.67]` within two pixels. These expected outputs were
observed with the current source build's image-inspection CLI.

This fixture preserves pixels from a real WGC capture of a live native HWND.
The regression test replays only that static image; it does not test client
enumeration, a live WGC session, overlay placement, click-through, or overlay
interaction. Those require separate native runtime evidence. This fixture and
test make no claim about Japan-client support. No official activity name is
recorded because it has not been verified.
