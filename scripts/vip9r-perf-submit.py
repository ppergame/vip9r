#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path, PurePosixPath

REPO_ROOT = Path(__file__).resolve().parents[1]
GRINDER_SOCKET_PATH = Path("/run/vip9r-perf/vip9r-perf.sock")
HOST_SOCKET_PATH = REPO_ROOT / "temp/perf/vip9r-perf.sock"
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
U32_MAX = 2**32 - 1
TARGETS = ("host", "device")
CORPUS_ROOT = PurePosixPath("/bulk/vip9r")
DEFAULT_VALIDATION_MEDIA = PurePosixPath("chromium/bear-vp9.ivf")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="vip9r-perf-submit",
        description="Submit a vip9r run to the performance daemon.",
    )
    parser.add_argument(
        "--target",
        choices=TARGETS,
        default="host",
        help="run target: host or device",
    )
    parser.add_argument(
        "--device",
        type=parse_non_negative_int,
        help="daemon device index (default: VIP9R_PERF_DEVICE)",
    )
    parser.add_argument(
        "--pin",
        help="device CPU pin: any, all, cpu:N[,N-M,...], or mask:HEX"
        " (default: pin from VIP9R_PERF_DEVICE=INDEX:PIN)",
    )

    subparsers = parser.add_subparsers(dest="command", required=True)

    validate = subparsers.add_parser("validate", help="run md5 validation")
    validate.add_argument(
        "--media",
        type=parse_media_path,
        default=DEFAULT_VALIDATION_MEDIA,
        help="corpus-relative media path or /bulk/vip9r/... absolute path",
    )
    validate.add_argument(
        "--allow-mismatch",
        action="store_true",
        help="allow wrong frame md5 values",
    )
    validate.add_argument(
        "--frames",
        type=parse_frame_range,
        help="output-frame selection as START:LAST",
    )
    add_pool_argument(validate)

    bench = subparsers.add_parser("bench", help="run full-decode timing")
    bench.add_argument(
        "--media",
        required=True,
        type=parse_media_path,
        help="corpus-relative media path or /bulk/vip9r/... absolute path",
    )
    bench.add_argument(
        "--frames",
        type=parse_frame_range,
        help="output-frame selection as START:LAST",
    )
    add_pool_argument(bench)
    bench_baseline = bench.add_mutually_exclusive_group()
    bench_baseline.add_argument(
        "--baseline",
        type=Path,
        help="baseline wasm file (default: VIP9R_PERF_BASELINE)",
    )
    bench_baseline.add_argument(
        "--no-op-control",
        action="store_true",
        help="A/B the candidate against itself to measure harness noise",
    )

    tests = subparsers.add_parser("tests", help="run wasm unit tests")
    tests.add_argument(
        "test_filter",
        nargs="?",
        type=parse_non_empty_string,
        metavar="TEST_SUBSTRING",
        help="wasm unit test substring",
    )

    microbench = subparsers.add_parser(
        "microbench", help="run a wasm microbenchmark slot"
    )
    microbench.add_argument(
        "--slot",
        required=True,
        type=parse_u32,
        help="microbenchmark slot",
    )

    profile = subparsers.add_parser(
        "profile", help="record a simpleperf profile of a device bench run"
    )
    profile.add_argument(
        "--media",
        required=True,
        type=parse_media_path,
        help="corpus-relative media path or /bulk/vip9r/... absolute path",
    )
    profile.add_argument(
        "--frames",
        type=parse_frame_range,
        help="output-frame selection as START:LAST",
    )
    profile.add_argument(
        "--freq",
        type=parse_freq,
        default=1000,
        help="simpleperf sample frequency in Hz (default 1000)",
    )
    add_pool_argument(profile)

    asm = subparsers.add_parser(
        "asm", help="dump native asm of every declared wasm function"
    )
    asm.add_argument(
        "--arch",
        choices=("arm32", "arm64", "host"),
        required=True,
        help="native target architecture",
    )
    return parser


