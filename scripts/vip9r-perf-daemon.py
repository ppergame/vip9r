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

REPO_ROOT = Path(__file__).resolve().parents[1]
SOCKET_PATH = REPO_ROOT / "temp/vip9r-perf.sock"
GOLDEN_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/golden.js"
MICROBENCH_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/microbench.js"
TESTS_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/tests.js"
MEDIA_ROOT = Path("/bulk/vip9r")
DEFAULT_DEVICE_ROOT = PurePosixPath("/data/local/tmp/vip9r")
MAX_REQUEST_BYTES = 64 * 1024
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
ACCEPT_HANDSHAKE_TIMEOUT_SECONDS = 10
RESPONSE_STRING_HEAD_BYTES = 8 * 1024
RESPONSE_STRING_TAIL_BYTES = 8 * 1024
RESPONSE_LIST_HEAD_ITEMS = 64
RESPONSE_LIST_TAIL_ITEMS = 16
U32_MAX = 2**32 - 1
PIN_HELP = "any, all, cpu:N, or mask:HEX"
HOST_D8_TIMEOUT_SECONDS = 180
DEVICE_D8_TIMEOUT_SECONDS = 180
ADB_D8_TIMEOUT_SECONDS = DEVICE_D8_TIMEOUT_SECONDS + 30
DEVICE_TIMEOUT_EXIT_CODE = 124
TARGETS = ("host", "device")
ARM_IMPLEMENTER = 0x41
ARM_PART_NAMES = {
    0xD05: "cortex-a55",
    0xD80: "cortex-a520",
    0xD81: "cortex-a720",
    0xD82: "cortex-x4",
}


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
    def __init__(self, target: str) -> None:
        self.target = target
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
                send_json(
                    job.conn,
                    {
                        "type": "queued",
                        "target": self.target,
                        "ahead": running_count + index,
                    },
                )
                live.append(job)
            except OSError:
                job.conn.close()
        self._pending = live


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="vip9r-perf-daemon",
        description="Queue vip9r performance runs.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("probe", help="probe connected Android devices")
    prepare = subparsers.add_parser("prepare", help="sync and probe device runtime state")
    prepare.add_argument(
        "--serial",
        required=True,
        help="adb device serial",
    )
    serve = subparsers.add_parser("serve", help="start the performance queue")
    serve.add_argument(
        "--baseline",
        required=True,
        type=Path,
        help="baseline wasm module built by the orchestrator",
    )
    serve.add_argument(
        "--serial",
        help="adb device serial; omit to run host-only",
    )
    serve.add_argument(
        "--pin",
        help=f"default device CPU pin: {PIN_HELP}",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.command == "probe":
        return run_probe()
    if args.command == "prepare":
        return run_prepare(args)
    if args.command == "serve":
        return run_serve(args)
    raise AssertionError(f"unknown command: {args.command!r}")


def run_probe() -> int:
    try:
        devices = probe_connected_devices()
    except Exception as error:
        print(json.dumps({"ok": False, "error": str(error)}, indent=2), flush=True)
        return 2
    print(json.dumps({"ok": True, "devices": devices}, indent=2), flush=True)
    return 0


def run_prepare(args: argparse.Namespace) -> int:
    try:
        device = probe_device(args.serial, DEFAULT_DEVICE_ROOT, require_runtime=False)
        v8_root = select_v8_root(device)
        prepare_device(device, v8_root)
        device.d8_version = probe_device_d8_version(device)
    except Exception as error:
        print(f"device prepare: {error}", file=sys.stderr)
        return 2

    print(
        json.dumps(
            {
                "ok": True,
                "serial": args.serial,
                "device": device_probe_summary(device),
            },
            indent=2,
        ),
        flush=True,
    )
    return 0


def run_serve(args: argparse.Namespace) -> int:
    if args.serial is None and args.pin is not None:
        print("--pin requires --serial", file=sys.stderr)
        return 2
    if args.serial is not None and args.pin is None:
        print("--pin is required with --serial", file=sys.stderr)
        return 2

    try:
        verify_local_wasm_runners()
        host_d8 = verify_host_runtime()
        baseline_bytes = args.baseline.read_bytes()
        baseline_fd = memfd_from_bytes("vip9r-baseline.wasm", baseline_bytes)
    except Exception as error:
        print(f"host setup: {error}", file=sys.stderr)
        return 2
    device: DeviceContext | None = None
    args.pin = args.pin or "any"
    default_pin_selection: PinSelection | None = None
    if args.serial is not None:
        try:
            device = probe_device(args.serial, DEFAULT_DEVICE_ROOT)
            default_pin_selection = resolve_and_verify_pin(device, args.pin)
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
        server.close()
        os.close(baseline_fd)
        if error.errno == errno.EADDRINUSE:
            print(f"socket already exists: {SOCKET_PATH}", file=sys.stderr)
            return 2
        raise

    host_jobs = JobQueue("host")
    device_jobs = JobQueue("device")
    host_worker = threading.Thread(
        target=run_worker,
        args=("host", args, host_jobs, baseline_fd, baseline_bytes, device),
        daemon=True,
    )
    device_worker = threading.Thread(
        target=run_worker,
        args=("device", args, device_jobs, baseline_fd, baseline_bytes, device),
        daemon=True,
    )
    host_worker.start()
    device_worker.start()

    try:
        server.listen()
        print(
            json.dumps(
                {
                    "baseline": str(args.baseline),
                    "ok": True,
                    "socket": str(SOCKET_PATH),
                    "host": {
                        "available": True,
                        "d8": host_d8,
                        "timeout_seconds": HOST_D8_TIMEOUT_SECONDS,
                    },
                    "device": {
                        "serial": device.serial,
                        "model": device.props.get("model"),
                        "pin": pin_summary(default_pin_selection),
                        "timeout_seconds": DEVICE_D8_TIMEOUT_SECONDS,
                        "adb_timeout_seconds": ADB_D8_TIMEOUT_SECONDS,
                    }
                    if device is not None and default_pin_selection is not None
                    else None,
                },
                indent=2,
            ),
            flush=True,
        )
        while True:
            conn, _ = server.accept()
            conn.settimeout(ACCEPT_HANDSHAKE_TIMEOUT_SECONDS)
            conn.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_CANDIDATE_BYTES)
            conn.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_RESPONSE_BYTES)
            accept_one(conn, host_jobs, device_jobs)
    except KeyboardInterrupt:
        return 130
    finally:
        server.close()
        os.close(baseline_fd)
        try:
            SOCKET_PATH.unlink()
        except FileNotFoundError:
            pass


