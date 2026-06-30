#!/usr/bin/env python3
import argparse
import json
import re
import socket
import sys
from pathlib import Path, PurePosixPath

GRINDER_SOCKET_PATH = Path("/run/vip9r-perf.sock")
HOST_SOCKET_PATH = Path("temp/vip9r-perf.sock")
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
U32_MAX = 2**32 - 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="vip9r-perf-submit",
        description="Submit a vip9r performance run to the daemon.",
    )
    parser.add_argument(
        "candidate",
        type=Path,
        help="candidate wasm module",
    )
    parser.add_argument(
        "--slot",
        type=parse_u32,
        help="microbenchmark slot to run instead of a media decode",
    )
    parser.add_argument(
        "--media",
        type=parse_media_path,
        help="corpus-relative media path",
    )
    parser.add_argument(
        "--frames",
        type=parse_frame_range,
        help="output-frame selection as OFFSET:LAST",
    )
    parser.add_argument(
        "--pin",
        help="device CPU pin: any, all, cpu:N, or mask:HEX",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    if args.slot is not None:
        if args.media is not None or args.frames is not None:
            parser.error("--slot cannot be combined with --media or --frames")
    else:
        if args.media is None:
            parser.error("--media is required unless --slot is set")

    request: dict[str, object] = {}
    if args.slot is not None:
        request["kind"] = "microbench"
        request["slot"] = args.slot
    else:
        request["kind"] = "decode"
        request["media"] = str(args.media)
        if args.frames is not None:
            request["frames"] = {"offset": args.frames[0], "last": args.frames[1]}
    if args.pin is not None:
        request["pin"] = args.pin

    response = submit_request(request, args.candidate.read_bytes())
    print(json.dumps(response, indent=2))
    return 0 if response.get("ok") is True else 1


def submit_request(request: dict[str, object], candidate: bytes) -> dict[str, object]:
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, MAX_CANDIDATE_BYTES)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, MAX_RESPONSE_BYTES)
    sock.connect(str(default_socket_path()))
    with sock:
        send_json(sock, request)
        send_record(sock, candidate)
        while True:
            message = recv_json(sock)
            message_type = message.get("type")
            if message_type == "queued":
                print(f"queued: {message.get('ahead')} ahead", file=sys.stderr)
                continue
            if message_type == "started":
                print("started", file=sys.stderr)
                continue
            if message_type == "result":
                return message
            raise ValueError(f"unexpected response type: {message_type!r}")


def default_socket_path() -> Path:
    if GRINDER_SOCKET_PATH.exists():
        return GRINDER_SOCKET_PATH
    return HOST_SOCKET_PATH


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
        raise argparse.ArgumentTypeError("must be OFFSET:LAST")
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


def parse_u32(value: str) -> int:
    parsed = parse_non_negative_int(value)
    if parsed > U32_MAX:
        raise argparse.ArgumentTypeError("must fit in u32")
    return parsed


if __name__ == "__main__":
    raise SystemExit(main())
