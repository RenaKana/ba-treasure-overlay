# Vision fixture provenance and evidence

All real frames are local, user-supplied MuMu/WGC captures. No image was
uploaded and no model/service is called by the recognizer. Indices are zero
based, row-major over the fixed 9 by 5 board.

## Production references

Production embeds only `vision-hidden-tiles.png` (the common beveled-cover
UI structure) and `vision-selected-cover.png` (the common selection frame
and check mark). The cover comparison includes affine grayscale/channel
contours and locally registered shapes. Covers independently verified in
the current frame supply additional raster phases for a second structural
pass. The derived all-purple-cover test preserves all 45 covered labels.
These are embedded UI references; it would be incorrect to claim that
production contains no captured reference pixels at all.

When no manual samples are saved, an additional per-round path learns all 45
covers from their own positions. It requires same-frame remaining=45, three
valid card shapes, no Finish, closed rectangular edges in every cell, and two
stable observations. This bootstrap checks RGB edge continuity without an
embedded color or bevel template. Each subsequent tile is compared only with
its own initial patch; whole-patch similarity, local new-pixel checks and
frame edges prevent small revealed fragments from being silently treated as
covers. Confirmed automatic references survive same-round resizing and
temporary occlusion, but reset with the round/session/calibration. They are
memory-only and are not new distributed image assets. Mid-round startup
retains the embedded structural fallback; selection markers retain their
existing recognition path.

Players may instead select one or more still-covered cells from a frozen
45-cell preview, choosing one cell for each appearance. If the normal cover
recognizer cannot locate a new appearance, sampling may still be offered when
the title, three cards and a supported board layout can be located. The
preview slices are only user-selection candidates and are not recommendation
evidence until saved. Saved samples are 32x32 RGB patches in the local
`%APPDATA%\local.ba.treasure.overlay\cover-reference.json`; they persist
across rounds, restarts and calibration, match at any board position, and are
neither uploaded nor added to the repository. A mismatch never silently
relearns or replaces manual samples. Clearing them restores automatic
recognition, including the per-position bootstrap and embedded structural
references above. This path stores selected patch pixels, not screenshots.

`vision_manual_cover.rs` uses a synthetic unfamiliar cover appearance to
exercise selection at any position, persistence through round resets and
resampling, changed-cell detection, and the lack of silent relearning or
fallback. It is synthetic replay evidence, not acceptance of a new live
event. No screenshot fixture is added for the generated appearance.

`vision_initial_board.rs` exercises a generated 45-cell skin on a real HUD,
per-position identity, a small inserted fragment, missing/stale counts,
Finish, occlusion, resizing, and reset. The module tests also replay real
MuMu/PC initial grids and 960–3840-pixel resampling. A separate host test
confirms two full-cover observations reset the round when the round number
is unreadable and the new covers differ from the cached ones. These are
replay/synthetic checks, not acceptance of another live event.

There are **no embedded item/card/fragment atlases or ice/background images**
in production. The old `vision-item*-*.png` and `vision-empty-*.png` files
remain historical provenance, not runtime references.

The three fixed card-art regions are sampled from the current viewport.
Side-margin background palettes, component cleanup and an eroded foreground
core retain the whole icon, including the phone lanyard. Per-round card
references and fingerprints survive a Finish overlay; a fresh recognizer
seeing Finish does not construct a colored template from the obscured art.
The shape icon is segmented relative to its own brightness, including its
dimmed Finish state. The host owns OCR and round reset.

Colored observations compare only transformed foreground cores, with coarse
360-degree search followed by one-degree angle, isotropic-scale and sub-cell
position refinement. A type needs valid observed pixels, a bounded score and
advantage over competing types. Covered pixels are excluded from the score.
Candidate rectangles are separate from observed cells; ambiguous coarse
placements produce no orientation constraint. Constraints are removed when
an anchor correction disagrees.

Completed objects use connected neutral-gray foreground silhouettes,
normalized mask intersection-over-union, rectangular grid bounds, positive
pixels in every constituent tile, and a texture-variance guard. Their
background is never rotated. Fully observed gray occupancy is `completed`;
`completed_objects.item_index` is optional. A missing Finish reference may
still produce an untyped whole occupied rectangle. A known-reference shape
mismatch, flat gray rectangle or incomplete colored object is not a complete
object. Conflicting manual corrections remove the object's metadata.

Empty recognition bootstraps a per-frame model from repeated exposed corner
colors, robust per-channel brightness fits and observed texture variation.
Local flat inserts fail the texture check. Automatic bootstrap is restricted
to light, nearly neutral surfaces: a dominant dark-gray or saturated object
cannot teach itself as background. Other backgrounds may need a trusted,
pixel-bound empty correction; that learned patch is cleared on reset. An
unmatched item alone never supplies empty evidence.

## Real frames and ground truth