def accept_one(conn: socket.socket, host_jobs: JobQueue, device_jobs: JobQueue) -> None:
    try:
        request_bytes = recv_record(conn, MAX_REQUEST_BYTES, "request")
        candidate = recv_record(conn, MAX_CANDIDATE_BYTES, "candidate wasm")
        conn.settimeout(None)
        request = json.loads(request_bytes.decode("utf-8"))
        if not isinstance(request, dict):
            raise ValueError("request must be a JSON object")
        target = target_from_request(request)
    except socket.timeout:
        safe_send_final(
            conn,
            {
                "ok": False,
                "error": f"request handshake timed out after {ACCEPT_HANDSHAKE_TIMEOUT_SECONDS}s",
            },
        )
        conn.close()
        return
    except Exception as error:
        safe_send_final(conn, {"ok": False, "error": str(error)})
        conn.close()
        return
    if target == "host":
        host_jobs.enqueue(Job(conn=conn, request=request, candidate=candidate))
    elif target == "device":
        device_jobs.enqueue(Job(conn=conn, request=request, candidate=candidate))
    else:
        safe_send_final(conn, {"ok": False, "error": f"unsupported target: {target!r}"})
        conn.close()


def run_worker(
    target: str,
    args: argparse.Namespace,
    jobs: JobQueue,
    baseline_fd: int,
    baseline_bytes: bytes,
    device: DeviceContext | None,
) -> None:
    while True:
        job = jobs.pop()
        try:
            send_json(job.conn, {"type": "started", "target": target})
            response = handle_request(
                target,
                args,
                baseline_fd,
                baseline_bytes,
                device,
                job.request,
                job.candidate,
            )
        except Exception as error:
            response = {"ok": False, "error": str(error)}
        try:
            safe_send_final(job.conn, response)
        finally:
            job.conn.close()
            jobs.finish(job)


