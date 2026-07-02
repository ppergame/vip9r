#!/usr/bin/env python3
import argparse
import bisect
from collections import deque
from dataclasses import dataclass
import errno
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import shutil
import socket
import struct
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
U32_MAX = 2**32 - 1
PIN_HELP = "any, all, cpu:N, or mask:HEX"
HOST_D8_TIMEOUT_SECONDS = 180
DEVICE_D8_TIMEOUT_SECONDS = 180
ADB_D8_TIMEOUT_SECONDS = DEVICE_D8_TIMEOUT_SECONDS + 30
DEVICE_TIMEOUT_EXIT_CODE = 124
TARGETS = ("host", "device")
ASM_D8_TIMEOUT_SECONDS = 120
PROFILE_DEFAULT_FREQ = 1000
PROFILE_MAX_FREQ = 100000
PROFILE_REPORT_PERCENT_LIMIT = 0.05
# V8's perf map must land where simpleperf report looks for JIT symbols;
# /data/local/tmp is the validated location (see docs/d8.md).
DEVICE_PERF_MAP_DIR = PurePosixPath("/data/local/tmp")
JITDUMP_MAGIC = 0x4A695444
JITDUMP_CODE_LOAD = 0
WASM_TURBOFAN_NAME = re.compile(r"JS:(?P<mangled>.*)-(?P<index>\d+)-turbofan")
# WebAssembly.Module construction with --no-wasm-lazy-compilation compiles
# every declared function; nothing needs to execute, so no imports either.
ASM_COMPILE_JS = "new WebAssembly.Module(readbuffer(arguments[0]));\n"
# Native code depends on the CPU features the fixed d8 binary probes, not on
# the core model beyond those bits. qemu's -cpu max advertises v8.8+ features
# (HBC, MOPS) that no device in scope implements; pin conservative models.
ASM_RUNTIMES = {
    "host": {
        "objdump_arch": "i386:x86-64",
    },
    "arm64": {
        "v8_env": "V8_ANDROID_ARM64",
        "root_env": "ANDROID_RUNTIME_ROOT_ARM64",
        "qemu": "qemu-aarch64",
        "cpu": "cortex-a710",
        "objdump_arch": "aarch64",
        "apex": True,
    },
    "arm32": {
        "v8_env": "V8_ANDROID_ARM32",
        "root_env": "ANDROID_RUNTIME_ROOT_ARM32",
        "qemu": "qemu-arm",
        # qemu-arm has no ARMv8-A aarch32 core model; max is the closest to
        # the streamer's Cortex-A55 aarch32 state.
        "cpu": "max",
        "objdump_arch": "arm",
        "apex": False,
        # Bionic >= M reads personality(0xffffffff) at startup and treats
        # failure as fatal; the devcontainer's outer seccomp denies the
        # syscall with ENOSYS. Harmless where personality(2) works: the
        # query returns 0 either way and bionic ignores the set.
        "personality_shim": True,
    },
}
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
    fds: list[int]


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


@dataclass(frozen=True)
class DeviceSession:
    path: str
    baseline_path: str


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


def build_baseline(daemon_dir: Path) -> Path:
    source = build_release_wasm()
    path = daemon_dir / "baseline.wasm"
    shutil.copyfile(source, path)
    return path


def build_release_wasm() -> Path:
    rust_root = rust_workspace_root()
    target_dir = rust_root / "target/wasm-release"
    wasm_path = target_dir / "wasm32-unknown-unknown/release/vip9r.wasm"
    cmd = [
        "cargo",
        "build",
        "--manifest-path",
        str(rust_root / "Cargo.toml"),
        "--target-dir",
        str(target_dir),
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "vip9r",
        "--release",
    ]
    completed = subprocess.run(cmd, check=False)
    if completed.returncode != 0:
        raise RuntimeError(f"baseline wasm build exited {completed.returncode}")
    if not wasm_path.is_file():
        raise RuntimeError(f"baseline wasm build did not create {wasm_path}")
    return wasm_path


def rust_workspace_root() -> Path:
    rust_root = REPO_ROOT / "rust"
    if (rust_root / "Cargo.toml").is_file():
        return rust_root
    if (REPO_ROOT / "Cargo.toml").is_file():
        return REPO_ROOT
    raise RuntimeError(f"could not find vip9r Cargo workspace from {REPO_ROOT}")