| Frame | Covered | Empty | Colored observations | Completed objects |
| --- | ---: | --- | --- | --- |
| `vision-initial.png` | 45 | none | none | none |
| `vision-opened.png` | 44 | 15 | none | none |
| `vision-partial-watergun.png` | 41 | 11,15,29 | item0: 33 | none |
| `vision-full-watergun.png` | 36 | 11,15,29 | none | item0: 24,25,33,34,42,43 |
| `vision-other-items.png` | 25 | 11,15,17,29,31,37 | item0: 13; item1: 26,39 | item0: 24,25,33,34,42,43; item1: 1,10,19; item2: 6,7 |
| `vision-after-realtime-flip.png` | 24 | same six | previous plus item1: 21 | same; covered 23 has selection marker |
| `vision-realtime-unselected.png` | 24 | same six | same | same; selection cancelled |
| `vision-blue-selected.png` | 24 | same six | same | same; blue covered 22 selected |
| `vision-waterguns-finished.png` | 19 | same six | item1: 21,26,39 | previous plus item0: 4,5,13,14,22,23; card0 Finish |
| `vision-phones-completed.png` | 13 | same six | none | two guns, four phones, one sunscreen; 26 occupied cells |

The latest real completed-phone frame is copied byte-for-byte from
`compatibility_probe/artifacts/20260930T114048Z_32620b1d/latest.png`, captured
by the read-only WGC probe (13 sampled frames, no generated input). SHA-256:
`62c3cf99b50ba82ff53d7bb97af2e60b51c685f357f53134e5d50b52f4d07a6c`.

Its completed rectangles are:

- Guns: `(4,0,2,3)` and `(6,2,2,3)`.
- Phones: `(1,0,1,3)`, `(3,1,1,3)`, `(8,2,1,3)`, `(2,4,3,1)`.
- Sunscreen: `(6,0,2,1)`.

The old fragment-atlas method left phone cells 12,26,35,38,44 uncertain.
Current whole-card masks identify all four phones despite their different
ice texture and horizontal/vertical orientations. From a fresh Finish frame
the two guns are untyped occupied rectangles; loading an earlier active
card in the same round permits typed identity without changing occupancy.
The card counts are `[0,1,1]`; these remaining-piece counts do not label any
board cell and include partially revealed pieces.

Earlier captures retain their original provenance:
`20260930T103006Z_453821ac` (selection cancelled),
`20260930T104458Z_49b7c1de` (blue selected cover), and
`20260930T111237Z_7d1bfb6a` (completed guns, 14 sampled frames).

## Derived regression evidence and limits

Tests separately exercise 1920/1440/960 viewport resizing; the original
white-fragment negative; flat/partial selection-marker negatives; an entire
card sprite rotated by 17 degrees, resized and placed on a new card
background; exposed background color replacement while preserving gray
foreground; purple covers; flat gray rectangles; dominant gray/saturated
foreground corner samples; Finish cache/reset; and corrected metadata.

The all-Finish/zero-counter state is derived, not a newly played round. It
copies already confirmed empty pixels over the remaining covers and copies
only the real Finish stripes, preserving all three different shape icons.
A trustworthy zero counter allows fixed-layout observation without cover
anchors, but never creates empty or completed labels. A covered frame with
counter zero and a flat popup with counter zero are negative controls. The
next initial round restores covered cells and fresh references after reset.

The exploratory local Python/OpenCV scripts in `.artifacts/round-vision`
were used only to establish mask feasibility. The first prototype measured
true gray-mask IoUs of .946/.867 for guns, .893/.895/.840/.839 for phones,
and .934 for sunscreen. These are prototype values, not reported Rust
confidence scores. The shipped recognizer is Rust plus `image`, with no
Python/OpenCV runtime dependency.

Real evidence is one activity/round with multiple states. Synthetic rotation,
background replacement and recoloring are regression evidence, not physical
cross-activity or new-round acceptance. New art, unusually similar silhouettes,
a dark/saturated board theme, severe occlusion and animation can remain
uncertain and require correction. No claim of exhaustive cross-activity
recognition is made.

## Generated new-item/new-round regression

`vision::dynamic::generated_round_tests::generated_new_item_and_two_by_two_shape_replace_previous_round_reference`
uses a programmatically drawn asymmetric purple satellite with gold stripes,
not any of the three captured game sprites. It renders that item independently
at **23 degrees in card slot 0** and **73 degrees at board scale**, changes
slot 0's visible shape icon from **3x2 to 2x2**, and retains a real cover over
one of the four footprint cells. Cells 13,14,22 must match the new item; cell
23 and the other 41 covers must remain unknown. The test checks the new card
fingerprint after reset and the original fingerprint/3x2 icon after resetting
back to the original round.

Snapshot remaining-piece inputs change from `[2,5,2]` to `[3,1,2]`; another
`[7,9,4]` input must leave pixel observations identical. This verifies the
vision boundary and reset/reference switching, not OCR reading of synthetic
digits or a physical round transition. The generated sprite test passed with
the frozen production algorithm; no threshold or production API was changed.

Targeted verification: `cargo test --test vision_dynamic generated_new_item
-- --nocapture` — 1 passed, 0 failed (the previous 27 unique vision cases
remain unchanged). This adds one distinct case, for 28 unique vision cases.
