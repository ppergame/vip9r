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
