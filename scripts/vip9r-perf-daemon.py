#!/usr/bin/env python3
import argparse
from collections import deque
from dataclasses import dataclass
import errno
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import time

SOCKET_PATH = Path("temp/vip9r-perf.sock")
REPO_ROOT = Path(__file__).resolve().parents[1]
GOLDEN_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/golden.js"
MICROBENCH_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/microbench.js"
MEDIA_ROOT = Path("/bulk/vip9r")
DEFAULT_DEVICE_ROOT = PurePosixPath("/data/local/tmp/vip9r")
MAX_REQUEST_BYTES = 64 * 1024
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
U32_MAX = 2**32 - 1
PIN_HELP = "any, all, cpu:N, or mask:HEX"


class DeviceError(RuntimeError):
    pass


@dataclass
class Job:
    conn: socket.socket
    request: object
    candidate: bytes


@dataclass
class CpuInfo:
    index: int
    capacity: int | None
    max_freq: int | None
    midr: str | None


@dataclass
class PinSelection:
    request: str
    mask: str | None
    cpus: list[int] | None
    verified_cpus: list[int] | None = None


@dataclass
class DeviceContext:
    serial: str
    root: PurePosixPath
    props: dict[str, str]
    cpus: list[CpuInfo]
    online_cpus: list[int]
    taskset: str | None
    d8_version: str


class JobQueue:
    def __init__(self) -> None:
        self._ready = threading.Condition()
        self._pending: deque[Job] = deque()
        self._running: Job | None = None

    def enqueue(self, job: Job) -> None:
        with self._ready:
            self._pending.append(job)
            self._send_queued_updates_locked()
            self._ready.notify()

    def pop(self) -> Job:
        with self._ready:
            while not self._pending:
                self._ready.wait()
            job = self._pending.popleft()
            self._running = job
            self._send_queued_updates_locked()
            return job

    def finish(self, job: Job) -> None:
        with self._ready:
            if self._running is job:
                self._running = None
                self._send_queued_updates_locked()

    def _send_queued_updates_locked(self) -> None:
        live: deque[Job] = deque()
        running_count = 1 if self._running is not None else 0
        for index, job in enumerate(self._pending):
            try:
                send_json(job.conn, {"type": "queued", "ahead": running_count + index})
                live.append(job)
            except OSError:
                job.conn.close()
        self._pending = live


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="vip9r-perf-daemon",
        description="Queue vip9r performance runs.",
    )
    parser.add_argument(
        "--baseline",
        required=True,
        type=Path,
        help="baseline wasm module built by the orchestrator",
    )
    parser.add_argument(
        "--serial",
        help="adb device serial; omit to run host-only",
    )
    parser.add_argument(
        "--pin",
        default="any",
        help=f"default device CPU pin: {PIN_HELP}",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.serial is None and args.pin != "any":
        parser.error("--pin requires --serial")

    baseline_bytes = args.baseline.read_bytes()
    baseline_fd = memfd_from_bytes("vip9r-baseline.wasm", baseline_bytes)
    device: DeviceContext | None = None
    if args.serial is not None:
        try:
            device = probe_device(args.serial, DEFAULT_DEVICE_ROOT)
            resolve_pin(device, args.pin)
        except Exception as error:
            os.close(baseline_fd)
            print(f"device setup: {error}", file=sys.stderr)
            return 2

    SOCKET_PATH.parent.mkdir(parents=True, exist_ok=True)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_CANDIDATE_BYTES)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_RESPONSE_BYTES)
    try:
        server.bind(str(SOCKET_PATH))
    except OSError as error:
        if error.errno == errno.EADDRINUSE:
            print(f"socket already exists: {SOCKET_PATH}", file=sys.stderr)
            return 2
        raise

    jobs = JobQueue()
    worker = threading.Thread(
        target=run_worker,
        args=(args, jobs, baseline_fd, baseline_bytes, device),
        daemon=True,
    )
    worker.start()

    try:
        server.listen()
        print(
            json.dumps(
                {
                    "baseline": str(args.baseline),
                    "serial": args.serial,
                    "pin": args.pin,
                    "socket": str(SOCKET_PATH),
                    "device": device_summary(device) if device is not None else None,
                },
                indent=2,
            ),
            flush=True,
        )
        while True:
            conn, _ = server.accept()
            conn.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_CANDIDATE_BYTES)
            conn.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_RESPONSE_BYTES)
            accept_one(conn, jobs)
    except KeyboardInterrupt:
        return 130
    finally:
        server.close()
        os.close(baseline_fd)
        try:
            SOCKET_PATH.unlink()
        except FileNotFoundError:
            pass