def add_pool_argument(subparser: argparse.ArgumentParser) -> None:
    subparser.add_argument(
        "--pool",
        action=argparse.BooleanOptionalAction,
        help="spawn the worker pool and activate tile-parallel decode"
        " (default: VIP9R_PERF_POOL)",
    )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.target == "host":
        if args.pin is not None:
            parser.error("--pin requires --target device")
        if args.device is not None:
            parser.error("--device requires --target device")
    if args.command == "asm" and args.target != "host":
        parser.error("asm runs on the daemon host; pick the ISA with --arch")
    if args.command == "profile" and args.target != "device":
        parser.error("profile requires --target device")

    if args.command in ("validate", "bench", "profile"):
        try:
            pool = args.pool if args.pool is not None else pool_env_default()
        except RuntimeError as error:
            print(f"submit: {error}", file=sys.stderr)
            return 2

    request: dict[str, object] = {"target": args.target}
    if args.target == "device":
        try:
            env_device, env_pin = device_env_default()
        except RuntimeError as error:
            print(f"submit: {error}", file=sys.stderr)
            return 2
        device_index = args.device if args.device is not None else env_device
        if device_index is None:
            parser.error("--target device requires --device N (or VIP9R_PERF_DEVICE)")
        request["device"] = device_index
        pin = args.pin if args.pin is not None else env_pin
        if pin is not None:
            request["pin"] = pin
        elif args.command in ("bench", "microbench", "profile"):
            parser.error(
                f"{args.command} on device requires --pin (or VIP9R_PERF_DEVICE=INDEX:PIN)"
            )
    if args.command == "asm":
        request["kind"] = "asm"
        request["arch"] = args.arch
        tests = False
    elif args.command == "profile":
        request["kind"] = "profile"
        request["media"] = str(args.media)
        if args.frames is not None:
            request["frames"] = {"offset": args.frames[0], "last": args.frames[1]}
        request["freq"] = args.freq
        if pool:
            request["pool"] = True
        tests = False
    elif args.command == "microbench":
        request["kind"] = "microbench"
        request["slot"] = args.slot
        tests = False
    elif args.command == "tests":
        request["kind"] = "tests"
        if args.test_filter is not None:
            request["filter"] = args.test_filter
        tests = True
    elif args.command == "validate":
        request["kind"] = "validate"
        request["media"] = str(args.media)
        if args.allow_mismatch:
            request["allow_mismatch"] = True
        if args.frames is not None:
            request["frames"] = {"offset": args.frames[0], "last": args.frames[1]}
        if pool:
            request["pool"] = True
        tests = False
    elif args.command == "bench":
        request["kind"] = "bench"
        request["media"] = str(args.media)
        if args.frames is not None:
            request["frames"] = {"offset": args.frames[0], "last": args.frames[1]}
        if pool:
            request["pool"] = True
        tests = False
    else:
        raise AssertionError(f"unknown command: {args.command!r}")

    socket_path = default_socket_path()
    if socket_path is None:
        print(f"submit: no perf daemon socket", file=sys.stderr)
        return 2

    try:
        candidate = build_candidate(tests).read_bytes()
    except Exception as error:
        print(f"candidate: {error}", file=sys.stderr)
        return 2

    baseline: bytes | None = None
    if args.command == "bench":
        if args.no_op_control:
            # An A/B run of identical wasm measures the harness, not a
            # change; only run one on purpose.
            baseline = candidate
        else:
            baseline_path = args.baseline
            if baseline_path is None:
                env_baseline = os.environ.get("VIP9R_PERF_BASELINE")
                if env_baseline:
                    baseline_path = Path(env_baseline)
            if baseline_path is None:
                parser.error(
                    "bench requires --baseline WASM (or VIP9R_PERF_BASELINE),"
                    " or --no-op-control"
                )
            try:
                baseline = baseline_path.read_bytes()
            except OSError as error:
                print(f"baseline: {error}", file=sys.stderr)
                return 2

    if args.command == "asm":
        file_dir = asm_output_dir(socket_path, candidate, args.arch)
    elif args.command == "profile":
        file_dir = profile_output_dir(socket_path, candidate)
    else:
        file_dir = None
    try:
        response = submit_request(request, candidate, socket_path, file_dir, baseline)
    except RuntimeError as error:
        print(f"submit: {error}", file=sys.stderr)
        return 2
    if file_dir is not None and response.get("ok") is True:
        response["dir"] = str(file_dir)
    print(json.dumps(response, indent=2))
    return 0 if response.get("ok") is True else 1