def handle_request(
    target: str,
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
        request_target = target_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}
    if request_target != target:
        return {
            "ok": False,
            "error": f"request target {request_target!r} was routed to {target!r}",
        }
    if target == "host" and "pin" in request:
        return {"ok": False, "target": "host", "error": "pin requires target device"}

    kind = request.get("kind")
    if kind == "validate":
        try:
            media = media_from_request(request)
            validation = validation_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "validate", "target": target, "error": str(error)}
        if target == "host":
            return handle_host_validate_request(candidate, MEDIA_ROOT / media, validation)
        if device is None:
            return {
                "ok": False,
                "kind": "validate",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, args.pin)
        except ValueError as error:
            return {"ok": False, "kind": "validate", "target": "device", "error": str(error)}
        return handle_device_validate_request(
            device,
            candidate,
            media,
            validation,
            pin_name,
        )
    if kind == "tests":
        if target == "host":
            return handle_host_tests_request(request, candidate)
        if device is None:
            return {
                "ok": False,
                "kind": "tests",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, args.pin)
            tests = tests_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "tests", "target": "device", "error": str(error)}
        return handle_device_tests_request(
            device,
            candidate,
            tests,
            pin_name,
        )
    if kind == "microbench":
        if target == "host":
            return handle_host_microbench_request(request, candidate)
        if device is None:
            return {
                "ok": False,
                "kind": "microbench",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, args.pin)
            microbench = microbench_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "microbench", "target": "device", "error": str(error)}
        return handle_device_microbench_request(
            device,
            candidate,
            microbench,
            pin_name,
        )
    if kind != "bench":
        return {"ok": False, "error": f"unsupported request kind: {kind!r}"}

    try:
        media = media_from_request(request)
        frame_range = frame_range_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    if target == "host":
        return handle_host_bench_request(args, baseline_fd, candidate, MEDIA_ROOT / media, frame_range)

    if device is None:
        return {
            "ok": False,
            "kind": "bench",
            "target": "device",
            "error": "target device requires a daemon started with --serial",
        }
    try:
        pin_name = pin_from_request(request, args.pin)
    except ValueError as error:
        return {"ok": False, "kind": "bench", "target": "device", "error": str(error)}
    return handle_device_bench_request(
        args,
        baseline_bytes,
        device,
        candidate,
        media,
        frame_range,
        pin_name,
    )


def handle_host_bench_request(
    args: argparse.Namespace,
    baseline_fd: int,
    candidate: bytes,
    media_path: Path,
    frame_range: tuple[int, int] | None,
) -> dict[str, object]:
    candidate_fd = memfd_from_bytes("vip9r-candidate.wasm", candidate)
    try:
        baseline = run_host_bench("baseline", baseline_fd, media_path, frame_range)
        candidate_result = run_host_bench("candidate", candidate_fd, media_path, frame_range)
    finally:
        os.close(candidate_fd)
    return {
        "ok": baseline.get("ok") is True and candidate_result.get("ok") is True,
        "kind": "bench",
        "target": "host",
        "baseline": str(args.baseline),
        "serial": None,
        "host": {
            "media": str(media_path),
            "baseline": baseline,
            "candidate": candidate_result,
        },
    }


def handle_host_validate_request(
    candidate: bytes,
    media_path: Path,
    validation: dict[str, object],
) -> dict[str, object]:
    candidate_fd = memfd_from_bytes("vip9r-candidate.wasm", candidate)
    try:
        candidate_result = run_host_validation("candidate", candidate_fd, media_path, validation)
    finally:
        os.close(candidate_fd)
    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "validate",
        "target": "host",
        "serial": None,
        "host": {
            "media": str(media_path),
            "allow_mismatch": validation.get("allow_mismatch") is True,
            "candidate": candidate_result,
        },
    }


def handle_host_microbench_request(
    request: dict[str, object],
    candidate: bytes,
) -> dict[str, object]:
    try:
        microbench = microbench_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    candidate_fd = memfd_from_bytes("vip9r-candidate.wasm", candidate)
    try:
        candidate_result = run_host_microbench("candidate", candidate_fd, microbench)
    finally:
        os.close(candidate_fd)

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "microbench",
        "target": "host",
        "serial": None,
        "host": {
            "candidate": candidate_result,
        },
    }


def handle_host_tests_request(
    request: dict[str, object],
    candidate: bytes,
) -> dict[str, object]:
    try:
        tests = tests_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    candidate_fd = memfd_from_bytes("vip9r-candidate-tests.wasm", candidate)
    try:
        candidate_result = run_host_tests("candidate", candidate_fd, tests)
    finally:
        os.close(candidate_fd)

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "tests",
        "target": "host",
        "serial": None,
        "host": {
            "candidate": candidate_result,
        },
    }


def frame_range_from_request(request: dict[str, object]) -> tuple[int, int] | None:
    frames = request.get("frames")
    if frames is None:
        return None
    if not isinstance(frames, dict):
        raise ValueError("bench request requires frames")
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


def validation_from_request(request: dict[str, object]) -> dict[str, object]:
    validation: dict[str, object] = {}
    allow_mismatch = request.get("allow_mismatch", False)
    if not isinstance(allow_mismatch, bool):
        raise ValueError("allow_mismatch must be boolean")
    if allow_mismatch:
        validation["allow_mismatch"] = True

    return validation


def tests_from_request(request: dict[str, object]) -> dict[str, str]:
    tests: dict[str, str] = {}
    test_filter = request.get("filter")
    if test_filter is None:
        return tests
    if not isinstance(test_filter, str) or test_filter == "":
        raise ValueError("filter must be a non-empty string")
    tests["filter"] = test_filter
    return tests


