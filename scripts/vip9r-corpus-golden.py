#!/usr/bin/env python3
"""Strict wasm-golden over the md5-backed corpus, host d8, max concurrency.

Default is the hardcoded compliance set (~1 min wall on 32 cores). --all
ignores the list and runs every md5-backed file under /bulk/vip9r; wall time
is then dominated by the multi-thousand-frame movie/VOD clips.
"""
import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_ROOT = Path("/bulk/vip9r")

# Skipped even under --all: 72k frames of 720p60, ~14% of full-corpus decode
# cost by frames x pixels, nearly 2x the next-heaviest clip.
ALL_EXCLUDE = {
    "youtube/yUsYZ75JSw0/yUsYZ75JSw0-f302-720p60-vp9-rawprefix-0000-2000.webm",
}
RUNNER = REPO_ROOT / "js/dist/wasm-driver/golden.js"
TARGET_DIR = REPO_ROOT / "rust/target/wasm-release"
WASM = TARGET_DIR / "wasm32-unknown-unknown/release/vip9r.wasm"

# Every md5-backed corpus file except the movie/VOD perf clips (estimated
# >70s decode each on the host; everything here finishes in well under a
# minute). Regenerate from the md5 sidecars under /bulk/vip9r, estimating
# decode cost as frames x pixels, if the corpus or decoder speed changes
# materially.
COMPLIANCE = [
    "chromium/bear-vp9.ivf",
    "libvpx/vp90-2-00-quantizer-00.webm",
    "libvpx/vp90-2-00-quantizer-01.webm",
    "libvpx/vp90-2-00-quantizer-02.webm",
    "libvpx/vp90-2-00-quantizer-03.webm",
    "libvpx/vp90-2-00-quantizer-04.webm",
    "libvpx/vp90-2-00-quantizer-05.webm",
    "libvpx/vp90-2-00-quantizer-06.webm",
    "libvpx/vp90-2-00-quantizer-07.webm",
    "libvpx/vp90-2-00-quantizer-08.webm",
    "libvpx/vp90-2-00-quantizer-09.webm",
    "libvpx/vp90-2-00-quantizer-10.webm",
    "libvpx/vp90-2-00-quantizer-11.webm",
    "libvpx/vp90-2-00-quantizer-12.webm",
    "libvpx/vp90-2-00-quantizer-13.webm",
    "libvpx/vp90-2-00-quantizer-14.webm",
    "libvpx/vp90-2-00-quantizer-15.webm",
    "libvpx/vp90-2-00-quantizer-16.webm",
    "libvpx/vp90-2-00-quantizer-17.webm",
    "libvpx/vp90-2-00-quantizer-18.webm",
    "libvpx/vp90-2-00-quantizer-19.webm",
    "libvpx/vp90-2-00-quantizer-20.webm",
    "libvpx/vp90-2-00-quantizer-21.webm",
    "libvpx/vp90-2-00-quantizer-22.webm",
    "libvpx/vp90-2-00-quantizer-23.webm",
    "libvpx/vp90-2-00-quantizer-24.webm",
    "libvpx/vp90-2-00-quantizer-25.webm",
    "libvpx/vp90-2-00-quantizer-26.webm",
    "libvpx/vp90-2-00-quantizer-27.webm",
    "libvpx/vp90-2-00-quantizer-28.webm",
    "libvpx/vp90-2-00-quantizer-29.webm",
    "libvpx/vp90-2-00-quantizer-30.webm",
    "libvpx/vp90-2-00-quantizer-31.webm",
    "libvpx/vp90-2-00-quantizer-32.webm",
    "libvpx/vp90-2-00-quantizer-33.webm",
    "libvpx/vp90-2-00-quantizer-34.webm",
    "libvpx/vp90-2-00-quantizer-35.webm",
    "libvpx/vp90-2-00-quantizer-36.webm",
    "libvpx/vp90-2-00-quantizer-37.webm",
    "libvpx/vp90-2-00-quantizer-38.webm",
    "libvpx/vp90-2-00-quantizer-39.webm",
    "libvpx/vp90-2-00-quantizer-40.webm",
    "libvpx/vp90-2-00-quantizer-41.webm",
    "libvpx/vp90-2-00-quantizer-42.webm",
    "libvpx/vp90-2-00-quantizer-43.webm",
    "libvpx/vp90-2-00-quantizer-44.webm",
    "libvpx/vp90-2-00-quantizer-45.webm",
    "libvpx/vp90-2-00-quantizer-46.webm",
    "libvpx/vp90-2-00-quantizer-47.webm",
    "libvpx/vp90-2-00-quantizer-48.webm",
    "libvpx/vp90-2-00-quantizer-49.webm",
    "libvpx/vp90-2-00-quantizer-50.webm",
    "libvpx/vp90-2-00-quantizer-51.webm",
    "libvpx/vp90-2-00-quantizer-52.webm",
    "libvpx/vp90-2-00-quantizer-53.webm",
    "libvpx/vp90-2-00-quantizer-54.webm",
    "libvpx/vp90-2-00-quantizer-55.webm",
    "libvpx/vp90-2-00-quantizer-56.webm",
    "libvpx/vp90-2-00-quantizer-57.webm",
    "libvpx/vp90-2-00-quantizer-58.webm",
    "libvpx/vp90-2-00-quantizer-59.webm",
    "libvpx/vp90-2-00-quantizer-60.webm",
    "libvpx/vp90-2-00-quantizer-61.webm",
    "libvpx/vp90-2-00-quantizer-62.webm",
    "libvpx/vp90-2-00-quantizer-63.webm",
    "libvpx/vp90-2-01-sharpness-1.webm",
    "libvpx/vp90-2-01-sharpness-2.webm",
    "libvpx/vp90-2-01-sharpness-3.webm",
    "libvpx/vp90-2-01-sharpness-4.webm",
    "libvpx/vp90-2-01-sharpness-5.webm",
    "libvpx/vp90-2-01-sharpness-6.webm",
    "libvpx/vp90-2-01-sharpness-7.webm",
    "libvpx/vp90-2-02-size-08x08.webm",
    "libvpx/vp90-2-02-size-08x10.webm",
    "libvpx/vp90-2-02-size-08x16.webm",
    "libvpx/vp90-2-02-size-08x18.webm",
    "libvpx/vp90-2-02-size-08x32.webm",
    "libvpx/vp90-2-02-size-08x34.webm",
    "libvpx/vp90-2-02-size-08x64.webm",
    "libvpx/vp90-2-02-size-08x66.webm",
    "libvpx/vp90-2-02-size-10x08.webm",
    "libvpx/vp90-2-02-size-10x10.webm",
    "libvpx/vp90-2-02-size-10x16.webm",
    "libvpx/vp90-2-02-size-10x18.webm",
    "libvpx/vp90-2-02-size-10x32.webm",
    "libvpx/vp90-2-02-size-10x34.webm",
    "libvpx/vp90-2-02-size-10x64.webm",
    "libvpx/vp90-2-02-size-10x66.webm",
    "libvpx/vp90-2-02-size-130x132.webm",
    "libvpx/vp90-2-02-size-132x130.webm",
    "libvpx/vp90-2-02-size-132x132.webm",
    "libvpx/vp90-2-02-size-16x08.webm",
    "libvpx/vp90-2-02-size-16x10.webm",
    "libvpx/vp90-2-02-size-16x16.webm",
    "libvpx/vp90-2-02-size-16x18.webm",
    "libvpx/vp90-2-02-size-16x32.webm",
    "libvpx/vp90-2-02-size-16x34.webm",
    "libvpx/vp90-2-02-size-16x64.webm",
    "libvpx/vp90-2-02-size-16x66.webm",
    "libvpx/vp90-2-02-size-178x180.webm",
    "libvpx/vp90-2-02-size-180x178.webm",
    "libvpx/vp90-2-02-size-180x180.webm",
    "libvpx/vp90-2-02-size-18x08.webm",
    "libvpx/vp90-2-02-size-18x10.webm",
    "libvpx/vp90-2-02-size-18x16.webm",
    "libvpx/vp90-2-02-size-18x18.webm",
    "libvpx/vp90-2-02-size-18x32.webm",
    "libvpx/vp90-2-02-size-18x34.webm",
    "libvpx/vp90-2-02-size-18x64.webm",
    "libvpx/vp90-2-02-size-18x66.webm",
    "libvpx/vp90-2-02-size-32x08.webm",
    "libvpx/vp90-2-02-size-32x10.webm",
    "libvpx/vp90-2-02-size-32x16.webm",
    "libvpx/vp90-2-02-size-32x18.webm",
    "libvpx/vp90-2-02-size-32x32.webm",
    "libvpx/vp90-2-02-size-32x34.webm",
    "libvpx/vp90-2-02-size-32x64.webm",
    "libvpx/vp90-2-02-size-32x66.webm",
    "libvpx/vp90-2-02-size-34x08.webm",
    "libvpx/vp90-2-02-size-34x10.webm",
    "libvpx/vp90-2-02-size-34x16.webm",
    "libvpx/vp90-2-02-size-34x18.webm",
    "libvpx/vp90-2-02-size-34x32.webm",
    "libvpx/vp90-2-02-size-34x34.webm",
    "libvpx/vp90-2-02-size-34x64.webm",
    "libvpx/vp90-2-02-size-34x66.webm",
    "libvpx/vp90-2-02-size-64x08.webm",
    "libvpx/vp90-2-02-size-64x10.webm",
    "libvpx/vp90-2-02-size-64x16.webm",
    "libvpx/vp90-2-02-size-64x18.webm",
    "libvpx/vp90-2-02-size-64x32.webm",
    "libvpx/vp90-2-02-size-64x34.webm",
    "libvpx/vp90-2-02-size-64x64.webm",
    "libvpx/vp90-2-02-size-64x66.webm",
    "libvpx/vp90-2-02-size-66x08.webm",
    "libvpx/vp90-2-02-size-66x10.webm",
    "libvpx/vp90-2-02-size-66x16.webm",
    "libvpx/vp90-2-02-size-66x18.webm",
    "libvpx/vp90-2-02-size-66x32.webm",
    "libvpx/vp90-2-02-size-66x34.webm",
    "libvpx/vp90-2-02-size-66x64.webm",
    "libvpx/vp90-2-02-size-66x66.webm",
    "libvpx/vp90-2-02-size-lf-1920x1080.webm",
    "libvpx/vp90-2-03-deltaq.webm",
    "libvpx/vp90-2-03-size-196x196.webm",
    "libvpx/vp90-2-03-size-196x198.webm",
    "libvpx/vp90-2-03-size-196x200.webm",
    "libvpx/vp90-2-03-size-196x202.webm",
    "libvpx/vp90-2-03-size-196x208.webm",
    "libvpx/vp90-2-03-size-196x210.webm",
    "libvpx/vp90-2-03-size-196x224.webm",
    "libvpx/vp90-2-03-size-196x226.webm",
    "libvpx/vp90-2-03-size-198x196.webm",
    "libvpx/vp90-2-03-size-198x198.webm",
    "libvpx/vp90-2-03-size-198x200.webm",
    "libvpx/vp90-2-03-size-198x202.webm",
    "libvpx/vp90-2-03-size-198x208.webm",
    "libvpx/vp90-2-03-size-198x210.webm",
    "libvpx/vp90-2-03-size-198x224.webm",
    "libvpx/vp90-2-03-size-198x226.webm",
    "libvpx/vp90-2-03-size-200x196.webm",
    "libvpx/vp90-2-03-size-200x198.webm",
    "libvpx/vp90-2-03-size-200x200.webm",
    "libvpx/vp90-2-03-size-200x202.webm",
    "libvpx/vp90-2-03-size-200x208.webm",
    "libvpx/vp90-2-03-size-200x210.webm",
    "libvpx/vp90-2-03-size-200x224.webm",
    "libvpx/vp90-2-03-size-200x226.webm",
    "libvpx/vp90-2-03-size-202x196.webm",
    "libvpx/vp90-2-03-size-202x198.webm",
    "libvpx/vp90-2-03-size-202x200.webm",
    "libvpx/vp90-2-03-size-202x202.webm",
    "libvpx/vp90-2-03-size-202x208.webm",
    "libvpx/vp90-2-03-size-202x210.webm",
    "libvpx/vp90-2-03-size-202x224.webm",
    "libvpx/vp90-2-03-size-202x226.webm",
    "libvpx/vp90-2-03-size-208x196.webm",
    "libvpx/vp90-2-03-size-208x198.webm",
    "libvpx/vp90-2-03-size-208x200.webm",
    "libvpx/vp90-2-03-size-208x202.webm",
    "libvpx/vp90-2-03-size-208x208.webm",
    "libvpx/vp90-2-03-size-208x210.webm",
    "libvpx/vp90-2-03-size-208x224.webm",
    "libvpx/vp90-2-03-size-208x226.webm",
    "libvpx/vp90-2-03-size-210x196.webm",
    "libvpx/vp90-2-03-size-210x198.webm",
    "libvpx/vp90-2-03-size-210x200.webm",
    "libvpx/vp90-2-03-size-210x202.webm",
    "libvpx/vp90-2-03-size-210x208.webm",
    "libvpx/vp90-2-03-size-210x210.webm",
    "libvpx/vp90-2-03-size-210x224.webm",
    "libvpx/vp90-2-03-size-210x226.webm",
    "libvpx/vp90-2-03-size-224x196.webm",
    "libvpx/vp90-2-03-size-224x198.webm",
    "libvpx/vp90-2-03-size-224x200.webm",
    "libvpx/vp90-2-03-size-224x202.webm",
    "libvpx/vp90-2-03-size-224x208.webm",
    "libvpx/vp90-2-03-size-224x210.webm",
    "libvpx/vp90-2-03-size-224x224.webm",
    "libvpx/vp90-2-03-size-224x226.webm",
    "libvpx/vp90-2-03-size-226x196.webm",
    "libvpx/vp90-2-03-size-226x198.webm",
    "libvpx/vp90-2-03-size-226x200.webm",
    "libvpx/vp90-2-03-size-226x202.webm",
    "libvpx/vp90-2-03-size-226x208.webm",
    "libvpx/vp90-2-03-size-226x210.webm",
    "libvpx/vp90-2-03-size-226x224.webm",
    "libvpx/vp90-2-03-size-226x226.webm",
    "libvpx/vp90-2-03-size-352x288.webm",
    "libvpx/vp90-2-05-resize.ivf",
    "libvpx/vp90-2-06-bilinear.webm",
    "libvpx/vp90-2-07-frame_parallel-1.webm",
    "libvpx/vp90-2-07-frame_parallel.webm",
    "libvpx/vp90-2-08-tile-4x1.webm",
    "libvpx/vp90-2-08-tile-4x4.webm",
    "libvpx/vp90-2-08-tile_1x2.webm",
    "libvpx/vp90-2-08-tile_1x2_frame_parallel.webm",
    "libvpx/vp90-2-08-tile_1x4.webm",
    "libvpx/vp90-2-08-tile_1x4_frame_parallel.webm",
    "libvpx/vp90-2-08-tile_1x8.webm",
    "libvpx/vp90-2-08-tile_1x8_frame_parallel.webm",
    "libvpx/vp90-2-09-aq2.webm",
    "libvpx/vp90-2-09-lf_deltas.webm",
    "libvpx/vp90-2-09-subpixel-00.ivf",
    "libvpx/vp90-2-10-show-existing-frame.webm",
    "libvpx/vp90-2-10-show-existing-frame2.webm",
    "libvpx/vp90-2-11-size-351x287.webm",
    "libvpx/vp90-2-11-size-351x288.webm",
    "libvpx/vp90-2-11-size-352x287.webm",
    "libvpx/vp90-2-12-droppable_1.ivf",
    "libvpx/vp90-2-12-droppable_2.ivf",
    "libvpx/vp90-2-12-droppable_3.ivf",
    "libvpx/vp90-2-13-largescaling.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-1-2-4-8.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-1-2.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-1-4.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-1-8.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-2-1.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-2-4.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-2-8.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-4-1.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-4-2.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-4-8.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-8-1.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-8-2.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-8-4-2-1.webm",
    "libvpx/vp90-2-14-resize-10frames-fp-tiles-8-4.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-1-16.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-1-2-4-8-16.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-1-2.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-1-4.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-1-8.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-16-1.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-16-2.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-16-4.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-16-8-4-2-1.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-16-8.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-2-1.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-2-16.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-2-4.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-2-8.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-4-1.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-4-16.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-4-2.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-4-8.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-8-1.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-8-16.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-8-2.webm",
    "libvpx/vp90-2-14-resize-fp-tiles-8-4.webm",
    "libvpx/vp90-2-15-segkey.webm",
    "libvpx/vp90-2-15-segkey_adpq.webm",
    "libvpx/vp90-2-16-intra-only.webm",
    "libvpx/vp90-2-17-show-existing-frame.webm",
    "libvpx/vp90-2-18-resize.ivf",
    "libvpx/vp90-2-19-skip-01.webm",
    "libvpx/vp90-2-19-skip-02.webm",
    "libvpx/vp90-2-19-skip.webm",
    "libvpx/vp90-2-20-big_superframe-01.webm",
    "libvpx/vp90-2-20-big_superframe-02.webm",
    "libvpx/vp90-2-21-resize_inter_1280x720_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_1280x720_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_1280x720_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_1280x720_7_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_1920x1080_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_1920x1080_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_1920x1080_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_1920x1080_7_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_320x180_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_320x180_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_320x180_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_320x180_7_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_320x240_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_320x240_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_320x240_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_320x240_7_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_640x360_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_640x360_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_640x360_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_640x360_7_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_640x480_5_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_640x480_5_3-4.webm",
    "libvpx/vp90-2-21-resize_inter_640x480_7_1-2.webm",
    "libvpx/vp90-2-21-resize_inter_640x480_7_3-4.webm",
    "libvpx/vp90-2-22-svc_1280x720_1.webm",
    "libvpx/vp90-2-22-svc_1280x720_3.ivf",
    "realworld/test-videos/jellyfish-720p30-1_68mbps.webm",
    "realworld/wikimedia/big-buck-bunny-720p25-1_54mbps.webm",
]