def accept_one(conn: socket.socket, jobs: JobQueue) -> None:
    try:
        request_bytes = recv_record(conn, MAX_REQUEST_BYTES, "request")
        candidate = recv_record(conn, MAX_CANDIDATE_BYTES, "candidate wasm")
        request = json.loads(request_bytes.decode("utf-8"))
    except Exception as error:
        send_final(conn, {"ok": False, "error": str(error)})
        conn.close()
        return
    jobs.enqueue(Job(conn=conn, request=request, candidate=candidate))


def run_worker(
    args: argparse.Namespace,
    jobs: JobQueue,
    baseline_fd: int,
    baseline_bytes: bytes,
    device: DeviceContext | None,
) -> None:
    while True:
        job = jobs.pop()
        try:
            send_json(job.conn, {"type": "started"})
            response = handle_request(args, baseline_fd, baseline_bytes, device, job.request, job.candidate)
        except Exception as error:
            response = {"ok": False, "error": str(error)}
        try:
            send_final(job.conn, response)
        finally:
            job.conn.close()
            jobs.finish(job)


def handle_request(
    args: argparse.Namespace,
    baseline_fd: int,
    baseline_bytes: bytes,
    device: DeviceContext | None,
    request: object,
    candidate: bytes,
) -> dict[str, object]:
    if not isinstance(request, dict):
        return {"ok": False, "error": "request must be a JSON object"}

    try:
        pin_name = pin_from_request(request, args.pin)
    except ValueError as error:
        return {"ok": False, "error": str(error)}
    if device is None and pin_name != "any":
        return {"ok": False, "error": "pin requires a daemon started with --serial"}

    kind = request.get("kind")
    if kind == "microbench":
        return handle_microbench_request(device, request, candidate, pin_name)
    if kind != "decode":
        return {"ok": False, "error": f"unsupported request kind: {kind!r}"}

    try:
        media_path = media_path_from_request(request)
        frame_range = frame_range_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    if device is not None:
        return handle_device_decode_request(
            args,
            baseline_bytes,
            device,
            request,
            candidate,
            frame_range,
            pin_name,
        )

    candidate_fd = memfd_from_bytes("vip9r-candidate.wasm", candidate)
    try:
        baseline = run_host_bench("baseline", baseline_fd, media_path, frame_range)
        candidate_result = run_host_bench("candidate", candidate_fd, media_path, frame_range)
    finally:
        os.close(candidate_fd)

    return {
        "ok": baseline.get("ok") is True and candidate_result.get("ok") is True,
        "baseline": str(args.baseline),
        "serial": args.serial,
        "host": {
            "baseline": baseline,
            "candidate": candidate_result,
        },
    }


def handle_microbench_request(
    device: DeviceContext | None,
    request: dict[str, object],
    candidate: bytes,
    pin_name: str,
) -> dict[str, object]:
    try:
        microbench = microbench_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    if device is not None:
        return handle_device_microbench_request(
            device,
            candidate,
            microbench,
            pin_name,
        )

    candidate_fd = memfd_from_bytes("vip9r-candidate.wasm", candidate)
    try:
        candidate_result = run_host_microbench("candidate", candidate_fd, microbench)
    finally:
        os.close(candidate_fd)

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "microbench",
        "serial": None,
        "host": {
            "candidate": candidate_result,
        },
    }


def media_path_from_request(request: dict[str, object]) -> Path:
    return MEDIA_ROOT / media_from_request(request)


def frame_range_from_request(request: dict[str, object]) -> tuple[int, int] | None:
    frames = request.get("frames")
    if frames is None:
        return None
    if not isinstance(frames, dict):
        raise ValueError("decode request requires frames")
    offset = frames.get("offset")
    last = frames.get("last")
    if not isinstance(offset, int) or isinstance(offset, bool) or offset < 0:
        raise ValueError("frames.offset must be a non-negative integer")
    if not isinstance(last, int) or isinstance(last, bool) or last < offset:
        raise ValueError("frames.last must be an integer greater than or equal to offset")
    return offset, last