def run_serve(args: argparse.Namespace) -> int:
    if args.serial is None and args.pin is not None:
        print("--pin requires --serial", file=sys.stderr)
        return 2
    if args.serial is not None and args.pin is None:
        print("--pin is required with --serial", file=sys.stderr)
        return 2

    args.pin = args.pin or "any"
    with tempfile.TemporaryDirectory(prefix="vip9r-perf-daemon-") as daemon_temp:
        daemon_dir = Path(daemon_temp)
        try:
            verify_local_wasm_runners()
            host_d8 = verify_host_runtime()
            baseline_path = build_baseline(daemon_dir)
        except Exception as error:
            print(f"host setup: {error}", file=sys.stderr)
            return 2

        device: DeviceContext | None = None
        device_session: DeviceSession | None = None
        default_pin_selection: PinSelection | None = None
        if args.serial is not None:
            try:
                device = probe_device(args.serial, DEFAULT_DEVICE_ROOT)
                default_pin_selection = resolve_and_verify_pin(device, args.pin)
            except Exception as error:
                print(f"device setup: {error}", file=sys.stderr)
                return 2

        SOCKET_PATH.parent.mkdir(parents=True, exist_ok=True)
        server = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_CANDIDATE_BYTES)
        server.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_RESPONSE_BYTES)
        try:
            server.bind(str(SOCKET_PATH))
            server.listen()
        except OSError as error:
            server.close()
            if error.errno == errno.EADDRINUSE:
                print(f"socket already exists: {SOCKET_PATH}", file=sys.stderr)
                return 2
            raise

        if device is not None:
            try:
                device_session = create_device_session(device, baseline_path)
            except Exception as error:
                server.close()
                try:
                    SOCKET_PATH.unlink()
                except FileNotFoundError:
                    pass
                print(f"device session setup: {error}", file=sys.stderr)
                return 2

        host_jobs = JobQueue("host")
        device_jobs = JobQueue("device")
        host_worker = threading.Thread(
            target=run_worker,
            args=("host", host_jobs, baseline_path, device, device_session, args.pin, daemon_dir),
            daemon=True,
        )
        device_worker = threading.Thread(
            target=run_worker,
            args=("device", device_jobs, baseline_path, device, device_session, args.pin, daemon_dir),
            daemon=True,
        )
        host_worker.start()
        device_worker.start()

        try:
            print(
                json.dumps(
                    {
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
                            "session_dir": device_session.path,
                            "timeout_seconds": DEVICE_D8_TIMEOUT_SECONDS,
                            "adb_timeout_seconds": ADB_D8_TIMEOUT_SECONDS,
                        }
                        if device is not None
                        and device_session is not None
                        and default_pin_selection is not None
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
            try:
                SOCKET_PATH.unlink()
            except FileNotFoundError:
                pass


def accept_one(conn: socket.socket, host_jobs: JobQueue, device_jobs: JobQueue) -> None:
    fds: list[int] = []
    try:
        # An asm request carries the client's output directory as an
        # SCM_RIGHTS fd riding on the request record.
        request_bytes, fds = recv_record_with_fds(conn, MAX_REQUEST_BYTES, "request")
        candidate = recv_record(conn, MAX_CANDIDATE_BYTES, "candidate wasm")
        conn.settimeout(None)
        request = json.loads(request_bytes.decode("utf-8"))
        if not isinstance(request, dict):
            raise ValueError("request must be a JSON object")
        target = target_from_request(request)
    except socket.timeout:
        close_fds(fds)
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
        close_fds(fds)
        safe_send_final(conn, {"ok": False, "error": str(error)})
        conn.close()
        return
    if target == "host":
        host_jobs.enqueue(Job(conn=conn, request=request, candidate=candidate, fds=fds))
    elif target == "device":
        device_jobs.enqueue(Job(conn=conn, request=request, candidate=candidate, fds=fds))
    else:
        close_fds(fds)
        safe_send_final(conn, {"ok": False, "error": f"unsupported target: {target!r}"})
        conn.close()


def close_fds(fds: list[int]) -> None:
    for fd in fds:
        try:
            os.close(fd)
        except OSError:
            pass


def run_worker(
    target: str,
    jobs: JobQueue,
    baseline_path: Path,
    device: DeviceContext | None,
    device_session: DeviceSession | None,
    default_pin: str,
    daemon_dir: Path,
) -> None:
    while True:
        job = jobs.pop()
        try:
            send_json(job.conn, {"type": "started", "target": target})
            with tempfile.TemporaryDirectory(prefix=f"{target}-job-", dir=daemon_dir) as job_temp:
                candidate_path = Path(job_temp) / "candidate.wasm"
                candidate_path.write_bytes(job.candidate)
                response = handle_request(
                    target,
                    baseline_path,
                    device,
                    device_session,
                    default_pin,
                    job.request,
                    candidate_path,
                    job.fds[0] if job.fds else None,
                )
        except Exception as error:
            response = {"ok": False, "error": str(error)}
        try:
            safe_send_final(job.conn, response)
        finally:
            close_fds(job.fds)
            job.conn.close()
            jobs.finish(job)


def handle_request(
    target: str,
    baseline_path: Path,
    device: DeviceContext | None,
    device_session: DeviceSession | None,
    default_pin: str,
    request: object,
    candidate_path: Path,
    dir_fd: int | None,
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
            return handle_host_validate_request(candidate_path, MEDIA_ROOT / media, validation)
        if device is None or device_session is None:
            return {
                "ok": False,
                "kind": "validate",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, default_pin)
        except ValueError as error:
            return {"ok": False, "kind": "validate", "target": "device", "error": str(error)}
        return handle_device_validate_request(
            device,
            device_session,
            candidate_path,
            media,
            validation,
            pin_name,
        )
    if kind == "tests":
        if target == "host":
            return handle_host_tests_request(request, candidate_path)
        if device is None or device_session is None:
            return {
                "ok": False,
                "kind": "tests",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, default_pin)
            tests = tests_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "tests", "target": "device", "error": str(error)}
        return handle_device_tests_request(
            device,
            device_session,
            candidate_path,
            tests,
            pin_name,
        )
    if kind == "microbench":
        if target == "host":
            return handle_host_microbench_request(request, candidate_path)
        if device is None or device_session is None:
            return {
                "ok": False,
                "kind": "microbench",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        try:
            pin_name = pin_from_request(request, default_pin)
            microbench = microbench_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "microbench", "target": "device", "error": str(error)}
        return handle_device_microbench_request(
            device,
            device_session,
            candidate_path,
            microbench,
            pin_name,
        )
    if kind == "asm":
        if target != "host":
            return {"ok": False, "kind": "asm", "target": target, "error": "asm runs on the daemon host"}
        try:
            arch = asm_arch_from_request(request)
        except ValueError as error:
            return {"ok": False, "kind": "asm", "target": "host", "error": str(error)}
        return handle_asm_request(arch, candidate_path, dir_fd)
    if kind == "profile":
        if target != "device":
            return {"ok": False, "kind": "profile", "target": target, "error": "profile requires target device"}
        if device is None or device_session is None:
            return {
                "ok": False,
                "kind": "profile",
                "target": "device",
                "error": "target device requires a daemon started with --serial",
            }
        if dir_fd is None:
            return {
                "ok": False,
                "kind": "profile",
                "target": "device",
                "error": "profile request carries no output directory fd",
            }
        try:
            media = media_from_request(request)
            frame_range = frame_range_from_request(request)
            freq = profile_freq_from_request(request)
            pin_name = pin_from_request(request, default_pin)
        except ValueError as error:
            return {"ok": False, "kind": "profile", "target": "device", "error": str(error)}
        return handle_device_profile_request(
            device,
            device_session,
            candidate_path,
            media,
            frame_range,
            freq,
            pin_name,
            dir_fd,
        )
    if kind != "bench":
        return {"ok": False, "error": f"unsupported request kind: {kind!r}"}

    try:
        media = media_from_request(request)
        frame_range = frame_range_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    if target == "host":
        return handle_host_bench_request(baseline_path, candidate_path, MEDIA_ROOT / media, frame_range)

    if device is None or device_session is None:
        return {
            "ok": False,
            "kind": "bench",
            "target": "device",
            "error": "target device requires a daemon started with --serial",
        }
    try:
        pin_name = pin_from_request(request, default_pin)
    except ValueError as error:
        return {"ok": False, "kind": "bench", "target": "device", "error": str(error)}
    return handle_device_bench_request(
        device,
        device_session,
        candidate_path,
        media,
        frame_range,
        pin_name,
    )


def handle_host_bench_request(
    baseline_path: Path,
    candidate_path: Path,
    media_path: Path,
    frame_range: tuple[int, int] | None,
) -> dict[str, object]:
    baseline_result = run_host_bench(baseline_path, media_path, frame_range)
    candidate_result = run_host_bench(candidate_path, media_path, frame_range)
    return {
        "ok": baseline_result.get("ok") is True and candidate_result.get("ok") is True,
        "kind": "bench",
        "target": "host",
        "host": {
            "media": str(media_path),
            "baseline": baseline_result,
            "candidate": candidate_result,
        },
    }


def handle_host_validate_request(
    candidate_path: Path,
    media_path: Path,
    validation: dict[str, object],
) -> dict[str, object]:
    candidate_result = run_host_validation(candidate_path, media_path, validation)
    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "validate",
        "target": "host",
        "host": {
            "media": str(media_path),
            "allow_mismatch": validation.get("allow_mismatch") is True,
            "candidate": candidate_result,
        },
    }


def handle_host_microbench_request(
    request: dict[str, object],
    candidate_path: Path,
) -> dict[str, object]:
    try:
        microbench = microbench_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    candidate_result = run_host_microbench(candidate_path, microbench)

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "microbench",
        "target": "host",
        "host": {
            "candidate": candidate_result,
        },
    }


