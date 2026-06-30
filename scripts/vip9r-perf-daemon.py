#!/usr/bin/env python3
import argparse
from collections import deque
from dataclasses import dataclass
import errno
import json
import os
import socket
import subprocess
import sys
import threading
from pathlib import Path

SOCKET_PATH = Path("temp/vip9r-perf.sock")
REPO_ROOT = Path(__file__).resolve().parents[1]
GOLDEN_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/golden.js"
MICROBENCH_RUNNER_PATH = REPO_ROOT / "js/dist/wasm-driver/microbench.js"
MEDIA_ROOT = Path("/bulk/vip9r")
MAX_REQUEST_BYTES = 64 * 1024
MAX_CANDIDATE_BYTES = 64 * 1024 * 1024
MAX_RESPONSE_BYTES = 1024 * 1024
U32_MAX = 2**32 - 1


@dataclass
class Job:
    conn: socket.socket
    request: object
    candidate: bytes


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
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    baseline_fd = memfd_from_bytes("vip9r-baseline.wasm", args.baseline.read_bytes())

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
    worker = threading.Thread(target=run_worker, args=(args, jobs, baseline_fd), daemon=True)
    worker.start()

    try:
        server.listen()
        print(
            json.dumps(
                {
                    "baseline": str(args.baseline),
                    "serial": args.serial,
                    "socket": str(SOCKET_PATH),
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


def run_worker(args: argparse.Namespace, jobs: JobQueue, baseline_fd: int) -> None:
    while True:
        job = jobs.pop()
        try:
            send_json(job.conn, {"type": "started"})
            response = handle_request(args, baseline_fd, job.request, job.candidate)
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
    request: object,
    candidate: bytes,
) -> dict[str, object]:
    if not isinstance(request, dict):
        return {"ok": False, "error": "request must be a JSON object"}

    kind = request.get("kind")
    if kind == "microbench":
        return handle_microbench_request(args, request, candidate)
    if kind != "decode":
        return {"ok": False, "error": f"unsupported request kind: {kind!r}"}

    try:
        media_path = media_path_from_request(request)
        frame_range = frame_range_from_request(request)
    except ValueError as error:
        return {"ok": False, "error": str(error)}

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
    args: argparse.Namespace,
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
        "serial": args.serial,
        "host": {
            "candidate": candidate_result,
        },
    }


def media_path_from_request(request: dict[str, object]) -> Path:
    media = request.get("media")
    if not isinstance(media, str) or media == "":
        raise ValueError("decode request requires media")
    path = Path(media)
    if path.is_absolute():
        return path
    return MEDIA_ROOT / path


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
