#!/usr/bin/env python3
"""Collect the ffvp9 decode baseline: static ffmpeg run directly through adb.

For each target (host + every connected device by default) the run matrix is
derived from CPU topology: one single-threaded run pinned to a representative
core of each core type, one multi-threaded run across each multi-core type,
and one across all online cores when the device is heterogeneous. Host runs
are unpinned (host numbers are a convenience, not the record).

Timing is ffmpeg's own -benchmark rtime over decoded frames, so it includes
process startup and demux; with the default 600-frame window that overhead is
noise. ms/frame is best-of `--repeat` (most favorable sample), with the
max/min spread reported so drifty runs are visible. This bypasses the perf
daemon queue on purpose (the baseline is ffmpeg, not wasm/d8): don't run it
while daemon device jobs are in flight.
"""

import argparse
from dataclasses import dataclass
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
import time

SCRIPTS_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPTS_DIR.parent

_spec = importlib.util.spec_from_file_location(
    "vip9r_perf_daemon", SCRIPTS_DIR / "vip9r-perf-daemon.py"
)
daemon = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(daemon)

MEDIA_ROOT = daemon.MEDIA_ROOT
DEVICE_ROOT = daemon.DEFAULT_DEVICE_ROOT
RUN_TIMEOUT_SECONDS = 900
BENCH_RE = re.compile(
    r"bench: utime=(?P<utime>[\d.]+)s stime=(?P<stime>[\d.]+)s rtime=(?P<rtime>[\d.]+)s"
)
FRAME_RE = re.compile(r"frame=\s*(\d+)")


@dataclass
class RunConfig:
    label: str
    threads: int
    cpus: list[int] | None  # None = unpinned


@dataclass
class Sample:
    rtime_s: float
    utime_s: float
    stime_s: float
    frames: int
    temp_start_c: float | None

    @property
    def ms_per_frame(self) -> float:
        return self.rtime_s * 1000.0 / self.frames


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="ffvp9-baseline",
        description="ffvp9 decode baseline on host and adb devices",
    )
    parser.add_argument(
        "--target",
        action="append",
        default=[],
        help="'host' or an adb serial; repeat to restrict, default host + all devices",
    )
    parser.add_argument(
        "--clip",
        action="append",
        default=[],
        help="corpus-relative media path; default all realworld/**/*.webm",
    )
    parser.add_argument(
        "--frames",
        type=int,
        default=600,
        help="output frame cap per run, 0 = whole clip (default 600)",
    )
    parser.add_argument(
        "--repeat",
        type=int,
        default=2,
        help="samples per cell, best-of reported (default 2)",
    )
    parser.add_argument("--ffmpeg-host", default=os.environ.get("FFMPEG_VP9_LINUX64"))
    parser.add_argument("--ffmpeg-arm64", default=os.environ.get("FFMPEG_VP9_ANDROID_ARM64"))
    parser.add_argument("--ffmpeg-arm32", default=os.environ.get("FFMPEG_VP9_ANDROID_ARM32"))
    parser.add_argument(
        "--out",
        type=Path,
        default=None,
        help="JSON result path (default temp/perf/ffvp9-baseline-<utc>.json)",
    )
    return parser


def default_clips() -> list[str]:
    clips = sorted(
        str(path.relative_to(MEDIA_ROOT))
        for path in (MEDIA_ROOT / "realworld").rglob("*.webm")
    )
    if not clips:
        raise RuntimeError(f"no realworld clips under {MEDIA_ROOT}")
    return clips


def ffmpeg_args(threads: int, media: str, frames: int) -> list[str]:
    args = ["-nostdin", "-benchmark", "-threads", str(threads), "-i", media]
    if frames > 0:
        args += ["-frames:v", str(frames)]
    args += ["-map", "0:v:0", "-f", "rawvideo", "-y", "/dev/null"]
    return args


def parse_run(stderr: str, context: str) -> tuple[float, float, float, int]:
    bench = None
    for bench in BENCH_RE.finditer(stderr):
        pass
    frames = [int(match.group(1)) for match in FRAME_RE.finditer(stderr)]
    if bench is None or not frames or frames[-1] <= 0:
        tail = stderr[-2000:]
        raise RuntimeError(f"{context}: no bench/frame output in ffmpeg stderr:\n{tail}")
    return (
        float(bench.group("rtime")),
        float(bench.group("utime")),
        float(bench.group("stime")),
        frames[-1],
    )


# --- host ---


