# Sparse-board fixture provenance

`vision-sparse-ten-covers.png` is a byte-for-byte copy of the local read-only
Windows.Graphics.Capture window capture
`compatibility_probe/artifacts/20260930T135214Z_2fa68ba9/latest.png`.
The original is 1924 by 1142 pixels. SHA-256:
`ada44b55db5ee0b362b49e026b581586b73b236d24f934f18aeedf1d0d365a53`.
The probe's `capture_status.json` reports WGC window capture and completed
sampling; its `evidence.json` identifies the MuMu window. The fixture was
copied without cropping, re-encoding, or adding any pixels. This test work
did not operate the game or upload the image.

All indices are zero-based, row-major on the 9 by 5 board. Full ground truth:

- Empty: 11,15,17,29,31,37.
- Completed: 1,4,5,6,7,10,12,13,14,18,19,21,22,23,24,25,26,27,30,33,34,35,36,38,39,40,42,43,44.
- Covered (`unknown`): 0,2,3,8,9,16,20,28,32,41.

Eight complete gray objects are visible:

- Guns: `(4,0,2,3)` and `(6,2,2,3)`.
- Phones: `(1,0,1,3)`, `(3,1,1,3)`, `(8,2,1,3)`, `(2,4,3,1)`, `(0,2,1,3)`.
- Sunscreen: `(6,0,2,1)`.

The first two cards show Finish; observed remaining-piece counts are
`[0,0,1]`. A fresh recognizer can observe gun/phone gray occupancy without
knowing their type; phone identity is retained when this frame follows
`vision-phones-completed.png` (13 covers, card counts `[0,1,1]`). Only cells
18,27,36 change in this transition. Counts do not supply any cell pixels.

`tests/vision_sparse_board.rs` contains seven distinct regressions. The two
derived sparse cases are generated in memory from the existing real
`vision-phones-completed.png`: its confirmed-empty 104 by 104 patch at
`(1534,450)` (index 15) replaces the other covers at their original grid
coordinates. The one-cover case retains index 16; the two-cover case retains
indices 16 and 20. Existing gray objects and all remaining real covers are
unchanged. These are synthetic geometry checks, not newly played rounds.
The copied opened pixels give positive geometry evidence even though no
cover survives at the old fixed nine sample positions.

The selected-cover derivative reuses the one-cover layout and replaces
index 16 with the intact 104 by 104 selected cover from the existing real
`vision-after-realtime-flip.png` (index 23) or `vision-blue-selected.png`
(index 22). Both variants still require all 45 labels and seven unchanged
gray objects. These are synthetic placements of real observed UI pixels,
not additional production references or captured selection transitions.

The half-cell manual-range shift and flat popup masking the board are
synthetic negative controls. The original header/cards stay visible;
matching OCR counts must not make either invalid board eligible. All
positive cases check every cell, not just the remaining-cover count. The
real ten-cover fresh case repeats observation without a board counter;
the derived one/two-cover cases supply their known counter because unknown
OCR still requires three independent cover anchors. Solver/WASM
recommendations and native runtime behavior
are outside this visual-test evidence.