def handle_host_tests_request(
    request: dict[str, object],
    candidate_path: Path,
) -> dict[str, object]:
    try:
        tests = tests_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

    candidate_result = run_host_tests(candidate_path, tests)

    return {
        "ok": candidate_result.get("ok") is True,
        "kind": "tests",
        "target": "host",
        "host": {
            "candidate": candidate_result,
        },
    }


def frame_range_from_request(request: dict[str, object]) -> tuple[int, int] | None:
    frames = request.get("frames")
    if frames is None:
        return None
    if not isinstance(frames, dict):
        raise ValueError("frames must be an object")
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


def profile_freq_from_request(request: dict[str, object]) -> int:
    freq = request.get("freq", PROFILE_DEFAULT_FREQ)
    if not isinstance(freq, int) or isinstance(freq, bool) or not 1 <= freq <= PROFILE_MAX_FREQ:
        raise ValueError(f"freq must be an integer in 1..{PROFILE_MAX_FREQ}")
    return freq


def validation_from_request(request: dict[str, object]) -> dict[str, object]:
    validation: dict[str, object] = {}
    allow_mismatch = request.get("allow_mismatch", False)
    if not isinstance(allow_mismatch, bool):
        raise ValueError("allow_mismatch must be boolean")
    if allow_mismatch:
        validation["allow_mismatch"] = True

    frames = frame_range_from_request(request)
    if frames is not None:
        validation["frames"] = frames

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
    device: DeviceContext,
    session: DeviceSession,
    local_candidate_path: Path,
    media: PurePosixPath,
    frame_range: tuple[int, int] | None,
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, session, "bench")
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        baseline_result_path = str(PurePosixPath(run_dir) / "baseline-result.json")
        result_path = str(PurePosixPath(run_dir) / "result.json")
        push_file_to_device(device, local_candidate_path, candidate_path)
        baseline_result = run_device_bench(
            device,
            device_session_path(session, GOLDEN_RUNNER_PATH),
            session.baseline_path,
            media_path,
            frame_range,
            pin,
            baseline_result_path,
        )
        candidate_result = run_device_bench(
            device,
            device_session_path(session, GOLDEN_RUNNER_PATH),
            candidate_path,
            media_path,
            frame_range,
            pin,
            result_path,
        )
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "bench", "target": "device", "serial": device.serial, "error": str(error)}

    return {
        "ok": baseline_result.get("ok") is True and candidate_result.get("ok") is True,
        "kind": "bench",
        "target": "device",
        "serial": device.serial,
        "device": {
            "summary": device_summary(device),
            "pin": pin_summary(pin),
            "session_dir": session.path,
            "run_dir": run_dir,
            "media": media_path,
            "result": result_path,
            "baseline_result": baseline_result_path,
            "baseline_wasm": session.baseline_path,
            "baseline": baseline_result,
            "candidate": candidate_result,
        },
    }


def handle_device_microbench_request(
    device: DeviceContext,
    session: DeviceSession,
    local_candidate_path: Path,
    microbench: dict[str, int],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        run_dir = create_device_run_dir(device, session, "microbench")
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        result_path = str(PurePosixPath(run_dir) / "result.json")
        push_file_to_device(device, local_candidate_path, candidate_path)
        candidate_result = run_device_microbench(
            device,
            device_session_path(session, MICROBENCH_RUNNER_PATH),
            candidate_path,
            microbench,
            pin,
            result_path,
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
            "session_dir": session.path,
            "run_dir": run_dir,
            "result": result_path,
            "candidate": candidate_result,
        },
    }


def handle_device_tests_request(
    device: DeviceContext,
    session: DeviceSession,
    local_candidate_path: Path,
    tests: dict[str, str],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        run_dir = create_device_run_dir(device, session, "tests")
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        result_path = str(PurePosixPath(run_dir) / "result.json")
        push_file_to_device(device, local_candidate_path, candidate_path)
        candidate_result = run_device_tests(
            device,
            device_session_path(session, TESTS_RUNNER_PATH),
            candidate_path,
            tests,
            pin,
            result_path,
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
            "session_dir": session.path,
            "run_dir": run_dir,
            "result": result_path,
            "candidate": candidate_result,
        },
    }


def handle_device_validate_request(
    device: DeviceContext,
    session: DeviceSession,
    local_candidate_path: Path,
    media: PurePosixPath,
    validation: dict[str, object],
    pin_name: str,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, session, "validate")
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        result_path = str(PurePosixPath(run_dir) / "result.json")
        push_file_to_device(device, local_candidate_path, candidate_path)
        candidate_result = run_device_validation(
            device,
            device_session_path(session, GOLDEN_RUNNER_PATH),
            candidate_path,
            media_path,
            validation,
            pin,
            result_path,
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
            "session_dir": session.path,
            "run_dir": run_dir,
            "media": media_path,
            "result": result_path,
            "allow_mismatch": validation.get("allow_mismatch") is True,
            "candidate": candidate_result,
        },
    }


def handle_device_profile_request(
    device: DeviceContext,
    session: DeviceSession,
    local_candidate_path: Path,
    media: PurePosixPath,
    frame_range: tuple[int, int] | None,
    freq: int,
    pin_name: str,
    dir_fd: int,
) -> dict[str, object]:
    try:
        pin = resolve_and_verify_pin(device, pin_name)
        missing_commands = missing_remote_commands(device, ["simpleperf"])
        if missing_commands:
            raise DeviceError("missing device commands: " + ", ".join(missing_commands))
        media_path = str(device.root / "media" / media)
        verify_remote_files(device, [media_path, f"{media_path}.md5"])
        run_dir = create_device_run_dir(device, session, "profile")
        candidate_path = str(PurePosixPath(run_dir) / "candidate.wasm")
        result_path = str(PurePosixPath(run_dir) / "result.json")
        perf_data_path = str(PurePosixPath(run_dir) / "perf.data")
        push_file_to_device(device, local_candidate_path, candidate_path)
        remove_device_perf_maps(device)
        d8_args = [
            "./d8",
            "--no-liftoff",
            "--perf-basic-prof",
            f"--perf-basic-prof-path={DEVICE_PERF_MAP_DIR}",
            "--module",
            device_session_path(session, GOLDEN_RUNNER_PATH),
            "--",
            candidate_path,
            "--bench",
        ]
        if frame_range is not None:
            start, last = frame_range
            d8_args.extend(["--frames", f"{start}:{last}"])
        d8_args.append(media_path)
        shell_command = device_profile_shell_command(
            device, d8_args, pin, result_path, freq, perf_data_path
        )
        candidate_result = run_device_shell_json(
            device, shell_command, pin, "benchmark", result_path
        )
        files: list[str] = []
        samples: int | None = None
        if candidate_result.get("ok") is True:
            work_dir = local_candidate_path.parent
            pull_device_file_at(device, perf_data_path, dir_fd, "perf.data", work_dir)
            map_names = pull_device_perf_maps(device, dir_fd, work_dir)
            report_text = build_profile_report(
                work_dir / "perf.data", [work_dir / name for name in map_names]
            )
            samples = profile_report_samples(report_text)
            write_file_at(dir_fd, "report.txt", report_text.encode("utf-8"))
            files = ["report.txt", "perf.data", *map_names]
    except (DeviceError, ValueError) as error:
        return {"ok": False, "kind": "profile", "target": "device", "serial": device.serial, "error": str(error)}

    ok = candidate_result.get("ok") is True
    return {
        "ok": ok,
        "kind": "profile",
        "target": "device",
        "serial": device.serial,
        "device": {
            "pin": pin_summary(pin),
            "run_dir": run_dir,
            "media": media_path,
            "freq": freq,
            "samples": samples,
            "files": files,
            # The report file is the deliverable; keep only enough of the
            # bench result to judge whether the profiled run was healthy.
            "candidate": profile_candidate_summary(candidate_result) if ok else candidate_result,
        },
    }