def asm_output_dir(socket_path: Path, candidate: bytes, arch: str) -> Path:
    name = f"{hashlib.sha256(candidate).hexdigest()[:8]}-{arch}"
    if socket_path == GRINDER_SOCKET_PATH:
        return Path("/tmp/vip9r-asm") / name
    return REPO_ROOT / "temp/vip9r-asm" / name


def profile_output_dir(socket_path: Path, candidate: bytes) -> Path:
    # Profile output is not deterministic per candidate; timestamp the
    # directory so repeated runs do not overwrite each other.
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    name = f"{hashlib.sha256(candidate).hexdigest()[:8]}-{stamp}"
    if socket_path == GRINDER_SOCKET_PATH:
        return Path("/tmp/vip9r-profiles") / name
    return REPO_ROOT / "temp/vip9r-profiles" / name


def build_candidate(tests: bool) -> Path:
    return build_wasm_tests_candidate() if tests else build_release_candidate()


def build_release_candidate() -> Path:
    rust_root = locate_rust_workspace()
    wasm_path = rust_root / "target/wasm32-unknown-unknown/release/vip9r.wasm"
    cmd = [
        "cargo",
        "build",
        "--manifest-path",
        str(rust_root / "Cargo.toml"),
        "--target",
        "wasm32-unknown-unknown",
        "-p",
        "vip9r",
        "--release",
    ]
    # cwd must be the workspace: cargo resolves .cargo/config.toml (which
    # carries target-feature flags) from cwd, not --manifest-path.
    completed = subprocess.run(cmd, check=False, cwd=rust_root, env=cargo_env())
    if completed.returncode != 0:
        raise RuntimeError(f"release wasm build exited {completed.returncode}")
    return wasm_path


def build_wasm_tests_candidate() -> Path:
    rust_root = locate_rust_workspace()
    target_dir = rust_root / "target/wasm-tests"
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
        "--features",
        "wasm-tests",
    ]
    completed = subprocess.run(cmd, check=False, cwd=rust_root, env=cargo_env())
    if completed.returncode != 0:
        raise RuntimeError(f"wasm-tests build exited {completed.returncode}")
    return wasm_path


def cargo_env() -> dict[str, str]:
    # Threaded-wasm build: stable cargo honors the [unstable] build-std table
    # in .cargo/config.toml only with this in its environment.
    return {**os.environ, "RUSTC_BOOTSTRAP": "1"}


def locate_rust_workspace() -> Path:
    root = git_root()
    if (root / "rust/Cargo.toml").is_file():
        return root / "rust"
    if (root / "Cargo.toml").is_file():
        return root
    raise RuntimeError(f"could not find vip9r Cargo workspace from {root}")


def git_root() -> Path:
    completed = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError("could not find git checkout")
    return Path(completed.stdout.strip())


def pool_env_default() -> bool:
    value = os.environ.get("VIP9R_PERF_POOL")
    if value is None or value == "":
        return False
    if value == "1":
        return True
    raise RuntimeError(f"VIP9R_PERF_POOL must be 1 or unset: {value!r}")


def device_env_default() -> tuple[int | None, str | None]:
    value = os.environ.get("VIP9R_PERF_DEVICE")
    if value is None or value == "":
        return None, None
    index_text, _, pin = value.partition(":")
    if not re.fullmatch(r"0|[1-9]\d*", index_text):
        raise RuntimeError(f"VIP9R_PERF_DEVICE must be INDEX[:PIN]: {value!r}")
    return int(index_text), pin or None


def submit_request(
    request: dict[str, object],
    candidate: bytes,
    socket_path: Path,
    file_dir: Path | None = None,
    baseline: bytes | None = None,
) -> dict[str, object]:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_CANDIDATE_BYTES)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_RESPONSE_BYTES)
    try:
        sock.connect(str(socket_path))
    except OSError as error:
        sock.close()
        raise RuntimeError(
            f"connect {socket_path}: {error.strerror or error}"
        ) from error
    with sock:
        if file_dir is None:
            send_json(sock, request)
        else:
            send_json_with_dir_fd(sock, request, file_dir)
        send_record(sock, candidate)
        if baseline is not None:
            send_record(sock, baseline)
        while True:
            message = recv_json(sock)
            message_type = message.get("type")
            if message_type == "queued":
                target = message.get("target")
                prefix = f"queued {target}" if isinstance(target, str) else "queued"
                print(f"{prefix}: {message.get('ahead')} ahead", file=sys.stderr)
                continue
            if message_type == "started":
                target = message.get("target")
                suffix = f" {target}" if isinstance(target, str) else ""
                print(f"started{suffix}", file=sys.stderr)
                continue
            if message_type == "result":
                return message
            raise ValueError(f"unexpected response type: {message_type!r}")