def microbench_from_request(request: dict[str, object]) -> dict[str, int]:
    slot = request.get("slot")
    if not isinstance(slot, int) or isinstance(slot, bool) or slot < 0:
        raise ValueError("slot must be a non-negative integer")
    if slot > U32_MAX:
        raise ValueError(f"slot must be less than or equal to {U32_MAX}")
    return {"slot": slot}


def pin_from_request(request: dict[str, object], default: str) -> str:
    pin = request.get("pin", default)
    if not isinstance(pin, str) or pin == "":
        raise ValueError("pin must be a non-empty string")
    return pin


def handle_device_decode_request(
    args: argparse.Namespace,
    baseline_bytes: bytes,
    device: DeviceContext,
    request: dict[str, object],
    candidate: bytes,
    frame_range: tuple[int, int] | None,
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        media = device_media_from_request(request)
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, "decode")
        runner_path = push_wasm_driver_to_device(device, GOLDEN_RUNNER_PATH, run_dir)
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        baseline_path = ensure_device_baseline(device, baseline_bytes)
        push_bytes_to_device(device, candidate, candidate_path)
        baseline = run_device_bench(
            device,
            "baseline",
            runner_path,
            baseline_path,
            media_path,
            frame_range,
            pin,
        )
        candidate_result = run_device_bench(
            device,
            "candidate",
            runner_path,
            candidate_path,
            media_path,
            frame_range,
            pin,
        )
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "decode", "serial": device.serial, "error": str(error)}

    return {
        "ok": baseline.get("ok") is True and candidate_result.get("ok") is True,
        "kind": "decode",
        "baseline": str(args.baseline),
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "media": media_path,
            "baseline_wasm": baseline_path,
            "baseline": baseline,
            "candidate": candidate_result,
        },
    }


def handle_device_microbench_request(
    device: DeviceContext,
    candidate: bytes,
    microbench: dict[str, int],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        run_dir = create_device_run_dir(device, "microbench")
        runner_path = push_wasm_driver_to_device(device, MICROBENCH_RUNNER_PATH, run_dir)
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        push_bytes_to_device(device, candidate, candidate_path)
        candidate_result = run_device_microbench(
            device,
            "candidate",
            runner_path,
            candidate_path,
            microbench,
            pin,
        )
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "microbench", "serial": device.serial, "error": str(error)}

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "microbench",
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "candidate": candidate_result,
        },
    }


def device_media_from_request(request: dict[str, object]) -> PurePosixPath:
    return media_from_request(request)


def media_from_request(request: dict[str, object]) -> PurePosixPath:
    media = request.get("media")
    if not isinstance(media, str) or media == "":
        raise ValueError("decode request requires media")
    path = PurePosixPath(media)
    if path.is_absolute() or path == PurePosixPath(".") or ".." in path.parts:
        raise ValueError("media must be a corpus-relative path without '..'")
    return path


def run_device_bench(
    device: DeviceContext,
    label: str,
    runner_path: str,
    wasm_path: str,
    media_path: str,
    frame_range: tuple[int, int] | None,
    pin: PinSelection,
) -> dict[str, object]:
    d8_args = [
        "./d8",
        "--no-liftoff",
        "--module",
        runner_path,
        "--",
        wasm_path,
        "--bench",
    ]
    if frame_range is not None:
        start, last = frame_range
        d8_args.extend(["--bench-frames", f"{start}:{last}"])
    d8_args.append(media_path)
    return run_device_json(device, label, d8_args, pin, "benchmark")


def run_device_microbench(
    device: DeviceContext,
    label: str,
    runner_path: str,
    wasm_path: str,
    microbench: dict[str, int],
    pin: PinSelection,
) -> dict[str, object]:
    d8_args = [
        "./d8",
        "--no-liftoff",
        "--module",
        runner_path,
        "--",
        wasm_path,
        "--slot",
        str(microbench["slot"]),
    ]
    return run_device_json(device, label, d8_args, pin, "microbenchmark")