def discover_media(run_all: bool) -> list[Path]:
    if run_all:
        media = [
            sidecar.with_suffix("")
            for sidecar in CORPUS_ROOT.rglob("*.md5")
            if sidecar.with_suffix("").is_file()
            and str(sidecar.with_suffix("").relative_to(CORPUS_ROOT))
            not in ALL_EXCLUDE
        ]
    else:
        media = [CORPUS_ROOT / rel for rel in COMPLIANCE]
        missing = [path for path in media if not path.is_file()]
        if missing:
            listing = "\n  ".join(str(path) for path in missing)
            sys.exit(f"compliance media missing from corpus:\n  {listing}")
    # Heaviest first so the long pole starts immediately; size is a good
    # enough decode-cost proxy for scheduling.
    media.sort(key=lambda p: p.stat().st_size, reverse=True)
    return media


def build_wasm() -> None:
    subprocess.run(
        [
            "cargo", "build", "--quiet",
            "--manifest-path", str(REPO_ROOT / "rust/Cargo.toml"),
            "--target-dir", str(TARGET_DIR),
            "--target", "wasm32-unknown-unknown",
            "-p", "vip9r", "--release",
        ],
        check=True,
        # cwd must be the workspace: cargo resolves .cargo/config.toml (which
        # carries target-feature flags) from cwd, not --manifest-path.
        cwd=REPO_ROOT / "rust",
        # Threaded-wasm build: stable cargo honors the [unstable] build-std
        # table in .cargo/config.toml only with this in its environment.
        env={**os.environ, "RUSTC_BOOTSTRAP": "1"},
    )