def host_configs() -> list[RunConfig]:
    cores = os.cpu_count() or 1
    configs = [RunConfig("host x1t", 1, None)]
    if cores > 1:
        configs.append(RunConfig(f"host x{cores}t", cores, None))
    return configs


def run_host_sample(ffmpeg: str, config: RunConfig, media: str, frames: int) -> Sample:
    completed = subprocess.run(
        [ffmpeg, *ffmpeg_args(config.threads, media, frames)],
        capture_output=True,
        text=True,
        check=False,
        timeout=RUN_TIMEOUT_SECONDS,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"host ffmpeg exit {completed.returncode}:\n{completed.stderr[-2000:]}")
    rtime, utime, stime, out_frames = parse_run(completed.stderr, "host")
    return Sample(rtime, utime, stime, out_frames, None)


# --- device ---


def select_device_ffmpeg(device, args: argparse.Namespace) -> str:
    abis = ",".join(
        filter(None, [device.props.get("abi", ""), device.props.get("abilist", "")])
    )
    if "arm64-v8a" in abis:
        path, flag = args.ffmpeg_arm64, "--ffmpeg-arm64 / FFMPEG_VP9_ANDROID_ARM64"
    elif "armeabi" in abis:
        path, flag = args.ffmpeg_arm32, "--ffmpeg-arm32 / FFMPEG_VP9_ANDROID_ARM32"
    else:
        raise RuntimeError(f"{device.serial}: unsupported ABI list {abis!r}")
    if not path:
        raise RuntimeError(f"{device.serial}: needs {flag}")
    return path


def device_configs(device) -> list[RunConfig]:
    groups = daemon.cpu_groups(device)
    configs: list[RunConfig] = []
    for group in groups:
        cpus = group["cpus"]
        name = group["name"]
        configs.append(RunConfig(f"{name} (cpu{cpus[0]}) x1t", 1, [cpus[0]]))
        if len(cpus) > 1:
            configs.append(
                RunConfig(f"{name} (cpu{daemon.format_cpu_list(cpus)}) x{len(cpus)}t", len(cpus), cpus)
            )
    if len(groups) > 1 and len(device.online_cpus) > 1:
        configs.append(
            RunConfig(f"all cores x{len(device.online_cpus)}t", len(device.online_cpus), device.online_cpus)
        )
    return configs


def push_device_ffmpeg(device, local_path: Path) -> str:
    if device.taskset is None:
        raise RuntimeError(f"{device.serial}: no taskset on device")
    data = Path(local_path).read_bytes()
    remote = str(DEVICE_ROOT / "ffvp9" / f"ffmpeg-{hashlib.sha256(data).hexdigest()[:16]}")
    if daemon.missing_remote_files(device, [(remote, "-x")]):
        daemon.ensure_remote_dir(device, str(DEVICE_ROOT / "ffvp9"))
        daemon.push_file_to_device(device, Path(local_path), remote)
        completed = daemon.adb_shell_completed(device.serial, f"chmod 755 {shlex.quote(remote)}")
        if completed.returncode != 0:
            raise RuntimeError(f"chmod device ffmpeg: {completed.stderr or completed.stdout}".strip())
    return remote


def run_device_sample(
    device, remote_ffmpeg: str, config: RunConfig, media: str, frames: int
) -> Sample:
    temp_start = daemon.read_cpu_temp_c(device)
    assert config.cpus
    mask = f"{daemon.mask_from_cpus(config.cpus):x}"
    args = " ".join(shlex.quote(arg) for arg in ffmpeg_args(config.threads, media, frames))
    script = (
        f"timeout {RUN_TIMEOUT_SECONDS} taskset {mask} {shlex.quote(remote_ffmpeg)} {args}"
    )
    completed = daemon.adb_shell_completed(
        device.serial, script, timeout=RUN_TIMEOUT_SECONDS + 30
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"{device.serial} ffmpeg exit {completed.returncode}:\n"
            f"{(completed.stderr or completed.stdout)[-2000:]}"
        )
    rtime, utime, stime, out_frames = parse_run(
        completed.stderr + completed.stdout, device.serial
    )
    return Sample(rtime, utime, stime, out_frames, temp_start)


# --- orchestration ---