def profile_candidate_summary(candidate_result: dict[str, object]) -> dict[str, object]:
    summary: dict[str, object] = {"ok": candidate_result.get("ok")}
    report = candidate_result.get("report")
    if isinstance(report, dict):
        measurement = report.get("measurement")
        if isinstance(measurement, dict):
            summary["measurement"] = {
                key: measurement.get(key)
                for key in ("passes", "minPassMs", "msPerFrame", "fps")
            }
    stderr = candidate_result.get("stderr")
    if stderr:
        summary["stderr"] = stderr
    telemetry = candidate_result.get("telemetry")
    if telemetry:
        summary["telemetry"] = telemetry
    return summary


def build_profile_report(perf_data_path: Path, map_paths: list[Path]) -> str:
    perf_data = parse_perf_data(perf_data_path)
    jit_ranges = load_jit_symbol_ranges(map_paths)
    jit_starts = [start for start, _end, _symbol in jit_ranges]
    event_counts: dict[str, int] = {}
    sample_counts: dict[str, int] = {}
    total_events = 0
    for ip, period in perf_data.samples:
        index = bisect.bisect_right(jit_starts, ip) - 1
        if index >= 0 and ip < jit_ranges[index][1]:
            symbol = jit_ranges[index][2]
        else:
            # Newest mapping wins: d8 mmaps input files and later unmaps
            # them, and munmap is never recorded.
            symbol = next(
                (
                    filename
                    for start, end, filename in reversed(perf_data.mmaps)
                    if start <= ip < end
                ),
                "<unmapped>",
            )
        event_counts[symbol] = event_counts.get(symbol, 0) + period
        sample_counts[symbol] = sample_counts.get(symbol, 0) + 1
        total_events += period
    lines = [
        "Host-attributed profile: V8 perf-map JIT symbols take precedence over",
        "file mappings; non-JIT samples are aggregated per mapped file.",
        f"Samples: {len(perf_data.samples)}",
        f"Event count: {total_events}",
        "",
        f"{'Overhead':<9} {'Samples':<8} Symbol",
    ]
    for symbol, events in sorted(event_counts.items(), key=lambda item: -item[1]):
        percent = 100.0 * events / total_events if total_events else 0.0
        if percent < PROFILE_REPORT_PERCENT_LIMIT:
            continue
        lines.append(f"{f'{percent:.2f}%':<9} {sample_counts[symbol]:<8} {symbol}")
    return demangle_profile_report("\n".join(lines) + "\n")


PERF_RECORD_MMAP = 1
PERF_RECORD_SAMPLE = 9
PERF_RECORD_MMAP2 = 10
PERF_SAMPLE_IP = 1 << 0
PERF_SAMPLE_PERIOD = 1 << 8
PERF_SAMPLE_IDENTIFIER = 1 << 16
# 8-byte PERF_RECORD_SAMPLE fields laid out before PERIOD, in record order:
# IDENTIFIER, IP, TID, TIME, ADDR, ID, STREAM_ID, CPU.
PERF_SAMPLE_FIELDS_BEFORE_PERIOD = (
    PERF_SAMPLE_IDENTIFIER,
    PERF_SAMPLE_IP,
    1 << 1,  # TID
    1 << 2,  # TIME
    1 << 3,  # ADDR
    1 << 6,  # ID
    1 << 9,  # STREAM_ID
    1 << 7,  # CPU
)


@dataclass
class PerfData:
    # (ip, period) per sample record.
    samples: list[tuple[int, int]]
    # (start, end, filename) per mmap record, in record order.
    mmaps: list[tuple[int, int, str]]


def parse_perf_data(path: Path) -> PerfData:
    data = path.read_bytes()
    if data[:8] != b"PERFILE2":
        raise DeviceError(f"{path.name}: not a PERFILE2 perf.data file")
    attrs_offset = struct.unpack_from("<Q", data, 24)[0]
    data_offset, data_size = struct.unpack_from("<QQ", data, 40)
    sample_type = struct.unpack_from("<Q", data, attrs_offset + 24)[0]
    if not sample_type & PERF_SAMPLE_IP:
        raise DeviceError(f"{path.name}: samples carry no IP field")
    ip_offset = 8 if sample_type & PERF_SAMPLE_IDENTIFIER else 0
    period_offset = sum(
        8 for bit in PERF_SAMPLE_FIELDS_BEFORE_PERIOD if sample_type & bit
    )
    has_period = bool(sample_type & PERF_SAMPLE_PERIOD)
    samples: list[tuple[int, int]] = []
    mmaps: list[tuple[int, int, str]] = []
    position = data_offset
    end = data_offset + data_size
    while position + 8 <= end:
        record_type, _misc, record_size = struct.unpack_from("<IHH", data, position)
        if record_size < 8 or position + record_size > end:
            break
        body = position + 8
        record_end = position + record_size
        if record_type == PERF_RECORD_SAMPLE:
            ip = struct.unpack_from("<Q", data, body + ip_offset)[0]
            period = 1
            if has_period and body + period_offset + 8 <= record_end:
                period = struct.unpack_from("<Q", data, body + period_offset)[0]
            samples.append((ip, period))
        elif record_type in (PERF_RECORD_MMAP, PERF_RECORD_MMAP2):
            start, length = struct.unpack_from("<QQ", data, body + 8)
            name_offset = body + (32 if record_type == PERF_RECORD_MMAP else 64)
            name_end = data.find(b"\0", name_offset, record_end)
            if name_end > name_offset:
                filename = data[name_offset:name_end].decode("utf-8", "replace")
                mmaps.append((start, start + length, filename))
        position += record_size
    return PerfData(samples=samples, mmaps=mmaps)


