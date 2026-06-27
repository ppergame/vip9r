# vip9r — implementation log

Human-facing narrative of decoder implementation and optimization progress.
Compact and legible. Record accepted implementation blocks, correctness
milestones, and measured optimization results. Each entry should cite the VCS
rev/change it describes. Keep routine task mechanics in `docs/design.md` or
`docs/tracker.md`, not here.

## 2026-06-22 — VP9 packet front end

- rev: jj `nurttsnw`
- Added the first grinder-produced decoder block: fixed-bit parsing,
  superframe splitting, and VP9 profile 0 / 8-bit uncompressed header parsing
  through `header_size_in_bytes`.
- `Decoder::decode_packet` now parses coded-frame packet structure and reference
  dimensions before stopping at the expected compressed-header/tile
  `Unimplemented` boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 tile payload layout

- rev: jj `wzkluspu`
- Added allocation-free tile payload layout validation after uncompressed header
  parsing: MI bounds, raster tile descriptors, and non-final little-endian tile
  size prefixes.
- `Decoder::decode_packet` now validates all coded-frame headers and tile byte
  ranges against scratch parser state before committing persistent reference
  dimensions, then stops at the expected tile-decode `Unimplemented` boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 boolean decoder primitive

- rev: jj `zrsknxmt`
- Added the internal allocation-free VP9 boolean decoder primitive for future
  compressed-header and tile syntax parsing: init marker validation,
  probability-coded bools, literals, renormalization underflow checks, and exit
  padding validation.
- The primitive is intentionally not wired into packet decode yet, so decoder
  behavior is unchanged at the public boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 entropy header prerequisites

- rev: jj `opnyvxto`
- Retained uncompressed-header fields needed by compressed-header parsing:
  frame-context flags/index, high-precision MV, interpolation filter,
  quantizer deltas, and `lossless`.
- The parser still does not parse compressed headers or tile contents; the
  public decode boundary remains unchanged.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra compressed headers

- rev: jj `qpvprvyy`
- Added the first compressed-header block for key and intra-only frames:
  transform mode parsing, tx/skip/coef probability updates, subexponential
  probability remapping, and retained default frame probability state.
- `Decoder::decode_packet` now consumes valid intra compressed headers before
  validating tile layout and stopping at the expected tile-decode
  `Unimplemented` boundary. Inter compressed headers remain intentionally
  unparsed for a later block; prior tile-layout validation for inter frames is
  preserved.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra tile mode info

- rev: jj `kmtukkoy`
- Added key/intra tile boolean syntax through the residual handoff: partition
  tree decoding, fixed key-frame partition/Y/UV probability tables, above/left
  partition and mode contexts, skip and transform-size parsing, and
  segmentation-disabled intra mode-info parsing.
- `Decoder::decode_packet` now enters key/intra tile payloads after tile-layout
  validation and stops at the expected residual `Unimplemented` boundary.
  Segmentation-enabled intra tile syntax is explicitly unimplemented rather than
  silently misparsed.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra residual tokens

- rev: jj `nsvwpklx`
- Added parse-only residual traversal for segmentation-disabled key/intra
  frames: UV transform sizing, plane block sizing, scan selection, coefficient
  token parsing, extra coefficient bits, sign-bit consumption, and above/left
  nonzero contexts.
- `parse_intra_tiles` now consumes the full tile bool stream for the first
  `bear-vp9.ivf` key frame and returns to the packet-level
  `Unimplemented` boundary. Prediction, inverse transform, reconstruction, loop
  filter, reference storage, and output frames remain unimplemented.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 inter compressed headers

- rev: jj `rtoszqvl`
- Added inter-frame compressed-header parsing through non-coef probability
  updates: inter mode, switchable interpolation filter, intra/inter, reference
  mode/reference probabilities, Y mode, partition, and MV probabilities.
- Extended frame probability state with the spec 10.5 default inter/non-coef/MV
  tables and wired `Decoder::decode_packet` to parse inter compressed headers
  before the still-unimplemented inter tile syntax boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-27 — VP9 inter tile syntax

- rev: jj `tpsrpqpp`
- Added parse-only inter-frame tile traversal: inter partition probabilities,
  mode-info syntax, reference-frame selection, interpolation filters, motion
  vector syntax/prediction scaffolding, intra blocks inside inter frames, and
  residual token parsing with the inter coefficient context.
- `Decoder::decode_coded_frame` now parses the first `bear-vp9.ivf` key frame
  and first inter frame tile syntax before stopping at the next honest boundary:
  non-frame-parallel inter probability adaptation needs syntax counts. Pixel
  prediction, reconstruction, loop filtering, reference pixels, and shown-frame
  output remain unwired.
- Golden smoke and strict runs on `bear-vp9.ivf` now stop at
  `decode packet 1 coded frame 0: Unimplemented`.

## 2026-06-27 — VP9 syntax counts and probability refresh

- rev: jj `zoyuowll`
- Added syntax count accumulation for the parse-only tile path and VP9 backward
  probability refresh for coefficient, non-coefficient, and MV probability
  state.
- `Decoder::decode_coded_frame` now removes the prior non-frame-parallel inter
  adaptation `Unimplemented` boundary, tracks previous frame type for the
  coefficient update factor, and refreshes frame contexts after tile parsing.
- Golden smoke on `bear-vp9.ivf` advances past packet 1 and now stops at
  `decode packet 17 coded frame 0: InvalidBitstream`.

## 2026-06-27 — VP9 previous-frame MV candidates

- rev: jj `ukpxsptp`
- Added previous-frame mode/MV history for the host tile parser and wired
  `UsePrevFrameMvs` into inter-frame MV reference discovery.
- `Decoder::decode_coded_frame` now records the current frame's per-MI
  reference/MV state after successful tile parsing and supplies it to the next
  eligible inter frame.
- Golden smoke on `bear-vp9.ivf` advances past packet 17 and now stops at
  `decode packet 25 coded frame 0: InvalidBitstream`.

## 2026-06-27 — VP9 exact mode history for MV refs

- rev: jj `upsntwvy`
- Replaced the 1-D above/left-context approximation for current-frame MV
  reference candidates with exact per-MI mode-history lookup when that grid is
  available, and retained sub-block MVs for sub-8x8 candidate extraction.
- Host golden smoke on `bear-vp9.ivf` now parses all 82 coded frames. It exits
  at the next expected M1 boundary: no shown frames are output yet, so the
  harness reports 82 missing frames. Wasm/no-std parity still waits on moving
  mode-history storage into workspace state.

## 2026-06-27 — VP9 mode history in workspace

- rev: jj `rysswwlv`
- Moved previous/current frame mode-history storage out of std-only `Vec`s and
  into `DecodeWorkspace` as two packed byte slots, so host and no-std/wasm use
  the same previous-frame MV candidate path.
- Reduced tile above-context storage to a column cap sized for project targets
  instead of 8192 MI columns, fixing a wasm stack trap in the d8 smoke path.
- Host and wasm smoke runs on `bear-vp9.ivf` now both parse all 82 coded frames
  and exit at the expected M1 boundary: no shown frames are output yet, so both
  report 82 missing frames.