def collect_target(
    target: str, args: argparse.Namespace, clips: list[str]
) -> dict[str, object]:
    if target == "host":
        if not args.ffmpeg_host:
            raise RuntimeError("host needs --ffmpeg-host / FFMPEG_VP9_LINUX64")
        label = "host"
        configs = host_configs()

        def run(config: RunConfig, clip: str) -> Sample:
            return run_host_sample(args.ffmpeg_host, config, str(MEDIA_ROOT / clip), args.frames)

    else:
        device = daemon.probe_device(target, DEVICE_ROOT, require_runtime=False)
        label = f"{device.props.get('model', target)} ({target})"
        remote_ffmpeg = push_device_ffmpeg(device, Path(select_device_ffmpeg(device, args)))
        media_paths = [str(DEVICE_ROOT / "media" / clip) for clip in clips]
        missing = daemon.missing_remote_files(device, [(path, "-f") for path in media_paths])
        if missing:
            raise RuntimeError(
                f"{target}: missing device media (run vip9r-perf-daemon prepare): "
                + ", ".join(missing)
            )
        configs = device_configs(device)

        def run(config: RunConfig, clip: str) -> Sample:
            return run_device_sample(
                device, remote_ffmpeg, config, str(DEVICE_ROOT / "media" / clip), args.frames
            )

    cells: list[dict[str, object]] = []
    for clip in clips:
        for config in configs:
            samples = [run(config, clip) for _ in range(args.repeat)]
            best = min(samples, key=lambda sample: sample.ms_per_frame)
            worst = max(samples, key=lambda sample: sample.ms_per_frame)
            cells.append(
                {
                    "clip": clip,
                    "config": config.label,
                    "threads": config.threads,
                    "cpus": config.cpus,
                    "frames": best.frames,
                    "ms_per_frame": best.ms_per_frame,
                    "fps": best.frames / best.rtime_s,
                    "spread": worst.ms_per_frame / best.ms_per_frame - 1,
                    "samples": [
                        {
                            "rtime_s": sample.rtime_s,
                            "utime_s": sample.utime_s,
                            "stime_s": sample.stime_s,
                            "frames": sample.frames,
                            "temp_start_c": sample.temp_start_c,
                        }
                        for sample in samples
                    ],
                }
            )
            print(
                f"  {label}: {Path(clip).name} | {config.label} | "
                f"{best.ms_per_frame:.2f} ms/frame ({best.frames / best.rtime_s:.1f} fps, "
                f"spread {100 * (worst.ms_per_frame / best.ms_per_frame - 1):.1f}%)",
                file=sys.stderr,
                flush=True,
            )
    return {"target": target, "label": label, "cells": cells}


def print_table(results: list[dict[str, object]]) -> None:
    for result in results:
        print(f"\n=== {result['label']} ===")
        rows = [("clip", "config", "ms/frame", "fps", "spread", "temp0")]
        for cell in result["cells"]:
            temp = cell["samples"][0]["temp_start_c"]
            rows.append(
                (
                    Path(cell["clip"]).name,
                    cell["config"],
                    f"{cell['ms_per_frame']:.2f}",
                    f"{cell['fps']:.1f}",
                    f"{100 * cell['spread']:.1f}%",
                    f"{temp:.0f}C" if temp is not None else "-",
                )
            )
        widths = [max(len(row[i]) for row in rows) for i in range(len(rows[0]))]
        for row in rows:
            print("  ".join(field.ljust(width) for width, field in zip(widths, row)))


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    clips = args.clip or default_clips()

    targets = args.target
    if not targets:
        targets = ["host"] + [
            serial for serial, state in daemon.adb_devices() if state == "device"
        ]
    if len(targets) != len(set(targets)):
        print("duplicate --target", file=sys.stderr)
        return 2

    print(f"targets: {', '.join(targets)}", file=sys.stderr)
    print(f"clips: {', '.join(Path(clip).name for clip in clips)}", file=sys.stderr)
    print(
        f"frames: {args.frames or 'all'}, repeat: {args.repeat} (best-of reported)",
        file=sys.stderr,
    )

    results: list[dict[str, object]] = []
    errors: list[str] = []
    with ThreadPoolExecutor(max_workers=len(targets)) as pool:
        futures = {
            pool.submit(collect_target, target, args, clips): target for target in targets
        }
        for future, target in futures.items():
            try:
                results.append(future.result())
            except Exception as error:
                errors.append(f"{target}: {error}")

    results.sort(key=lambda result: targets.index(result["target"]))
    print_table(results)
    for error in errors:
        print(f"\nFAILED {error}", file=sys.stderr)

    out = args.out or REPO_ROOT / "temp/perf" / (
        f"ffvp9-baseline-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}.json"
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(
            {
                "frames": args.frames,
                "repeat": args.repeat,
                "clips": clips,
                "results": results,
                "errors": errors,
            },
            indent=2,
        )
        + "\n"
    )
    print(f"\nresults: {out}")
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
