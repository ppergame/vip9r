#!/usr/bin/env python3
import argparse
import json
import re
import socket
import subprocess
import sys
from pathlib import Path, PurePosixPath

REPO_ROOT = Path(__file__).resolve().parents[1]
GRINDER_SOCKET_PATH = Path("/run/vip9r-perf.sock")
HOST_SOCKET_PATH = REPO_ROOT / "temp/vip9r-perf.sock"
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
U32_MAX = 2**32 - 1
TARGETS = ("host", "device")
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
        "--pin",
        help="device CPU pin: any, all, cpu:N, or mask:HEX",
    )

    subparsers = parser.add_subparsers(dest="command", required=True)

    validate = subparsers.add_parser("validate", help="run md5 validation")
    validate.add_argument(
        "--media",
        type=parse_media_path,
        default=DEFAULT_VALIDATION_MEDIA,
        help="corpus-relative media path",
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

    bench = subparsers.add_parser("bench", help="run full-decode timing")
    bench.add_argument(
        "--media",
        required=True,
        type=parse_media_path,
        help="corpus-relative media path",
    )
    bench.add_argument(
        "--frames",
        type=parse_frame_range,
        help="output-frame selection as START:LAST",
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
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.target == "host" and args.pin is not None:
        parser.error("--pin requires --target device")

    request: dict[str, object] = {"target": args.target}
    if args.command == "microbench":
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
        tests = False
    elif args.command == "bench":
        request["kind"] = "bench"
        request["media"] = str(args.media)
        if args.frames is not None:
            request["frames"] = {"offset": args.frames[0], "last": args.frames[1]}
        tests = False
    else:
        raise AssertionError(f"unknown command: {args.command!r}")
    if args.pin is not None:
        request["pin"] = args.pin

    socket_path = default_socket_path()
    if socket_path is None:
        print(f"submit: no perf daemon socket", file=sys.stderr)
        return 2

    try:
        candidate = build_candidate(tests).read_bytes()
    except Exception as error:
        print(f"candidate: {error}", file=sys.stderr)
        return 2

    try:
        response = submit_request(request, candidate, socket_path)
    except RuntimeError as error:
        print(f"submit: {error}", file=sys.stderr)
        return 2
    print(json.dumps(response, indent=2))
    return 0 if response.get("ok") is True else 1


def build_candidate(tests: bool) -> Path:
    return build_wasm_tests_candidate() if tests else build_release_candidate()


def build_release_candidate() -> Path:
    rust_root = locate_rust_workspace()
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
    completed = subprocess.run(cmd, check=False)
    if completed.returncode != 0:
        raise RuntimeError(f"wasm-tests build exited {completed.returncode}")
    return wasm_path


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


def submit_request(
    request: dict[str, object], candidate: bytes, socket_path: Path
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
        send_json(sock, request)
        send_record(sock, candidate)
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
    if path.is_absolute() or path == PurePosixPath(".") or ".." in path.parts:
        raise argparse.ArgumentTypeError("must be a corpus-relative path without '..'")
    return path


def parse_non_negative_int(value: str) -> int:
    if not re.fullmatch(r"0|[1-9]\d*", value):
        raise argparse.ArgumentTypeError("must be a non-negative integer")
    return int(value)


def parse_non_empty_string(value: str) -> str:
    if value == "":
        raise argparse.ArgumentTypeError("must be non-empty")
    return value


def parse_u32(value: str) -> int:
    parsed = parse_non_negative_int(value)
    if parsed > U32_MAX:
        raise argparse.ArgumentTypeError("must fit in u32")
    return parsed


if __name__ == "__main__":
    raise SystemExit(main())