def load_jit_symbol_ranges(map_paths: list[Path]) -> list[tuple[int, int, str]]:
    ranges: list[tuple[int, int, str]] = []
    for path in map_paths:
        for line in path.read_text(errors="replace").splitlines():
            parts = line.split(maxsplit=2)
            if len(parts) != 3:
                continue
            try:
                start = int(parts[0], 16)
                size = int(parts[1], 16)
            except ValueError:
                continue
            ranges.append((start, start + size, parts[2]))
    ranges.sort(key=lambda entry: entry[0])
    return ranges


PROFILE_SYMBOL = re.compile(r"JS:(?P<mangled>\S+?)-(?P<index>\d+)-turbofan")


def demangle_profile_report(text: str) -> str:
    def replace(match: re.Match[str]) -> str:
        return f"wasm[{match.group('index')}] {demangle_rust(match.group('mangled'))}"

    return PROFILE_SYMBOL.sub(replace, text)


def profile_report_samples(text: str) -> int | None:
    match = re.search(r"^Samples: (\d+)", text, re.MULTILINE)
    return int(match.group(1)) if match else None


def list_device_perf_maps(device: DeviceContext) -> list[str]:
    completed = adb_shell_completed(
        device.serial, f"ls {DEVICE_PERF_MAP_DIR}/perf-*.map 2>/dev/null"
    )
    if completed.returncode != 0:
        return []
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def remove_device_perf_maps(device: DeviceContext) -> None:
    adb_shell_completed(device.serial, f"rm -f {DEVICE_PERF_MAP_DIR}/perf-*.map")


def pull_device_perf_maps(
    device: DeviceContext, dir_fd: int, work_dir: Path
) -> list[str]:
    names: list[str] = []
    for remote_path in list_device_perf_maps(device):
        name = PurePosixPath(remote_path).name
        pull_device_file_at(device, remote_path, dir_fd, name, work_dir)
        names.append(name)
    remove_device_perf_maps(device)
    return names