def send_json_with_dir_fd(
    sock: socket.socket, request: dict[str, object], file_dir: Path
) -> None:
    # The daemon writes result files straight into this directory via the
    # passed fd; that works across the grinder sandbox's mount namespace.
    file_dir.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(request, separators=(",", ":")).encode("utf-8")
    dir_fd = os.open(file_dir, os.O_RDONLY | os.O_DIRECTORY)
    try:
        sent = socket.send_fds(sock, [payload], [dir_fd])
        if sent != len(payload):
            raise OSError(f"short seqpacket send: {sent}/{len(payload)} bytes")
    finally:
        os.close(dir_fd)


def default_socket_path() -> Path | None:
    if GRINDER_SOCKET_PATH.is_socket():
        return GRINDER_SOCKET_PATH
    if HOST_SOCKET_PATH.is_socket():
        return HOST_SOCKET_PATH
    return None


def send_json(sock: socket.socket, value: object) -> None:
    send_record(sock, json.dumps(value, separators=(",", ":")).encode("utf-8"))


def send_record(sock: socket.socket, data: bytes) -> None:
    sent = sock.sendmsg([data])
    if sent != len(data):
        raise OSError(f"short seqpacket send: {sent}/{len(data)} bytes")


def recv_json(sock: socket.socket) -> dict[str, object]:
    data, _ancillary, flags, _address = sock.recvmsg(MAX_RESPONSE_BYTES)
    if flags & getattr(socket, "MSG_TRUNC", 0):
        raise ValueError(f"response message exceeds {MAX_RESPONSE_BYTES} bytes")
    if data == b"":
        raise ValueError("missing response message")
    value = json.loads(data.decode("utf-8"))
    if not isinstance(value, dict):
        raise ValueError("response is not a JSON object")
    return value


def parse_frame_range(value: str) -> tuple[int, int]:
    match = re.fullmatch(r"(\d+):(\d+)", value)
    if match is None:
        raise argparse.ArgumentTypeError("must be START:LAST")
    offset = int(match.group(1))
    last = int(match.group(2))
    if last < offset:
        raise argparse.ArgumentTypeError("last must be greater than or equal to offset")
    return offset, last


def parse_media_path(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if ".." in path.parts:
        raise argparse.ArgumentTypeError(
            f"must be corpus-relative or under {CORPUS_ROOT}, without '..'"
        )
    if path.is_absolute():
        try:
            path = path.relative_to(CORPUS_ROOT)
        except ValueError as error:
            raise argparse.ArgumentTypeError(
                f"absolute media paths must be under {CORPUS_ROOT}"
            ) from error
    if path == PurePosixPath(".") or path.is_absolute():
        raise argparse.ArgumentTypeError(
            f"must be corpus-relative or under {CORPUS_ROOT}, without '..'"
        )
    return path


def parse_non_negative_int(value: str) -> int:
    if not re.fullmatch(r"0|[1-9]\d*", value):
        raise argparse.ArgumentTypeError("must be a non-negative integer")
    return int(value)


def parse_non_empty_string(value: str) -> str:
    if value == "":
        raise argparse.ArgumentTypeError("must be non-empty")
    return value


def parse_freq(value: str) -> int:
    parsed = parse_non_negative_int(value)
    if not 1 <= parsed <= 100000:
        raise argparse.ArgumentTypeError("must be in 1..100000")
    return parsed


def parse_u32(value: str) -> int:
    parsed = parse_non_negative_int(value)
    if parsed > U32_MAX:
        raise argparse.ArgumentTypeError("must fit in u32")
    return parsed


if __name__ == "__main__":
    raise SystemExit(main())