def target_from_request(request: dict[str, object]) -> str:
    target = request.get("target", "host")
    if not isinstance(target, str) or target not in TARGETS:
        raise ValueError("target must be host or device")
    return target


def pin_from_request(request: dict[str, object], default: str) -> str:
    pin = request.get("pin", default)
    if not isinstance(pin, str) or pin == "":
        raise ValueError("pin must be a non-empty string")
    return pin


def handle_device_bench_request(
    args: argparse.Namespace,
    baseline_bytes: bytes,
    device: DeviceContext,
    candidate: bytes,
    media: PurePosixPath,
    frame_range: tuple[int, int] | None,
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, "bench")
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
        return {"ok": False, "kind": "bench", "target": "device", "serial": device.serial, "error": str(error)}

    return {
        "ok": baseline.get("ok") is True and candidate_result.get("ok") is True,
        "kind": "bench",
        "target": "device",
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
        return {"ok": False, "kind": "microbench", "target": "device", "serial": device.serial, "error": str(error)}

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "microbench",
        "target": "device",
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "candidate": candidate_result,
        },
    }


def handle_device_tests_request(
    device: DeviceContext,
    candidate: bytes,
    tests: dict[str, str],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        run_dir = create_device_run_dir(device, "tests")
        runner_path = push_wasm_driver_to_device(device, TESTS_RUNNER_PATH, run_dir)
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        push_bytes_to_device(device, candidate, candidate_path)
        candidate_result = run_device_tests(
            device,
            "candidate",
            runner_path,
            candidate_path,
            tests,
            pin,
        )
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "tests", "target": "device", "serial": device.serial, "error": str(error)}

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "tests",
        "target": "device",
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "candidate": candidate_result,
        },
    }


def handle_device_validate_request(
    device: DeviceContext,
    candidate: bytes,
    media: PurePosixPath,
    validation: dict[str, object],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, "validate")
        runner_path = push_wasm_driver_to_device(device, GOLDEN_RUNNER_PATH, run_dir)
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        push_bytes_to_device(device, candidate, candidate_path)
        candidate_result = run_device_validation(
            device,
            "candidate",
            runner_path,
            candidate_path,
            media_path,
            validation,
            pin,
        )
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "validate", "target": "device", "serial": device.serial, "error": str(error)}

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "validate",
        "target": "device",
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "media": media_path,
            "allow_mismatch": validation.get("allow_mismatch") is True,
            "candidate": candidate_result,
        },
    }


def media_from_request(request: dict[str, object]) -> PurePosixPath:
    media = request.get("media")
    if not isinstance(media, str) or media == "":
        raise ValueError("bench request requires media")
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


def run_device_validation(
    device: DeviceContext,
    label: str,
    runner_path: str,
    wasm_path: str,
    media_path: str,
    validation: dict[str, object],
    pin: PinSelection,
) -> dict[str, object]:
    d8_args = [
        "./d8",
        "--no-liftoff",
        "--module",
        runner_path,
        "--",
        wasm_path,
    ]
    if validation.get("allow_mismatch") is True:
        d8_args.append("--allow-mismatch")
    d8_args.append(media_path)
    return run_device_text(device, label, d8_args, pin)


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


