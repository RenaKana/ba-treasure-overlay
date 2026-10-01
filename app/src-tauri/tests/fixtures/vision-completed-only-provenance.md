# Completed-only recognition fixture

`vision-round-two-completed.png` is a byte-for-byte copy of the read-only
MuMu/WGC capture at
`compatibility_probe/artifacts/20260930T145508Z_588e2fd1/latest.png`.
SHA-256: `bc7133d81e9bb2b3cd861396df9d9b5ba3f19e184880c5956dad079ef5e81a11`.
The original frame is 1924x1142. Its HUD shows round 2, 27/45 remaining
cells, card quantities 0/1/3, and shape badges 4x2, 4x1, 3x1.

All 18 exposed cells belong to four visibly complete gray objects:

- Surfboard: `(x=3, y=0, width=2, height=4)`.
- Phones: `(0,1,3,1)` and `(0,2,1,3)`.
- Umbrella: `(6,1,1,4)`.

The surfboard card has a Finish banner. Its gray component at the 64px/cell
recognition resolution has bounds `(218,7,75,240)`, area 13068, and positive
pixels in each of its eight cells. The previous minimum silhouette width
was `2 * 64 * 0.60 = 76.8`, so it rejected the component before evaluating
the footprint. The initial round-two card reference gives a silhouette IoU
of 0.972304, confirming that the reference cache was not the blocker.

`vision_completed_only` verifies this frame both cold and after loading
`vision-round-two-initial.png`. It also verifies that existing real partial
watergun/phone frames publish uncertainty and no automatic typed hits or
candidate placements, while the real completed watergun and prior completed
phone frame retain their pixel-confirmed occupancies. Card quantities do not
generate observed board labels. This is captured-frame evidence; live host
pause/resume behavior is verified separately.