def pull_device_file_at(
    device: DeviceContext,
    remote_path: str,
    dir_fd: int,
    name: str,
    work_dir: Path,
) -> None:
    local_path = work_dir / name
    completed = subprocess.run(
        ["adb", "-s", device.serial, "pull", remote_path, str(local_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise DeviceError(f"adb pull {remote_path}: {completed.stderr or completed.stdout}".strip())
    write_file_at(dir_fd, name, local_path.read_bytes())


def media_from_request(request: dict[str, object]) -> PurePosixPath:
    media = request.get("media")
    if not isinstance(media, str) or media == "":
        raise ValueError("request requires media")
    path = PurePosixPath(media)
    if path.is_absolute() or path == PurePosixPath(".") or ".." in path.parts:
        raise ValueError("media must be a corpus-relative path without '..'")
    return path


def run_device_bench(
    device: DeviceContext,
    runner_path: str,
    wasm_path: str,
    media_path: str,
    frame_range: tuple[int, int] | None,
    pin: PinSelection,
    result_path: str,
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
        d8_args.extend(["--frames", f"{start}:{last}"])
    d8_args.append(media_path)
    return run_device_json(device, d8_args, pin, "benchmark", result_path)


def run_device_validation(
    device: DeviceContext,
    runner_path: str,
    wasm_path: str,
    media_path: str,
    validation: dict[str, object],
    pin: PinSelection,
    result_path: str,
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
    frames = validation.get("frames")
    if frames is not None:
        start, last = frames
        d8_args.extend(["--frames", f"{start}:{last}"])
    d8_args.append(media_path)
    return run_device_json(device, d8_args, pin, "validation", result_path)


def run_device_microbench(
    device: DeviceContext,
    runner_path: str,
    wasm_path: str,
    microbench: dict[str, int],
    pin: PinSelection,
    result_path: str,
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
    return run_device_json(device, d8_args, pin, "microbenchmark", result_path)


def run_device_tests(
    device: DeviceContext,
    runner_path: str,
    wasm_path: str,
    tests: dict[str, str],
    pin: PinSelection,
    result_path: str,
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
    return run_device_json(device, d8_args, pin, "test", result_path)


def run_device_json(
    device: DeviceContext,
    d8_args: list[str],
    pin: PinSelection,
    json_label: str,
    result_path: str,
) -> dict[str, object]:
    shell_command = device_d8_shell_command(device, d8_args, pin, result_path)
    return run_device_shell_json(device, shell_command, pin, json_label, result_path)


def run_device_shell_json(
    device: DeviceContext,
    shell_command: str,
    pin: PinSelection,
    json_label: str,
    result_path: str,
) -> dict[str, object]:
    telemetry: dict[str, object] = {}
    record_telemetry(telemetry, device, pin, "start")
    result = execute_device_json(device, shell_command, json_label, result_path)
    record_telemetry(telemetry, device, pin, "end")
    if telemetry:
        result["telemetry"] = telemetry
    return result


def record_telemetry(
    telemetry: dict[str, object],
    device: DeviceContext,
    pin: PinSelection,
    edge: str,
) -> None:
    freq = read_pinned_cpu_freq_khz(device, pin)
    if freq is not None:
        telemetry[f"freq_{edge}_khz"] = freq
    temp = read_cpu_temp_c(device)
    if temp is not None:
        telemetry[f"temp_{edge}_c"] = temp


def read_pinned_cpu_freq_khz(device: DeviceContext, pin: PinSelection) -> int | None:
    cpus = pin.verified_cpus or pin.cpus or device.online_cpus
    if not cpus:
        return None
    path = f"/sys/devices/system/cpu/cpu{cpus[0]}/cpufreq/scaling_cur_freq"
    completed = adb_shell_completed(device.serial, f"cat {path}")
    if completed.returncode != 0:
        return None
    return parse_optional_int(completed.stdout.strip())


def read_cpu_temp_c(device: DeviceContext) -> float | None:
    completed = adb_shell_completed(device.serial, "dumpsys thermalservice")
    if completed.returncode != 0:
        return None
    return parse_hal_cpu_temp(completed.stdout)


def parse_hal_cpu_temp(text: str) -> float | None:
    # Only the "Current temperatures from HAL" section tracks the sensor;
    # "Cached temperatures" is event-driven and can be stale by tens of C.
    # Prefer the Pixel's BIG cluster sensor, else the first CPU-type sensor
    # (streamer: soc_max).
    in_hal = False
    big = None
    cpu = None
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.endswith(":"):
            in_hal = stripped == "Current temperatures from HAL:"
            continue
        if not in_hal:
            continue
        match = re.search(r"mValue=([0-9]+(?:\.[0-9]+)?)", stripped)
        if match is None:
            continue
        value = float(match.group(1))
        if "mName=BIG" in stripped:
            big = value
        elif "mType=0," in stripped and cpu is None:
            cpu = value
    return big if big is not None else cpu


def execute_device_json(
    device: DeviceContext,
    shell_command: str,
    json_label: str,
    result_path: str,
) -> dict[str, object]:
    try:
        completed = adb_shell_completed(
            device.serial,
            shell_command,
            timeout=ADB_D8_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as error:
        return timeout_result("adb shell", ADB_D8_TIMEOUT_SECONDS, error)
    stdout, read_error = read_remote_text_file(device, result_path)
    returncode, stderr = device_result_status(completed.returncode, completed.stderr, read_error)
    try:
        report = json.loads(stdout)
    except json.JSONDecodeError as error:
        result = {
            "ok": False,
            "error": f"invalid {json_label} JSON: {error}",
            "stdout": stdout,
            "stderr": stderr,
        }
        if returncode != 0:
            result["returncode"] = returncode
        if returncode == DEVICE_TIMEOUT_EXIT_CODE:
            result["timeout"] = True
            result["timeout_kind"] = "device d8"
            result["timeout_seconds"] = DEVICE_D8_TIMEOUT_SECONDS
        return result
    if not isinstance(report, dict):
        result = {
            "ok": False,
            "error": f"invalid {json_label} JSON: expected object",
            "stdout": stdout,
            "stderr": stderr,
        }
        if returncode != 0:
            result["returncode"] = returncode
        return result

    result = {
        "ok": returncode == 0 and report.get("ok") is True,
        "report": report,
        "stderr": stderr,
    }
    if returncode != 0:
        result["returncode"] = returncode
    if returncode == DEVICE_TIMEOUT_EXIT_CODE:
        result["timeout"] = True
        result["timeout_kind"] = "device d8"
        result["timeout_seconds"] = DEVICE_D8_TIMEOUT_SECONDS
    return result


def device_result_status(
    returncode: int,
    stderr: str,
    read_error: str | None,
) -> tuple[int, str]:
    if read_error is not None:
        stderr = "\n".join(part for part in [stderr, read_error] if part)
        if returncode == 0:
            returncode = 1
    return returncode, stderr


def read_remote_text_file(device: DeviceContext, path: str) -> tuple[str, str | None]:
    completed = adb_shell_completed(device.serial, f"cat {shlex.quote(path)}")
    if completed.returncode == 0:
        return completed.stdout, None
    output = completed.stderr or completed.stdout or f"exit {completed.returncode}"
    return "", f"read device file {path}: {output.strip()}"


def device_d8_shell_command(
    device: DeviceContext,
    d8_args: list[str],
    pin: PinSelection,
    stdout_path: str,
) -> str:
    return device_timed_shell_command(device, pinned_d8_command(d8_args, pin, stdout_path))


def device_profile_shell_command(
    device: DeviceContext,
    d8_args: list[str],
    pin: PinSelection,
    stdout_path: str,
    freq: int,
    perf_data_path: str,
) -> str:
    # simpleperf itself stays unpinned; the perf events follow the pinned d8
    # child, so only the workload competes for the selected CPUs.
    inner = "exec " + pinned_d8_command(d8_args, pin, stdout_path)
    record = " ".join(
        shlex.quote(arg)
        for arg in [
            "simpleperf",
            "record",
            "-f",
            str(freq),
            "-o",
            perf_data_path,
            "--",
            "sh",
            "-c",
            inner,
        ]
    )
    return device_timed_shell_command(device, record)


def pinned_d8_command(d8_args: list[str], pin: PinSelection, stdout_path: str) -> str:
    command = " ".join(shlex.quote(arg) for arg in d8_args)
    command = f"{command} > {shlex.quote(stdout_path)}"
    if pin.mask is not None:
        command = f"taskset {shlex.quote(pin.mask)} {command}"
    return command


def device_timed_shell_command(device: DeviceContext, command: str) -> str:
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


def create_device_session(device: DeviceContext, baseline_path: Path) -> DeviceSession:
    session_dir = str(device.root / "runs" / unique_run_name("session"))
    ensure_remote_dir(device, session_dir)

    remote_baseline_path = str(PurePosixPath(session_dir) / "baseline.wasm")
    push_file_to_device(device, baseline_path, remote_baseline_path)
    for local_path in sorted(GOLDEN_RUNNER_PATH.parent.glob("*.js")):
        push_file_to_device(device, local_path, str(PurePosixPath(session_dir) / local_path.name))

    return DeviceSession(
        path=session_dir,
        baseline_path=remote_baseline_path,
    )


def device_session_path(session: DeviceSession, local_path: Path) -> str:
    return str(PurePosixPath(session.path) / local_path.name)


def create_device_run_dir(device: DeviceContext, session: DeviceSession, kind: str) -> str:
    run_dir = str(PurePosixPath(session.path) / unique_run_name("run", kind))
    ensure_remote_dir(device, run_dir)
    return run_dir


def unique_run_name(prefix: str, kind: str | None = None) -> str:
    name = (
        f"{prefix}-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}-"
        f"{os.getpid()}-{time.time_ns() % 1_000_000_000:09d}"
    )
    if kind is not None:
        name = f"{name}-{kind}"
    return name


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


def push_file_to_device(device: DeviceContext, local_path: Path, remote_path: str) -> None:
    completed = subprocess.run(
        ["adb", "-s", device.serial, "push", str(local_path), remote_path],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise DeviceError(f"adb push {local_path} {remote_path}: {completed.stderr or completed.stdout}".strip())


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
    wasm_path: Path,
    media_path: Path,
    frame_range: tuple[int, int] | None,
) -> dict[str, object]:
    cmd = [
        host_d8_path(),
        "--no-liftoff",
        "--module",
        str(GOLDEN_RUNNER_PATH),
        "--",
        str(wasm_path),
        "--bench",
    ]
    if frame_range is not None:
        start, last = frame_range
        cmd.extend(["--frames", f"{start}:{last}"])
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
        return timeout_result("host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return json_stdout_result(completed, "benchmark")


def run_host_validation(
    wasm_path: Path,
    media_path: Path,
    validation: dict[str, object],
) -> dict[str, object]:
    cmd = [
        host_d8_path(),
        "--no-liftoff",
        "--module",
        str(GOLDEN_RUNNER_PATH),
        "--",
        str(wasm_path),
    ]
    if validation.get("allow_mismatch") is True:
        cmd.append("--allow-mismatch")
    frames = validation.get("frames")
    if frames is not None:
        start, last = frames
        cmd.extend(["--frames", f"{start}:{last}"])
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
        return timeout_result("host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return json_stdout_result(completed, "validation")


def run_host_microbench(
    wasm_path: Path,
    microbench: dict[str, int],
) -> dict[str, object]:
    cmd = [
        host_d8_path(),
        "--no-liftoff",
        "--module",
        str(MICROBENCH_RUNNER_PATH),
        "--",
        str(wasm_path),
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
        return timeout_result("host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return json_stdout_result(completed, "microbenchmark")


def run_host_tests(
    wasm_path: Path,
    tests: dict[str, str],
) -> dict[str, object]:
    cmd = [
        host_d8_path(),
        "--module",
        str(TESTS_RUNNER_PATH),
        "--",
        str(wasm_path),
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
        return timeout_result("host d8", HOST_D8_TIMEOUT_SECONDS, error)
    return json_stdout_result(completed, "test")


@dataclass(frozen=True)
class WasmFunctionCode:
    index: int
    mangled: str
    demangled: str
    code_addr: int
    code: bytes


def asm_arch_from_request(request: dict[str, object]) -> str:
    arch = request.get("arch")
    if not isinstance(arch, str) or arch not in ASM_RUNTIMES:
        raise ValueError(f"arch must be one of: {', '.join(sorted(ASM_RUNTIMES))}")
    return arch


def handle_asm_request(
    arch: str,
    candidate_path: Path,
    dir_fd: int | None,
) -> dict[str, object]:
    runtime = ASM_RUNTIMES[arch]
    base: dict[str, object] = {"kind": "asm", "target": "host", "arch": arch}
    if "cpu" in runtime:
        base["cpu"] = runtime["cpu"]
    if dir_fd is None:
        return {**base, "ok": False, "error": "asm request carries no output directory fd"}
    try:
        tools = resolve_asm_tools(runtime)
    except RuntimeError as error:
        return {**base, "ok": False, "error": str(error)}

    out_dir = candidate_path.parent / "perf-out"
    out_dir.mkdir()
    shutil.copyfile(candidate_path, out_dir / "candidate.wasm")
    (out_dir / "compile.js").write_text(ASM_COMPILE_JS)
    seccomp_fd: int | None = None
    if runtime.get("personality_shim"):
        bpf_path = out_dir / "personality0.bpf"
        bpf_path.write_bytes(personality_seccomp_bpf())
        seccomp_fd = os.open(bpf_path, os.O_RDONLY)
    try:
        completed = subprocess.run(
            asm_compile_command(runtime, tools, out_dir, seccomp_fd),
            capture_output=True,
            text=True,
            check=False,
            timeout=ASM_D8_TIMEOUT_SECONDS,
            pass_fds=() if seccomp_fd is None else (seccomp_fd,),
            # --perf-prof also opens v8.log in cwd; keep it out of the
            # daemon's cwd.
            cwd=out_dir,
        )
    except subprocess.TimeoutExpired as error:
        return {**base, **timeout_result("asm d8", ASM_D8_TIMEOUT_SECONDS, error)}
    finally:
        if seccomp_fd is not None:
            os.close(seccomp_fd)

    if completed.returncode != 0:
        return {
            **base,
            "ok": False,
            "error": "wasm compile failed",
            "returncode": completed.returncode,
            "stdout": completed.stdout[-2000:],
            "stderr": completed.stderr[-2000:],
        }

    dumps = sorted(out_dir.glob("jit-*.dump"))
    if len(dumps) != 1:
        return {**base, "ok": False, "error": f"expected one jitdump, found {len(dumps)}"}
    functions = wasm_functions_from_jitdump(dumps[0])
    if not functions:
        return {**base, "ok": False, "error": "no TurboFan wasm records in jitdump"}

    for fn in functions:
        text = disassemble_wasm_function(tools["objdump"], runtime["objdump_arch"], fn, out_dir)
        write_file_at(dir_fd, asm_file_name(fn), text.encode("utf-8"))
    return {**base, "ok": True, "functions": len(functions)}


def write_file_at(dir_fd: int, name: str, data: bytes) -> None:
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644, dir_fd=dir_fd)
    with os.fdopen(fd, "wb") as file:
        file.write(data)


def resolve_asm_tools(runtime: dict[str, object]) -> dict[str, str]:
    tools: dict[str, str] = {}
    if "qemu" in runtime:
        for key, env_key in (("v8_root", "v8_env"), ("runtime_root", "root_env")):
            env_name = str(runtime[env_key])
            value = os.environ.get(env_name)
            if value is None:
                raise RuntimeError(f"{env_name} is not set")
            tools[key] = value
        if not (Path(tools["v8_root"]) / "d8").is_file():
            raise RuntimeError(f"missing d8 in {tools['v8_root']}")
        for command in (str(runtime["qemu"]), "bwrap"):
            path = shutil.which(command)
            if path is None:
                raise RuntimeError(f"{command} not found on PATH")
            tools[command if command == "bwrap" else "qemu"] = path
    objdump = os.environ.get("OBJDUMP_MULTIARCH")
    if objdump is None:
        raise RuntimeError("OBJDUMP_MULTIARCH is not set")
    tools["objdump"] = objdump
    return tools


def personality_seccomp_bpf() -> bytes:
    # Classic-BPF seccomp filter: make personality(2) return 0 instead of the
    # host kernel's ENOSYS denial. Stacked filters resolve to this ERRNO(0).
    bpf_ld_w_abs, bpf_jeq_k, bpf_ret_k = 0x20, 0x15, 0x06
    ret_allow, ret_errno_0 = 0x7FFF0000, 0x00050000
    personality_nr_x86_64 = 135
    program = [
        (bpf_ld_w_abs, 0, 0, 0),
        (bpf_jeq_k, 0, 1, personality_nr_x86_64),
        (bpf_ret_k, 0, 0, ret_errno_0),
        (bpf_ret_k, 0, 0, ret_allow),
    ]
    return b"".join(struct.pack("<HBBI", *instruction) for instruction in program)


def asm_compile_command(
    runtime: dict[str, object],
    tools: dict[str, str],
    out_dir: Path,
    seccomp_fd: int | None,
) -> list[str]:
    d8_args = [
        "--no-liftoff",
        "--no-wasm-lazy-compilation",
        "--perf-prof",
    ]
    if "qemu" not in runtime:
        return [
            host_d8_path(),
            *d8_args,
            f"--perf-prof-path={out_dir}",
            str(out_dir / "compile.js"),
            "--",
            str(out_dir / "candidate.wasm"),
        ]
    v8_root = Path(tools["v8_root"])
    runtime_root = Path(tools["runtime_root"])
    apex_binds = (
        ["--ro-bind", str(runtime_root / "apex"), "/apex"] if runtime["apex"] else []
    )
    seccomp_args = [] if seccomp_fd is None else ["--seccomp", str(seccomp_fd)]
    return [
        tools["bwrap"],
        *seccomp_args,
        # 32-bit bionic packs pthread mutex owner tids into 16 bits; host
        # tids >= 2^16 self-deadlock on the first recursive lock. A fresh
        # pid namespace keeps guest tids small.
        "--unshare-pid",
        "--ro-bind", "/nix", "/nix",
        "--ro-bind", str(runtime_root / "system"), "/system",
        *apex_binds,
        "--bind", str(out_dir), "/out",
        "--tmpfs", "/tmp",
        "--proc", "/proc",
        "--dev", "/dev",
        "--chdir", str(v8_root),
        tools["qemu"],
        "-cpu", str(runtime["cpu"]),
        str(v8_root / "d8"),
        *d8_args,
        "--perf-prof-path=/out",
        "/out/compile.js",
        "--",
        "/out/candidate.wasm",
    ]


def wasm_functions_from_jitdump(path: Path) -> list[WasmFunctionCode]:
    data = path.read_bytes()
    if len(data) < 16:
        raise RuntimeError(f"short jitdump: {path}")
    magic, _version, header_size, _elf_mach = struct.unpack_from("<IIII", data, 0)
    if magic != JITDUMP_MAGIC:
        raise RuntimeError(f"bad jitdump magic in {path}")
    by_index: dict[int, WasmFunctionCode] = {}
    offset = header_size
    while offset + 16 <= len(data):
        record_id, total_size, _timestamp = struct.unpack_from("<IIQ", data, offset)
        if total_size == 0 or offset + total_size > len(data):
            break
        if record_id == JITDUMP_CODE_LOAD:
            _pid, _tid, _vma, code_addr, code_size, _code_index = struct.unpack_from(
                "<IIQQQQ", data, offset + 16
            )
            name_offset = offset + 16 + 40
            name_end = data.index(b"\x00", name_offset)
            name = data[name_offset:name_end].decode("utf-8", errors="replace")
            match = WASM_TURBOFAN_NAME.fullmatch(name)
            if match is not None:
                index = int(match.group("index"))
                mangled = match.group("mangled")
                code = data[name_end + 1 : name_end + 1 + code_size]
                # Recompilations append later records; last one wins.
                by_index[index] = WasmFunctionCode(
                    index=index,
                    mangled=mangled,
                    demangled=demangle_rust(mangled),
                    code_addr=code_addr,
                    code=code,
                )
        offset += total_size
    return [by_index[index] for index in sorted(by_index)]


RUST_MANGLE_ESCAPES = {
    "$SP$": "@",
    "$BP$": "*",
    "$RF$": "&",
    "$LT$": "<",
    "$GT$": ">",
    "$LP$": "(",
    "$RP$": ")",
    "$C$": ",",
}


def demangle_rust(mangled: str) -> str:
    match = re.fullmatch(r"_ZN(.*)E", mangled)
    if match is None:
        return mangled
    rest = match.group(1)
    segments: list[str] = []
    position = 0
    while position < len(rest):
        length_match = re.match(r"[1-9]\d*", rest[position:])
        if length_match is None:
            return mangled
        length = int(length_match.group())
        position += len(length_match.group())
        segment = rest[position : position + length]
        if len(segment) != length:
            return mangled
        position += length
        segments.append(segment)
    if segments and re.fullmatch(r"h[0-9a-f]{16}", segments[-1]):
        segments.pop()
    decoded: list[str] = []
    for segment in segments:
        if segment.startswith("_$"):
            segment = segment[1:]
        for escape, replacement in RUST_MANGLE_ESCAPES.items():
            segment = segment.replace(escape, replacement)
        segment = re.sub(
            r"\$u([0-9a-f]{2,4})\$", lambda m: chr(int(m.group(1), 16)), segment
        )
        decoded.append(segment.replace("..", "::"))
    return "::".join(decoded)


def asm_file_name(fn: WasmFunctionCode) -> str:
    short = fn.demangled.rsplit("::", 1)[-1]
    short = re.sub(r"[^A-Za-z0-9_.-]+", "_", short)[:48] or "fn"
    return f"{fn.index:03d}-{short}.s"


def disassemble_wasm_function(
    objdump: str,
    objdump_arch: str,
    fn: WasmFunctionCode,
    work_dir: Path,
) -> str:
    bin_path = work_dir / f"disasm-{fn.index}.bin"
    bin_path.write_bytes(fn.code)
    completed = subprocess.run(
        [objdump, "-D", "-b", "binary", "-m", objdump_arch, str(bin_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"objdump failed for function index {fn.index}: {completed.stderr.strip()}"
        )
    lines = [
        line for line in completed.stdout.splitlines() if re.match(r"^ *[0-9a-f]+:", line)
    ]
    header = [
        f"; {fn.demangled}",
        f"; wasm function index {fn.index}, {len(fn.code)} bytes, code address 0x{fn.code_addr:x}",
        f"; record: JS:{fn.mangled}-{fn.index}-turbofan",
        "; trailing constant-pool data disassembles as garbage instructions",
    ]
    return "\n".join(header + lines) + "\n"


def json_stdout_result(
    completed: subprocess.CompletedProcess[str],
    json_label: str,
) -> dict[str, object]:
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        result = {
            "ok": False,
            "error": f"invalid {json_label} JSON: {error}",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
        if completed.returncode != 0:
            result["returncode"] = completed.returncode
        return result
    if not isinstance(report, dict):
        result = {
            "ok": False,
            "error": f"invalid {json_label} JSON: expected object",
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }
        if completed.returncode != 0:
            result["returncode"] = completed.returncode
        return result

    result = {
        "ok": completed.returncode == 0 and report.get("ok") is True,
        "report": report,
        "stderr": completed.stderr,
    }
    if completed.returncode != 0:
        result["returncode"] = completed.returncode
    return result


def timeout_result(
    timeout_kind: str,
    timeout_seconds: int,
    error: subprocess.TimeoutExpired,
) -> dict[str, object]:
    return {
        "ok": False,
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


def recv_record(sock: socket.socket, max_bytes: int, label: str) -> bytes:
    data, _ancillary, flags, _address = sock.recvmsg(max_bytes)
    if flags & getattr(socket, "MSG_TRUNC", 0):
        raise ValueError(f"{label} message exceeds {max_bytes} bytes")
    if data == b"":
        raise ValueError(f"missing {label} message")
    return data


def recv_record_with_fds(
    sock: socket.socket, max_bytes: int, label: str
) -> tuple[bytes, list[int]]:
    data, fds, flags, _address = socket.recv_fds(sock, max_bytes, 1)
    fds = list(fds)
    if flags & (
        getattr(socket, "MSG_TRUNC", 0) | getattr(socket, "MSG_CTRUNC", 0)
    ):
        close_fds(fds)
        raise ValueError(f"{label} message exceeds {max_bytes} bytes or 1 fd")
    if data == b"":
        close_fds(fds)
        raise ValueError(f"missing {label} message")
    return data, fds


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
    return b'{"type":"result","ok":false,"error":"response too large"}'


def sanitize_response_value(value: object) -> object:
    if isinstance(value, str):
        return truncate_response_text(value)
    if isinstance(value, dict):
        return {str(key): sanitize_response_value(child) for key, child in value.items()}
    if isinstance(value, list):
        return [sanitize_response_value(item) for item in value]
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
        "head": data[:RESPONSE_STRING_HEAD_BYTES].decode("utf-8", errors="replace"),
        "tail": data[-RESPONSE_STRING_TAIL_BYTES:].decode("utf-8", errors="replace"),
    }


def encode_json_record(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode("utf-8")


def send_record(sock: socket.socket, data: bytes) -> None:
    sent = sock.sendmsg([data])
    if sent != len(data):
        raise OSError(f"short seqpacket send: {sent}/{len(data)} bytes")


if __name__ == "__main__":
    raise SystemExit(main())