def run_one(d8: str, media: Path) -> tuple[Path, bool, str]:
    proc = subprocess.run(
        [d8, "--no-liftoff", "--module", str(RUNNER), "--", str(WASM), str(media)],
        capture_output=True,
        text=True,
    )
    detail = ""
    try:
        report = json.loads(proc.stdout.strip().splitlines()[-1])
        ok = bool(report.get("ok"))
        if not ok:
            detail = json.dumps(
                {
                    key: report[key]
                    for key in (
                        "error",
                        "matchedCount",
                        "mismatchedCount",
                        "missingCount",
                        "extraCount",
                        "mismatches",
                    )
                    if key in report and report[key]
                },
            )
    except (json.JSONDecodeError, IndexError):
        ok = False
        detail = f"unparseable driver output (exit {proc.returncode})"
    if not ok and proc.stderr.strip():
        tail = proc.stderr.strip().splitlines()[-5:]
        detail += "\n  " + "\n  ".join(tail)
    return media, ok, detail


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="vip9r-corpus-golden",
        description="Run strict wasm-golden across the md5-backed corpus.",
    )
    parser.add_argument(
        "--all",
        action="store_true",
        help="include the heavy movie/VOD clips",
    )
    parser.add_argument(
        "-j",
        "--jobs",
        type=int,
        default=os.cpu_count(),
        help="concurrent d8 processes (default: all cores)",
    )
    args = parser.parse_args()

    d8 = os.environ.get("D8_LINUX64")
    if not d8:
        print("D8_LINUX64 is not set; run inside the dev shell", file=sys.stderr)
        return 2
    if not RUNNER.is_file():
        print(f"missing prebuilt wasm driver: {RUNNER}", file=sys.stderr)
        print("run: cd <project>/js && pnpm build:wasm-driver", file=sys.stderr)
        return 2

    media = discover_media(args.all)
    if not media:
        print(f"no md5-backed media under {CORPUS_ROOT}", file=sys.stderr)
        return 2

    build_wasm()

    label = "full corpus" if args.all else "compliance set"
    print(f"{label}: {len(media)} files, {args.jobs} jobs")
    started = time.monotonic()
    failures = []
    done = 0
    midline = False
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(run_one, d8, path) for path in media]
        for future in concurrent.futures.as_completed(futures):
            path, ok, detail = future.result()
            done += 1
            if ok:
                if done % 5 == 0:
                    print(".", end="", flush=True)
                    midline = True
            else:
                rel = path.relative_to(CORPUS_ROOT)
                failures.append((rel, detail))
                if midline:
                    print()
                    midline = False
                print(f"[{done}/{len(media)}] FAIL {rel}")
    if midline:
        print()

    elapsed = time.monotonic() - started
    if failures:
        print(f"\n{len(failures)} failure(s) in {elapsed:.0f}s:")
        for rel, detail in failures:
            print(f"  {rel}\n    {detail}")
        return 1
    print(f"\n{len(media)}/{len(media)} ok in {elapsed:.0f}s")
    return 0


if __name__ == "__main__":
    sys.exit(main())