def run_device_json(
    device: DeviceContext,
    label: str,
    d8_args: list[str],
    pin: PinSelection,
    json_label: str,
) -> dict[str, object]:
    completed = adb_shell_completed(device.serial, device_d8_shell_command(device, d8_args, pin))
    if completed.returncode != 0:
        return {
            "ok": False,
            "label": label,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }

    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        return {
            "ok": False,
            "label": label,
            "error": f"invalid {json_label} JSON: {error}",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
    return {"ok": True, "label": label, "report": report, "stderr": completed.stderr}


def device_d8_shell_command(device: DeviceContext, d8_args: list[str], pin: PinSelection) -> str:
    command = " ".join(shlex.quote(arg) for arg in d8_args)
    if pin.mask is not None:
        command = f"taskset {shlex.quote(pin.mask)} {command}"
    return f"cd {shlex.quote(str(device.root / 'bin'))} && {command}"


def probe_device(serial: str, root: PurePosixPath) -> DeviceContext:
    state = subprocess.run(
        ["adb", "-s", serial, "get-state"],
        capture_output=True,
        text=True,
        check=False,
    )
    if state.returncode != 0 or state.stdout.strip() != "device":
        raise DeviceError(f"adb device {serial} is not ready: {state.stdout}{state.stderr}".strip())

    probe = adb_shell_completed(serial, device_probe_script())
    if probe.returncode != 0:
        raise DeviceError(f"device probe failed: {probe.stderr or probe.stdout}".strip())

    props: dict[str, str] = {}
    cpus: list[CpuInfo] = []
    online: str | None = None
    taskset: str | None = None
    for line in probe.stdout.splitlines():
        fields = line.split("\t")
        if not fields:
            continue
        if fields[0] == "prop" and len(fields) >= 3:
            props[fields[1]] = fields[2]
        elif fields[0] == "online" and len(fields) >= 2:
            online = fields[1]
        elif fields[0] == "taskset" and len(fields) >= 2:
            taskset = fields[1] or None
        elif fields[0] == "cpu" and len(fields) >= 5:
            cpus.append(
                CpuInfo(
                    index=int(fields[1]),
                    capacity=parse_optional_int(fields[2]),
                    max_freq=parse_optional_int(fields[3]),
                    midr=fields[4] or None,
                )
            )

    if not cpus:
        raise DeviceError("device probe found no CPUs")
    online_cpus = parse_cpu_list(online) if online is not None else [cpu.index for cpu in cpus]
    device = DeviceContext(
        serial=serial,
        root=root,
        props=props,
        cpus=sorted(cpus, key=lambda cpu: cpu.index),
        online_cpus=online_cpus,
        taskset=taskset,
        d8_version="",
    )
    verify_device_runtime(device)
    device.d8_version = probe_device_d8_version(device)
    return device


def device_probe_script() -> str:
    return """
printf 'prop\\tmodel\\t%s\\n' "$(getprop ro.product.model)"
printf 'prop\\tdevice\\t%s\\n' "$(getprop ro.product.device)"
printf 'prop\\tproduct\\t%s\\n' "$(getprop ro.build.product)"
printf 'prop\\tabi\\t%s\\n' "$(getprop ro.product.cpu.abi)"
printf 'prop\\tabilist\\t%s\\n' "$(getprop ro.product.cpu.abilist)"
printf 'prop\\tsdk\\t%s\\n' "$(getprop ro.build.version.sdk)"
printf 'prop\\trelease\\t%s\\n' "$(getprop ro.build.version.release)"
printf 'prop\\tbuild_type\\t%s\\n' "$(getprop ro.build.type)"
printf 'online\\t%s\\n' "$(cat /sys/devices/system/cpu/online 2>/dev/null)"
printf 'taskset\\t%s\\n' "$(command -v taskset || true)"
for c in /sys/devices/system/cpu/cpu[0-9]*; do
  cpu=${c##*cpu}
  cap=$(cat "$c/cpu_capacity" 2>/dev/null || true)
  max=$(cat "$c/cpufreq/cpuinfo_max_freq" 2>/dev/null || true)
  midr=$(cat "$c/regs/identification/midr_el1" 2>/dev/null || true)
  printf 'cpu\\t%s\\t%s\\t%s\\t%s\\n' "$cpu" "$cap" "$max" "$midr"
done
"""


def verify_device_runtime(device: DeviceContext) -> None:
    bin_dir = str(device.root / "bin")
    required = [
        (str(PurePosixPath(bin_dir) / "d8"), "-x"),
        (str(PurePosixPath(bin_dir) / "icudtl.dat"), "-f"),
        (str(PurePosixPath(bin_dir) / "snapshot_blob.bin"), "-f"),
    ]
    missing = missing_remote_files(device, required)
    if missing:
        raise DeviceError("missing device d8 payload: " + ", ".join(missing))


def probe_device_d8_version(device: DeviceContext) -> str:
    script = f"cd {shlex.quote(str(device.root / 'bin'))} && ./d8 -e 'print(version()); quit(0)'"
    completed = adb_shell_completed(device.serial, script)
    if completed.returncode != 0:
        raise DeviceError(f"device d8 failed to launch: {completed.stderr or completed.stdout}".strip())
    lines = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
    if not lines:
        raise DeviceError("device d8 version probe produced no output")
    return lines[-1]


def resolve_and_verify_pin(device: DeviceContext, pin_name: str) -> PinSelection:
    pin = resolve_pin(device, pin_name)
    if pin.mask is not None:
        completed = adb_shell_completed(
            device.serial,
            f"taskset {shlex.quote(pin.mask)} sh -c 'grep Cpus_allowed_list /proc/self/status'",
        )
        if completed.returncode != 0:
            raise DeviceError(f"taskset {pin.mask} failed: {completed.stderr or completed.stdout}".strip())
        verified_cpus = parse_cpus_allowed_list(completed.stdout)
        if verified_cpus is None:
            raise DeviceError(f"taskset {pin.mask} did not report Cpus_allowed_list")
        if pin.cpus is None or verified_cpus != pin.cpus:
            raise DeviceError(
                f"taskset {pin.mask} selected CPUs {format_cpu_list(verified_cpus)}, "
                f"expected {format_cpu_list(pin.cpus or [])}"
            )
        pin.verified_cpus = verified_cpus
    return pin


def resolve_pin(device: DeviceContext, pin_name: str) -> PinSelection:
    if pin_name == "any":
        return PinSelection(request=pin_name, mask=None, cpus=None)
    if device.taskset is None:
        raise ValueError("device has no taskset command")
    if pin_name == "all":
        return pin_for_cpus(device, pin_name, device.online_cpus)
    if pin_name.startswith("cpu:"):
        cpu = parse_pin_cpu(pin_name)
        return pin_for_cpus(device, pin_name, [cpu])
    if pin_name.startswith("mask:"):
        mask = parse_pin_mask(pin_name)
        cpus = cpus_from_mask(mask)
        if not cpus:
            raise ValueError("pin mask selects no CPUs")
        unknown = sorted(set(cpus) - set(device.online_cpus))
        if unknown:
            raise ValueError(f"pin mask selects offline or missing CPUs: {format_cpu_list(unknown)}")
        return PinSelection(request=pin_name, mask=f"{mask:x}", cpus=cpus)
    raise ValueError(f"pin must be {PIN_HELP}")


def pin_for_cpus(device: DeviceContext, request: str, cpus: list[int]) -> PinSelection:
    selected = sorted(set(cpus))
    if not selected:
        raise ValueError(f"pin {request!r} selects no CPUs")
    unknown = sorted(set(selected) - set(device.online_cpus))
    if unknown:
        raise ValueError(f"pin {request!r} selects offline or missing CPUs: {format_cpu_list(unknown)}")
    return PinSelection(request=request, mask=f"{mask_from_cpus(selected):x}", cpus=selected)


def parse_pin_cpu(pin_name: str) -> int:
    value = pin_name.removeprefix("cpu:")
    if not re.fullmatch(r"0|[1-9]\d*", value):
        raise ValueError("cpu pin must be cpu:N")
    return int(value)


def parse_pin_mask(pin_name: str) -> int:
    value = pin_name.removeprefix("mask:")
    if not re.fullmatch(r"[0-9a-fA-F]+", value):
        raise ValueError("mask pin must be mask:HEX")
    return int(value, 16)


def create_device_run_dir(device: DeviceContext, kind: str) -> str:
    run_id = f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-{os.getpid()}-{time.time_ns() % 1_000_000_000:09d}-{kind}"
    run_dir = str(device.root / "runs" / run_id)
    ensure_remote_dir(device, run_dir)
    return run_dir


def ensure_device_baseline(device: DeviceContext, baseline: bytes) -> str:
    digest = hashlib.sha256(baseline).hexdigest()[:24]
    cache_dir = str(device.root / "runs" / "_cache")
    remote_path = str(PurePosixPath(cache_dir) / f"baseline-{digest}.wasm")
    ensure_remote_dir(device, cache_dir)
    if not remote_file_has_size(device, remote_path, len(baseline)):
        push_bytes_to_device(device, baseline, remote_path)
    return remote_path


def verify_remote_files(device: DeviceContext, paths: list[str]) -> None:
    missing = missing_remote_files(device, [(path, "-f") for path in paths])
    if missing:
        raise DeviceError("missing device media: " + ", ".join(missing))


def missing_remote_files(device: DeviceContext, required: list[tuple[str, str]]) -> list[str]:
    missing: list[str] = []
    for path, test in required:
        completed = adb_shell_completed(device.serial, f"[ {test} {shlex.quote(path)} ]")
        if completed.returncode != 0:
            missing.append(path)
    return missing


def ensure_remote_dir(device: DeviceContext, path: str) -> None:
    completed = adb_shell_completed(device.serial, f"mkdir -p {shlex.quote(path)}")
    if completed.returncode != 0:
        raise DeviceError(f"create device directory {path}: {completed.stderr or completed.stdout}".strip())


def remote_file_has_size(device: DeviceContext, path: str, size: int) -> bool:
    completed = adb_shell_completed(
        device.serial,
        f"[ -f {shlex.quote(path)} ] && wc -c < {shlex.quote(path)}",
    )
    if completed.returncode != 0:
        return False
    try:
        return int(completed.stdout.strip()) == size
    except ValueError:
        return False


def push_wasm_driver_to_device(device: DeviceContext, main_runner: Path, run_dir: str) -> str:
    for local_path in sorted(main_runner.parent.glob("*.js")):
        push_file_to_device(device, local_path, str(PurePosixPath(run_dir) / local_path.name))
    return str(PurePosixPath(run_dir) / main_runner.name)


def push_file_to_device(device: DeviceContext, local_path: Path, remote_path: str) -> None:
    completed = subprocess.run(
        ["adb", "-s", device.serial, "push", str(local_path), remote_path],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise DeviceError(f"adb push {local_path} {remote_path}: {completed.stderr or completed.stdout}".strip())


def push_bytes_to_device(device: DeviceContext, data: bytes, remote_path: str) -> None:
    with tempfile.NamedTemporaryFile(prefix="vip9r-perf-", suffix=".wasm") as temp:
        temp.write(data)
        temp.flush()
        push_file_to_device(device, Path(temp.name), remote_path)


def adb_shell_completed(serial: str, script: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["adb", "-s", serial, "shell", script],
        capture_output=True,
        text=True,
        check=False,
    )


def parse_optional_int(value: str) -> int | None:
    if value == "":
        return None
    try:
        return int(value)
    except ValueError:
        return None


def parse_cpu_list(value: str | None) -> list[int]:
    if value is None or value == "":
        return []
    cpus: list[int] = []
    for part in value.split(","):
        if "-" in part:
            start, end = part.split("-", 1)
            cpus.extend(range(int(start), int(end) + 1))
        else:
            cpus.append(int(part))
    return sorted(set(cpus))


def format_cpu_list(cpus: list[int]) -> str:
    return ",".join(str(cpu) for cpu in cpus)


def mask_from_cpus(cpus: list[int]) -> int:
    mask = 0
    for cpu in cpus:
        mask |= 1 << cpu
    return mask


def cpus_from_mask(mask: int) -> list[int]:
    cpus: list[int] = []
    cpu = 0
    while mask:
        if mask & 1:
            cpus.append(cpu)
        mask >>= 1
        cpu += 1
    return cpus


def parse_cpus_allowed_list(status: str) -> list[int] | None:
    for line in status.splitlines():
        if line.startswith("Cpus_allowed_list:"):
            return parse_cpu_list(line.split(":", 1)[1].strip())
    return None


def device_summary(device: DeviceContext | None) -> dict[str, object] | None:
    if device is None:
        return None
    return {
        "serial": device.serial,
        "root": str(device.root),
        "model": device.props.get("model"),
        "device": device.props.get("device"),
        "product": device.props.get("product"),
        "abi": device.props.get("abi"),
        "abilist": device.props.get("abilist"),
        "sdk": device.props.get("sdk"),
        "release": device.props.get("release"),
        "build_type": device.props.get("build_type"),
        "d8_version": device.d8_version,
        "taskset": device.taskset,
        "online_cpus": device.online_cpus,
        "cpus": [
            {
                "index": cpu.index,
                "capacity": cpu.capacity,
                "max_freq": cpu.max_freq,
                "midr": cpu.midr,
            }
            for cpu in device.cpus
        ],
    }


def pin_summary(pin: PinSelection) -> dict[str, object]:
    return {
        "request": pin.request,
        "mask": pin.mask,
        "cpus": pin.cpus,
        "verified_cpus": pin.verified_cpus,
    }


def run_host_bench(
    label: str,
    wasm_fd: int,
    media_path: Path,
    frame_range: tuple[int, int] | None,
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        os.environ.get("D8_LINUX64", "d8"),
        "--no-liftoff",
        "--module",
        str(GOLDEN_RUNNER_PATH),
        "--",
        proc_fd_path(wasm_fd),
        "--bench",
    ]
    if frame_range is not None:
        start, last = frame_range
        cmd.extend(["--bench-frames", f"{start}:{last}"])
    cmd.append(str(media_path))
    completed = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        return {
            "ok": False,
            "label": label,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }

    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        return {
            "ok": False,
            "label": label,
            "error": f"invalid benchmark JSON: {error}",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
    return {"ok": True, "label": label, "report": report, "stderr": completed.stderr}


def run_host_microbench(
    label: str,
    wasm_fd: int,
    microbench: dict[str, int],
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        os.environ.get("D8_LINUX64", "d8"),
        "--no-liftoff",
        "--module",
        str(MICROBENCH_RUNNER_PATH),
        "--",
        proc_fd_path(wasm_fd),
        "--slot",
        str(microbench["slot"]),
    ]

    completed = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if completed.returncode != 0:
        return {
            "ok": False,
            "label": label,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }

    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        return {
            "ok": False,
            "label": label,
            "error": f"invalid microbenchmark JSON: {error}",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
    return {"ok": True, "label": label, "report": report, "stderr": completed.stderr}


def proc_fd_path(fd: int) -> str:
    return f"/proc/{os.getpid()}/fd/{fd}"


def memfd_from_bytes(name: str, data: bytes) -> int:
    fd = os.memfd_create(name, os.MFD_CLOEXEC)
    try:
        write_all(fd, data)
        os.lseek(fd, 0, os.SEEK_SET)
    except Exception:
        os.close(fd)
        raise
    return fd


def write_all(fd: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        written = os.write(fd, view)
        view = view[written:]


def recv_record(sock: socket.socket, max_bytes: int, label: str) -> bytes:
    data, _ancillary, flags, _address = sock.recvmsg(max_bytes)
    if flags & getattr(socket, "MSG_TRUNC", 0):
        raise ValueError(f"{label} message exceeds {max_bytes} bytes")
    if data == b"":
        raise ValueError(f"missing {label} message")
    return data


def send_json(sock: socket.socket, value: object) -> None:
    send_record(sock, json.dumps(value, separators=(",", ":")).encode("utf-8"))


def send_final(sock: socket.socket, value: dict[str, object]) -> None:
    send_json(sock, {"type": "result", **value})


def send_record(sock: socket.socket, data: bytes) -> None:
    sent = sock.sendmsg([data])
    if sent != len(data):
        raise OSError(f"short seqpacket send: {sent}/{len(data)} bytes")


if __name__ == "__main__":
    raise SystemExit(main())