def run_device_tests(
    device: DeviceContext,
    label: str,
    runner_path: str,
    wasm_path: str,
    tests: dict[str, str],
    pin: PinSelection,
) -> dict[str, object]:
    d8_args = [
        "./d8",
        "--module",
        runner_path,
        "--",
        wasm_path,
        "--json",
    ]
    test_filter = tests.get("filter")
    if test_filter is not None:
        d8_args.append(test_filter)
    try:
        completed = adb_shell_completed(
            device.serial,
            device_d8_shell_command(device, d8_args, pin),
            timeout=ADB_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "adb shell", ADB_D8_TIMEOUT_SECONDS, error)
    if completed.returncode == DEVICE_TIMEOUT_EXIT_CODE:
        return {
            "ok": False,
            "label": label,
            "returncode": completed.returncode,
            "timeout": True,
            "timeout_kind": "device d8",
            "timeout_seconds": DEVICE_D8_TIMEOUT_SECONDS,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
    return json_stdout_result(completed, label, "test")


def run_device_text(
    device: DeviceContext,
    label: str,
    d8_args: list[str],
    pin: PinSelection,
) -> dict[str, object]:
    try:
        completed = adb_shell_completed(
            device.serial,
            device_d8_shell_command(device, d8_args, pin),
            timeout=ADB_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "adb shell", ADB_D8_TIMEOUT_SECONDS, error)
    result = completed_text_result(completed, label)
    if completed.returncode == DEVICE_TIMEOUT_EXIT_CODE:
        result["timeout"] = True
        result["timeout_kind"] = "device d8"
        result["timeout_seconds"] = DEVICE_D8_TIMEOUT_SECONDS
    return result


def run_device_json(
    device: DeviceContext,
    label: str,
    d8_args: list[str],
    pin: PinSelection,
    json_label: str,
) -> dict[str, object]:
    try:
        completed = adb_shell_completed(
            device.serial,
            device_d8_shell_command(device, d8_args, pin),
            timeout=ADB_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "adb shell", ADB_D8_TIMEOUT_SECONDS, error)
    if completed.returncode != 0:
        result = {
            "ok": False,
            "label": label,
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
        if completed.returncode == DEVICE_TIMEOUT_EXIT_CODE:
            result["timeout"] = True
            result["timeout_kind"] = "device d8"
            result["timeout_seconds"] = DEVICE_D8_TIMEOUT_SECONDS
        return result

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
    timed_command = f"timeout {DEVICE_D8_TIMEOUT_SECONDS} sh -c {shlex.quote('exec ' + command)}"
    return (
        f"cd {shlex.quote(str(device.root / 'bin'))} && {timed_command}; "
        "status=$?; "
        f"case $status in {DEVICE_TIMEOUT_EXIT_CODE}|137|143) "
        f"echo {shlex.quote(f'vip9r device d8 timeout after {DEVICE_D8_TIMEOUT_SECONDS}s')} >&2; "
        f"exit {DEVICE_TIMEOUT_EXIT_CODE};; "
        "*) exit $status;; "
        "esac"
    )


def probe_device(serial: str, root: PurePosixPath, *, require_runtime: bool = True) -> DeviceContext:
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
    if require_runtime:
        verify_device_runtime(device)
        device.d8_version = probe_device_d8_version(device)
    return device


def probe_connected_devices() -> list[dict[str, object]]:
    devices: list[dict[str, object]] = []
    for serial, state in adb_devices():
        if state != "device":
            devices.append(
                {
                    "serial": serial,
                    "state": state,
                    "error": f"adb state is {state!r}",
                }
            )
            continue
        try:
            device = probe_device(serial, DEFAULT_DEVICE_ROOT, require_runtime=False)
            devices.append(device_probe_summary(device, state=state))
        except Exception as error:
            devices.append(
                {
                    "serial": serial,
                    "state": state,
                    "error": str(error),
                }
            )
    return devices


def adb_devices() -> list[tuple[str, str]]:
    try:
        completed = subprocess.run(
            ["adb", "devices"],
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError as error:
        raise DeviceError("adb not found") from error
    if completed.returncode != 0:
        raise DeviceError(f"adb devices: {completed.stderr or completed.stdout}".strip())

    devices: list[tuple[str, str]] = []
    for line in completed.stdout.splitlines()[1:]:
        line = line.strip()
        if line == "" or line.startswith("*"):
            continue
        fields = line.split()
        if len(fields) < 2:
            continue
        devices.append((fields[0], fields[1]))
    return devices


def device_probe_summary(device: DeviceContext, *, state: str = "device") -> dict[str, object]:
    prepared, runtime_error = device_prepared_status(device)
    if prepared:
        try:
            device.d8_version = probe_device_d8_version(device)
        except DeviceError as error:
            prepared = False
            runtime_error = str(error)

    summary = device_summary(device)
    assert summary is not None
    summary["state"] = state
    summary["prepared"] = prepared
    if runtime_error is not None:
        summary["runtime_error"] = runtime_error
    summary["cpu_groups"] = cpu_groups(device)
    return summary


def device_prepared_status(device: DeviceContext) -> tuple[bool, str | None]:
    try:
        verify_device_runtime(device)
    except DeviceError as error:
        return False, str(error)
    return True, None


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
    missing_commands = missing_remote_commands(device, ["timeout"])
    if missing_commands:
        raise DeviceError("missing device commands: " + ", ".join(missing_commands))


def prepare_device(device: DeviceContext, v8_root: Path) -> None:
    if not MEDIA_ROOT.is_dir():
        raise DeviceError(f"media root is not a directory: {MEDIA_ROOT}")
    verify_local_v8_root(v8_root)

    ensure_remote_dir(device, str(device.root))
    ensure_remote_dir(device, str(device.root / "bin"))
    ensure_remote_dir(device, str(device.root / "media"))
    ensure_remote_dir(device, str(device.root / "runs"))
    sync_v8_to_device(device, v8_root)
    sync_media_to_device(device)
    verify_device_runtime(device)


def select_v8_root(device: DeviceContext) -> Path:
    abi_values = ",".join(
        value
        for value in [
            device.props.get("abi", ""),
            device.props.get("abilist", ""),
        ]
        if value
    )
    if "arm64-v8a" in abi_values:
        env_name = "V8_ANDROID_ARM64"
    elif "armeabi-v7a" in abi_values or "armeabi" in abi_values:
        env_name = "V8_ANDROID_ARM32"
    else:
        raise DeviceError(f"unsupported Android ABI list: {abi_values or '<empty>'}")

    value = os.environ.get(env_name)
    if value is None:
        raise DeviceError(f"{env_name} is not set")
    return Path(value)


def verify_local_v8_root(v8_root: Path) -> None:
    missing = [
        str(path)
        for path in local_v8_payload_paths(v8_root)
        if not path.exists()
    ]
    if missing:
        raise DeviceError("missing local V8 payload: " + ", ".join(missing))


def verify_local_wasm_runners() -> None:
    missing = [str(path) for path in local_wasm_runner_paths() if not path.is_file()]
    if missing:
        raise RuntimeError("missing prebuilt wasm driver: " + ", ".join(missing))


def local_wasm_runner_paths() -> list[Path]:
    return [
        GOLDEN_RUNNER_PATH,
        MICROBENCH_RUNNER_PATH,
        TESTS_RUNNER_PATH,
    ]


def local_v8_payload_paths(v8_root: Path) -> list[Path]:
    return [
        v8_root / "d8",
        v8_root / "icudtl.dat",
        v8_root / "snapshot_blob.bin",
    ]


def sync_v8_to_device(device: DeviceContext, v8_root: Path) -> None:
    bin_dir = str(device.root / "bin")
    for local_path in local_v8_payload_paths(v8_root):
        remote_path = str(PurePosixPath(bin_dir) / local_path.name)
        adb_push_sync(device, local_path, remote_path)
    completed = adb_shell_completed(
        device.serial,
        f"chmod 755 {shlex.quote(str(device.root / 'bin' / 'd8'))}",
    )
    if completed.returncode != 0:
        raise DeviceError(f"chmod device d8: {completed.stderr or completed.stdout}".strip())


def sync_media_to_device(device: DeviceContext) -> None:
    source = f"{MEDIA_ROOT}/."
    adb_push_sync(device, source, str(device.root / "media"))


def adb_push_sync(device: DeviceContext, local_path: Path | str, remote_path: str) -> None:
    completed = subprocess.run(
        ["adb", "-s", device.serial, "push", "--sync", str(local_path), remote_path],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        output = completed_output_summary(completed)
        suffix = f": {output}" if output else f": exit {completed.returncode}"
        raise DeviceError(f"adb push --sync {local_path} {remote_path}{suffix}")
    forward_completed_output_to_stderr(completed)


def forward_completed_output_to_stderr(completed: subprocess.CompletedProcess[str]) -> None:
    if completed.stdout:
        print(completed.stdout, end="" if completed.stdout.endswith("\n") else "\n", file=sys.stderr)
    if completed.stderr:
        print(completed.stderr, end="" if completed.stderr.endswith("\n") else "\n", file=sys.stderr)


def completed_output_summary(completed: subprocess.CompletedProcess[str]) -> str:
    output = "\n".join(part.strip() for part in [completed.stderr, completed.stdout] if part.strip())
    return output or f"exit {completed.returncode}"


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
    run_id = (
        f"{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-"
        f"{os.getpid()}-{time.time_ns() % 1_000_000_000:09d}-{kind}"
    )
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


def missing_remote_commands(device: DeviceContext, commands: list[str]) -> list[str]:
    missing: list[str] = []
    for command in commands:
        completed = adb_shell_completed(device.serial, f"command -v {shlex.quote(command)} >/dev/null")
        if completed.returncode != 0:
            missing.append(command)
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


def push_bytes_to_device(
    device: DeviceContext,
    data: bytes,
    remote_path: str,
    *,
    suffix: str = ".wasm",
) -> None:
    with tempfile.NamedTemporaryFile(prefix="vip9r-perf-", suffix=suffix) as temp:
        temp.write(data)
        temp.flush()
        push_file_to_device(device, Path(temp.name), remote_path)


def adb_shell_completed(
    serial: str,
    script: str,
    *,
    timeout: int | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["adb", "-s", serial, "shell", script],
        capture_output=True,
        text=True,
        check=False,
        timeout=timeout,
    )


def parse_optional_int(value: str) -> int | None:
    if value == "":
        return None
    try:
        return int(value)
    except ValueError:
        return None


def cpu_groups(device: DeviceContext) -> list[dict[str, object]]:
    by_key: dict[tuple[str | None, int | None, int | None], list[CpuInfo]] = {}
    online = set(device.online_cpus)
    for cpu in device.cpus:
        if cpu.index not in online:
            continue
        midr = normalize_midr(cpu.midr)
        key = (midr, cpu.capacity, cpu.max_freq)
        by_key.setdefault(key, []).append(cpu)

    groups: list[dict[str, object]] = []
    for (midr, capacity, max_freq), cpus in sorted(by_key.items(), key=lambda item: item[1][0].index):
        cpu_indices = [cpu.index for cpu in cpus]
        decoded = decode_midr(midr)
        name = core_name_from_midr(decoded, midr)
        groups.append(
            {
                "name": name,
                "cpus": cpu_indices,
                "default_pin": f"cpu:{cpu_indices[0]}" if device.taskset is not None else None,
                "capacity": capacity,
                "max_freq": max_freq,
                "midr": midr,
                "midr_decoded": decoded,
            }
        )
    return groups


def normalize_midr(value: str | None) -> str | None:
    parsed = parse_midr(value)
    if parsed is None:
        return None
    return f"0x{parsed:08x}"


def parse_midr(value: str | None) -> int | None:
    if value is None:
        return None
    text = value.strip().lower()
    if text == "":
        return None
    try:
        return int(text, 0)
    except ValueError:
        try:
            return int(text, 16)
        except ValueError:
            return None


def decode_midr(value: str | None) -> dict[str, object] | None:
    midr = parse_midr(value)
    if midr is None:
        return None
    implementer = (midr >> 24) & 0xFF
    part = (midr >> 4) & 0xFFF
    name = ARM_PART_NAMES.get(part) if implementer == ARM_IMPLEMENTER else None
    return {
        "implementer": f"0x{implementer:02x}",
        "variant": (midr >> 20) & 0xF,
        "architecture": (midr >> 16) & 0xF,
        "part": f"0x{part:03x}",
        "revision": midr & 0xF,
        "name": name,
    }


def core_name_from_midr(decoded: dict[str, object] | None, midr: str | None) -> str:
    if decoded is None:
        return "unknown-midr"
    name = decoded.get("name")
    if isinstance(name, str):
        return name
    if midr is not None:
        return f"unknown-midr-{midr}"
    return "unknown-midr"


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


def host_d8_path() -> str:
    return os.environ.get("D8_LINUX64", "d8")


def verify_host_runtime() -> str:
    d8 = host_d8_path()
    try:
        completed = subprocess.run(
            [d8, "-e", "quit(0)"],
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError as error:
        raise RuntimeError(f"host d8 not found: {d8}") from error
    if completed.returncode != 0:
        raise RuntimeError(f"host d8 failed to launch: {completed.stderr or completed.stdout}".strip())
    return d8


def run_host_bench(
    label: str,
    wasm_fd: int,
    media_path: Path,
    frame_range: tuple[int, int] | None,
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        host_d8_path(),
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
    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False,
            timeout=HOST_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "host d8", HOST_D8_TIMEOUT_SECONDS, error)
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


def run_host_validation(
    label: str,
    wasm_fd: int,
    media_path: Path,
    validation: dict[str, object],
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        host_d8_path(),
        "--no-liftoff",
        "--module",
        str(GOLDEN_RUNNER_PATH),
        "--",
        proc_fd_path(wasm_fd),
    ]
    if validation.get("allow_mismatch") is True:
        cmd.append("--allow-mismatch")
    cmd.append(str(media_path))
    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False,
            timeout=HOST_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return completed_text_result(completed, label)


def run_host_microbench(
    label: str,
    wasm_fd: int,
    microbench: dict[str, int],
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        host_d8_path(),
        "--no-liftoff",
        "--module",
        str(MICROBENCH_RUNNER_PATH),
        "--",
        proc_fd_path(wasm_fd),
        "--slot",
        str(microbench["slot"]),
    ]

    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False,
            timeout=HOST_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "host d8", HOST_D8_TIMEOUT_SECONDS, error)
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


def run_host_tests(
    label: str,
    wasm_fd: int,
    tests: dict[str, str],
) -> dict[str, object]:
    os.lseek(wasm_fd, 0, os.SEEK_SET)
    cmd = [
        host_d8_path(),
        "--module",
        str(TESTS_RUNNER_PATH),
        "--",
        proc_fd_path(wasm_fd),
        "--json",
    ]
    test_filter = tests.get("filter")
    if test_filter is not None:
        cmd.append(test_filter)

    try:
        completed = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            check=False,
            timeout=HOST_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result(label, "host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return json_stdout_result(completed, label, "test")


def completed_text_result(
    completed: subprocess.CompletedProcess[str],
    label: str,
) -> dict[str, object]:
    result: dict[str, object] = {
        "ok": completed.returncode == 0,
        "label": label,
        "stdout": completed.stdout,
        "stderr": completed.stderr,
    }
    if completed.returncode != 0:
        result["returncode"] = completed.returncode
    return result


def json_stdout_result(
    completed: subprocess.CompletedProcess[str],
    label: str,
    json_label: str,
) -> dict[str, object]:
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        result = {
            "ok": False,
            "label": label,
            "error": f"invalid {json_label} JSON: {error}",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
        if completed.returncode != 0:
            result["returncode"] = completed.returncode
        return result

    result = {
        "ok": completed.returncode == 0 and report.get("ok") is True,
        "label": label,
        "report": report,
        "stderr": completed.stderr,
    }
    if completed.returncode != 0:
        result["returncode"] = completed.returncode
    return result


def timeout_result(
    label: str,
    timeout_kind: str,
    timeout_seconds: int,
    error: subprocess.TimeoutExpired,
) -> dict[str, object]:
    return {
        "ok": False,
        "label": label,
        "timeout": True,
        "timeout_kind": timeout_kind,
        "timeout_seconds": timeout_seconds,
        "stdout": timeout_output_text(error.stdout),
        "stderr": timeout_output_text(error.stderr),
    }


def timeout_output_text(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode("utf-8", errors="replace")
    return value


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
    send_record(sock, response_payload({**value, "type": "result"}))


def safe_send_final(sock: socket.socket, value: dict[str, object]) -> None:
    try:
        send_final(sock, value)
    except OSError:
        pass


def response_payload(value: dict[str, object]) -> bytes:
    sanitized = sanitize_response_value(value)
    assert isinstance(sanitized, dict)
    payload = encode_json_record(sanitized)
    if len(payload) <= MAX_RESPONSE_BYTES:
        return payload

    fallback = response_too_large_result(sanitized, payload)
    fallback_payload = encode_json_record(fallback)
    if len(fallback_payload) <= MAX_RESPONSE_BYTES:
        return fallback_payload

    return b'{"type":"result","ok":false,"error":"response too large"}'


def response_too_large_result(value: dict[str, object], payload: bytes) -> dict[str, object]:
    result: dict[str, object] = {
        "type": "result",
        "ok": False,
        "error": "response exceeds maximum size after truncation",
        "response_too_large": True,
        "encoded_bytes": len(payload),
        "max_response_bytes": MAX_RESPONSE_BYTES,
        "sha256": hashlib.sha256(payload).hexdigest(),
    }
    original_ok = value.get("ok")
    if isinstance(original_ok, bool):
        result["original_ok"] = original_ok
    for key in ["kind", "target", "serial"]:
        selected = value.get(key)
        if selected is None or isinstance(selected, (str, int, float, bool)):
            result[key] = selected
    return result


def sanitize_response_value(value: object) -> object:
    if isinstance(value, str):
        return truncate_response_text(value)
    if isinstance(value, dict):
        return {str(key): sanitize_response_value(child) for key, child in value.items()}
    if isinstance(value, list):
        return truncate_response_list(value)
    if value is None or isinstance(value, (bool, int, float)):
        return value
    return truncate_response_text(str(value))


def truncate_response_text(value: str) -> object:
    data = value.encode("utf-8")
    limit = RESPONSE_STRING_HEAD_BYTES + RESPONSE_STRING_TAIL_BYTES
    if len(data) <= limit:
        return value
    return {
        "truncated": True,
        "original_bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "head": data[:RESPONSE_STRING_HEAD_BYTES].decode("utf-8", errors="replace"),
        "tail": data[-RESPONSE_STRING_TAIL_BYTES:].decode("utf-8", errors="replace"),
    }


def truncate_response_list(value: list[object]) -> list[object]:
    max_items = RESPONSE_LIST_HEAD_ITEMS + RESPONSE_LIST_TAIL_ITEMS
    if len(value) <= max_items:
        return [sanitize_response_value(item) for item in value]

    head = [sanitize_response_value(item) for item in value[:RESPONSE_LIST_HEAD_ITEMS]]
    tail = [sanitize_response_value(item) for item in value[-RESPONSE_LIST_TAIL_ITEMS:]]
    marker = {
        "truncated": True,
        "original_length": len(value),
        "omitted_items": len(value) - max_items,
        "sha256": json_value_sha256(value),
    }
    return [*head, marker, *tail]


def json_value_sha256(value: object) -> str:
    payload = encode_json_record(value)
    return hashlib.sha256(payload).hexdigest()


def encode_json_record(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode("utf-8")


def send_record(sock: socket.socket, data: bytes) -> None:
    sent = sock.sendmsg([data])
    if sent != len(data):
        raise OSError(f"short seqpacket send: {sent}/{len(data)} bytes")


if __name__ == "__main__":
    raise SystemExit(main())
