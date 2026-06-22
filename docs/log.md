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
