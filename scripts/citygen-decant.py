#!/usr/bin/env python3
import base64
import hashlib
import http.client
import json
import math
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlencode, urlparse

SCRIPT_PATH = Path(__file__).resolve()
REPO_ROOT = SCRIPT_PATH.parent.parent
JS_DIR = REPO_ROOT / "js"

SCENE_KEYS = {
    "seed",
    "speed",
    "altitude",
    "pitch",
    "fov",
    "sway",
    "density",
    "towers",
    "lit",
    "warmth",
    "fog",
    "glow",
    "streets",
    "traffic",
    "neon",
    "grain",
    "exposure",
}
ENCODE_KEYS = {"duration", "mbps", "kf"}
PAGE_KEYS = SCENE_KEYS | ENCODE_KEYS | {"samples"}
DEFAULT_QUERY = {
    "duration": "60",
    "mbps": "4",
    "kf": "5",
    "samples": "4",
}
CHUNK_BYTES = 1 << 20


def usage() -> str:
    return """Usage:
  scripts/citygen-decant.py [options] [-- chrome flags...]

Decants js/citygen-decant.html through headless Chromium and writes a VP9 WebM.
The default Chrome mode is host NVIDIA via ANGLE GL/EGL.

Options:
  -o, --output PATH             output .webm path; default is page-generated name
      --chrome PATH             Chrome/Chromium executable; default: CHROME or PATH lookup
      --chrome-mode MODE        nvidia | swiftshader | default; default: nvidia
      --profile DIR             Chrome user-data-dir; default: temp/citygen-chrome-profile
      --vite-port PORT          Vite port; default: auto
      --timeout-ms MS           fail if decant is not done in time; 0 disables; default: 900000
      --hash HASH               citygen presentation hash, e.g. '#seed=7&speed=55'
      --hardware-acceleration V no-preference | prefer-hardware | prefer-software; default: prefer-software
      --duration SEC            output duration; default: 60
      --mbps N                  target VideoEncoder bitrate in Mbps; default: 4
      --kf SEC                  keyframe interval; default: 5
      --samples N               shader samples per pixel; default: 4
      --seed N                  any citygen scene param also works as --key value or --key=value
  -h, --help                    print this help

Examples:
  scripts/citygen-decant.py --duration 60 --mbps 4 --seed 3 -o city.webm
  CHROME=/opt/chrome/chrome scripts/citygen-decant.py --hash '#seed=3&fog=0.4'
  scripts/citygen-decant.py --chrome-mode swiftshader --duration 1 --timeout-ms 60000
"""


def parse_args(argv: list[str]) -> dict:
    options = {
        "chrome": os.environ.get("CHROME", ""),
        "chrome_mode": "nvidia",
        "output": "",
        "profile": REPO_ROOT / "temp" / "citygen-chrome-profile",
        "timeout_ms": 900_000,
        "vite_port": 0,
        "hash": "",
        "hardware_acceleration": "prefer-software",
        "query": dict(DEFAULT_QUERY),
        "extra_chrome_flags": [],
    }

    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg == "--":
            options["extra_chrome_flags"] = argv[index + 1 :]
            break
        if arg in ("-h", "--help"):
            sys.stdout.write(usage())
            raise SystemExit(0)

        value = None
        if arg == "-o":
            name = "output"
        elif arg.startswith("--") and "=" in arg:
            name, value = arg[2:].split("=", 1)
        elif arg.startswith("--"):
            name = arg[2:]
        else:
            raise ValueError(f"unknown argument: {arg}")

        if value is None:
            index += 1
            if index >= len(argv):
                raise ValueError(f"{arg} needs a value")
            value = argv[index]

        if name == "chrome":
            options["chrome"] = value
        elif name == "chrome-mode":
            if value not in {"nvidia", "swiftshader", "default"}:
                raise ValueError(f"invalid --chrome-mode: {value}")
            options["chrome_mode"] = value
        elif name == "output":
            options["output"] = value
        elif name == "profile":
            options["profile"] = Path(value).resolve()
        elif name == "vite-port":
            options["vite_port"] = parse_positive_int(value, "vite-port")
        elif name == "timeout-ms":
            options["timeout_ms"] = parse_non_negative_int(value, "timeout-ms")
        elif name == "hash":
            options["hash"] = value[1:] if value.startswith("#") else value
        elif name == "hardware-acceleration":
            if value not in {"no-preference", "prefer-hardware", "prefer-software"}:
                raise ValueError(f"invalid --hardware-acceleration: {value}")
            options["hardware_acceleration"] = value
        elif name in PAGE_KEYS:
            assert_finite_number(value, name)
            options["query"][name] = value
        else:
            raise ValueError(f"unknown option: --{name}")

        index += 1

    return options


def parse_positive_int(raw: str, name: str) -> int:
    value = int(raw)
    if value <= 0:
        raise ValueError(f"--{name} must be a positive integer")
    return value


def parse_non_negative_int(raw: str, name: str) -> int:
    value = int(raw)
    if value < 0:
        raise ValueError(f"--{name} must be a non-negative integer")
    return value


def assert_finite_number(raw: str, name: str) -> None:
    value = float(raw)
    if not math.isfinite(value):
        raise ValueError(f"--{name} must be a finite number")


def main() -> None:
    options = parse_args(sys.argv[1:])
    chrome = find_chrome(options["chrome"])
    vite_port = options["vite_port"] or pick_port()
    page_url = build_page_url(vite_port, options)

    Path(options["profile"]).mkdir(parents=True, exist_ok=True)
    vite = start_vite(vite_port)
    chrome_process = start_chrome(chrome, options)
    cdp = None
    try:
        wait_for_http(f"127.0.0.1:{vite_port}", vite.proc, 30_000)
        browser_ws_url = chrome_process.wait_for_devtools(30_000)
        cdp = Cdp.connect(browser_ws_url)

        target = cdp.send("Target.createTarget", {"url": page_url})
        attached = cdp.send(
            "Target.attachToTarget",
            {"targetId": target["targetId"], "flatten": True},
        )
        session_id = attached["sessionId"]
        cdp.send("Runtime.enable", session_id=session_id)

        sys.stderr.write(f"citygen: {page_url}\n")
        sys.stderr.write(f"chrome: {chrome} ({options['chrome_mode']})\n")

        state = wait_for_decant(cdp, session_id, options["timeout_ms"])
        if state.get("status") != "done" or not state.get("ok"):
            raise RuntimeError(state.get("error") or f"decant failed: {state}")
        result = state.get("result")
        if not result:
            raise RuntimeError("decant finished without result metadata")

        output = Path(options["output"] or result["name"]).resolve()
        pull_bytes(cdp, session_id, output, result["bytes"])
        sys.stderr.write(
            f"wrote {output} ({result['bytes'] / 1_000_000:.1f} MB, "
            f"{result['packets']} packets, {result['mbps']:.2f} Mbps)\n"
        )
        webgl = state.get("webgl")
        if webgl:
            renderer = webgl.get("unmaskedRenderer") or webgl.get("renderer")
            sys.stderr.write(f"webgl: {renderer}\n")
    finally:
        if cdp is not None:
            try:
                cdp.send("Browser.close")
            except Exception:
                pass
            cdp.close()
        else:
            terminate(chrome_process.proc)
        wait_then_kill(chrome_process.proc, 3)
        terminate(vite.proc)
        wait_then_kill(vite.proc, 3)


def build_page_url(port: int, options: dict) -> str:
    query = dict(options["query"])
    query["hardwareAcceleration"] = options["hardware_acceleration"]
    url = f"http://127.0.0.1:{port}/citygen-decant.html?{urlencode(query)}"
    if options["hash"]:
        url += "#" + options["hash"]
    return url


def find_chrome(explicit: str) -> str:
    candidates = [
        explicit,
        "google-chrome-stable",
        "google-chrome",
        "chromium",
        "chromium-browser",
    ]
    for candidate in candidates:
        if not candidate:
            continue
        if "/" in candidate or candidate.startswith("."):
            path = Path(candidate).resolve()
            if path.exists():
                return str(path)
        else:
            resolved = shutil.which(candidate)
            if resolved:
                return resolved
    raise RuntimeError("Chrome not found; set CHROME=/path/to/chrome")


def pick_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


class ProcessCapture:
    def __init__(self, proc: subprocess.Popen[str], name: str):
        self.proc = proc
        self.name = name
        self._tail = ""
        self._lock = threading.Lock()
        for pipe in (proc.stdout, proc.stderr):
            if pipe is not None:
                threading.Thread(
                    target=self._drain,
                    args=(pipe,),
                    daemon=True,
                ).start()

    def _drain(self, pipe) -> None:
        for chunk in pipe:
            with self._lock:
                self._tail = (self._tail + chunk)[-8000:]

    def tail(self) -> str:
        with self._lock:
            return self._tail


class ChromeProcess(ProcessCapture):
    def __init__(self, proc: subprocess.Popen[str], name: str):
        self._devtools_event = threading.Event()
        self._devtools_url = None
        super().__init__(proc, name)

    def _drain(self, pipe) -> None:
        for chunk in pipe:
            with self._lock:
                self._tail = (self._tail + chunk)[-8000:]
            match = re.search(r"DevTools listening on (ws://\S+)", chunk)
            if match:
                self._devtools_url = match.group(1)
                self._devtools_event.set()

    def wait_for_devtools(self, timeout_ms: int) -> str:
        deadline = time.monotonic() + timeout_ms / 1000
        while time.monotonic() < deadline:
            if self._devtools_event.wait(0.05):
                assert self._devtools_url is not None
                return self._devtools_url
            if self.proc.poll() is not None:
                raise RuntimeError(
                    "Chrome exited before DevTools: "
                    f"code={self.proc.returncode}\n{self.tail()}"
                )
        raise TimeoutError("Chrome did not open DevTools\n" + self.tail())


def start_vite(port: int) -> ProcessCapture:
    env = dict(os.environ)
    env["BROWSER"] = "none"
    proc = subprocess.Popen(
        [
            "pnpm",
            "exec",
            "vite",
            "--host",
            "127.0.0.1",
            "--port",
            str(port),
            "--strictPort",
            "--clearScreen",
            "false",
        ],
        cwd=JS_DIR,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return ProcessCapture(proc, "vite")


def start_chrome(chrome: str, options: dict) -> ChromeProcess:
    env = dict(os.environ)
    args = [
        "--headless=new",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-domain-reliability",
        "--disable-extensions",
        "--disable-sync",
        "--metrics-recording-only",
        "--no-pings",
        "--password-store=basic",
        "--safebrowsing-disable-auto-update",
        "--host-resolver-rules=MAP * 0.0.0.0,EXCLUDE localhost,EXCLUDE 127.0.0.1",
        "--window-size=1280,720",
        f"--user-data-dir={options['profile']}",
        *chrome_mode_flags(options["chrome_mode"], env),
        *options["extra_chrome_flags"],
        "about:blank",
    ]
    proc = subprocess.Popen(
        [chrome, *args],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    return ChromeProcess(proc, "chrome")


def chrome_mode_flags(mode: str, env: dict) -> list[str]:
    if mode == "nvidia":
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
        env.setdefault("__NV_PRIME_RENDER_OFFLOAD", "1")
        env.setdefault("__GLX_VENDOR_LIBRARY_NAME", "nvidia")
        env.setdefault(
            "__EGL_VENDOR_LIBRARY_FILENAMES",
            "/run/opengl-driver/share/glvnd/egl_vendor.d/10_nvidia.json",
        )
        return [
            "--ozone-platform=wayland",
            "--use-gl=angle",
            "--use-angle=gl-egl",
            "--ignore-gpu-blocklist",
            "--enable-gpu",
            "--disable-software-rasterizer",
            "--disable-features=Vulkan,DefaultANGLEVulkan,VulkanFromANGLE,WaylandWpColorManagerV1",
        ]
    if mode == "swiftshader":
        return [
            "--use-gl=angle",
            "--use-angle=swiftshader",
            "--enable-unsafe-swiftshader",
            "--ignore-gpu-blocklist",
            "--enable-gpu",
        ]
    if mode == "default":
        return []
    raise ValueError(f"unknown Chrome mode: {mode}")


def wait_for_http(host_port: str, proc: subprocess.Popen[str], timeout_ms: int) -> None:
    deadline = time.monotonic() + timeout_ms / 1000
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"Vite exited with code {proc.returncode}")
        try:
            conn = http.client.HTTPConnection(host_port, timeout=1)
            conn.request("GET", "/")
            response = conn.getresponse()
            response.read()
            conn.close()
            return
        except OSError:
            time.sleep(0.25)
    raise TimeoutError(f"Vite did not respond at http://{host_port}")


def wait_for_decant(cdp: "Cdp", session_id: str, timeout_ms: int) -> dict:
    deadline = math.inf if timeout_ms == 0 else time.monotonic() + timeout_ms / 1000
    last_frame = 0
    while time.monotonic() < deadline:
        state = read_state(cdp, session_id)
        if state and state.get("status") in {"done", "error"}:
            sys.stderr.write("\n")
            return state
        progress = state.get("progress") if state else None
        if progress and progress.get("frame") != last_frame:
            last_frame = progress["frame"]
            sys.stderr.write(
                f"\rdecant {progress['frame']}/{progress['totalFrames']} "
                f"{progress['fps']:.1f} fps eta {progress['etaS']:.0f}s"
            )
            sys.stderr.flush()
        time.sleep(0.5)
    raise TimeoutError("decant timeout")


def read_state(cdp: "Cdp", session_id: str):
    response = cdp.send(
        "Runtime.evaluate",
        {
            "expression": "window.__citygenDecantState ?? null",
            "returnByValue": True,
        },
        session_id=session_id,
    )
    if response.get("exceptionDetails"):
        raise RuntimeError("reading decant state failed")
    return response["result"].get("value")


def pull_bytes(cdp: "Cdp", session_id: str, output: Path, byte_length: int) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("xb") as file:
        for offset in range(0, byte_length, CHUNK_BYTES):
            size = min(CHUNK_BYTES, byte_length - offset)
            response = cdp.send(
                "Runtime.evaluate",
                {
                    "expression": f"""(() => {{
            const bytes = window.__citygenDecantBytes;
            if (!bytes) throw new Error("decant bytes are missing");
            const slice = bytes.subarray({offset}, {offset + size});
            let binary = "";
            for (let i = 0; i < slice.length; i += 0x8000) {{
              binary += String.fromCharCode(...slice.subarray(i, i + 0x8000));
            }}
            return btoa(binary);
          }})()""",
                    "returnByValue": True,
                },
                session_id=session_id,
            )
            if response.get("exceptionDetails"):
                raise RuntimeError("pulling decant bytes failed")
            file.write(base64.b64decode(response["result"]["value"]))
            sys.stderr.write(f"\rwrite {min(offset + size, byte_length)}/{byte_length}")
            sys.stderr.flush()
        sys.stderr.write("\n")


class Cdp:
    def __init__(self, ws: "WebSocket"):
        self.ws = ws
        self.next_id = 1

    @classmethod
    def connect(cls, url: str) -> "Cdp":
        return cls(WebSocket.connect(url))

    def send(self, method: str, params=None, session_id: str | None = None) -> dict:
        message_id = self.next_id
        self.next_id += 1
        message = {
            "id": message_id,
            "method": method,
            "params": params or {},
        }
        if session_id is not None:
            message["sessionId"] = session_id
        self.ws.send_text(json.dumps(message, separators=(",", ":")))
        while True:
            response = json.loads(self.ws.recv_text())
            if response.get("id") != message_id:
                continue
            if "error" in response:
                error = response["error"]
                raise RuntimeError(
                    f"{error.get('message', 'CDP error')}: {error.get('data', '')}"
                )
            return response.get("result", {})

    def close(self) -> None:
        self.ws.close()


class WebSocket:
    def __init__(self, sock: socket.socket):
        self.sock = sock

    @classmethod
    def connect(cls, url: str) -> "WebSocket":
        parsed = urlparse(url)
        if parsed.scheme != "ws":
            raise ValueError(f"only ws:// DevTools URLs are supported: {url}")
        host = parsed.hostname
        if host is None:
            raise ValueError(f"DevTools URL has no host: {url}")
        port = parsed.port or 80
        path = parsed.path or "/"
        if parsed.query:
            path += "?" + parsed.query

        sock = socket.create_connection((host, port), timeout=30)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "\r\n"
        )
        sock.sendall(request.encode("ascii"))
        response = read_http_header(sock)
        status_line, *header_lines = response.split("\r\n")
        if " 101 " not in status_line:
            raise RuntimeError(f"WebSocket upgrade failed: {status_line}")
        headers = {}
        for line in header_lines:
            if ":" in line:
                name, value = line.split(":", 1)
                headers[name.lower()] = value.strip()
        accept = headers.get("sec-websocket-accept")
        expected = base64.b64encode(
            hashlib.sha1(
                (key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode("ascii")
            ).digest()
        ).decode("ascii")
        if accept != expected:
            raise RuntimeError("WebSocket accept header mismatch")
        return cls(sock)

    def send_text(self, text: str) -> None:
        self._send_frame(0x1, text.encode("utf-8"))

    def send_pong(self, payload: bytes) -> None:
        self._send_frame(0xA, payload)

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        header = bytearray([0x80 | opcode])
        length = len(payload)
        if length < 126:
            header.append(0x80 | length)
        elif length < 65536:
            header.append(0x80 | 126)
            header.extend(struct.pack("!H", length))
        else:
            header.append(0x80 | 127)
            header.extend(struct.pack("!Q", length))
        mask = os.urandom(4)
        header.extend(mask)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(header + masked)

    def recv_text(self) -> str:
        fragments = []
        while True:
            fin, opcode, payload = self._recv_frame()
            if opcode == 0x8:
                raise RuntimeError("WebSocket closed")
            if opcode == 0x9:
                self.send_pong(payload)
                continue
            if opcode == 0xA:
                continue
            if opcode not in (0x0, 0x1):
                continue
            fragments.append(payload)
            if fin:
                return b"".join(fragments).decode("utf-8")

    def _recv_frame(self) -> tuple[bool, int, bytes]:
        head = recv_exact(self.sock, 2)
        first, second = head
        fin = bool(first & 0x80)
        opcode = first & 0x0F
        masked = bool(second & 0x80)
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", recv_exact(self.sock, 2))[0]
        elif length == 127:
            length = struct.unpack("!Q", recv_exact(self.sock, 8))[0]
        mask = recv_exact(self.sock, 4) if masked else None
        payload = recv_exact(self.sock, length)
        if mask is not None:
            payload = bytes(
                byte ^ mask[index % 4] for index, byte in enumerate(payload)
            )
        return fin, opcode, payload

    def close(self) -> None:
        try:
            self._send_frame(0x8, b"")
        except OSError:
            pass
        self.sock.close()


def read_http_header(sock: socket.socket) -> str:
    data = bytearray()
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(4096)
        if not chunk:
            raise RuntimeError("connection closed during WebSocket upgrade")
        data.extend(chunk)
    return data.split(b"\r\n\r\n", 1)[0].decode("iso-8859-1")


def recv_exact(sock: socket.socket, length: int) -> bytes:
    parts = bytearray()
    while len(parts) < length:
        chunk = sock.recv(length - len(parts))
        if not chunk:
            raise RuntimeError("WebSocket connection closed")
        parts.extend(chunk)
    return bytes(parts)


def terminate(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is None:
        proc.terminate()


def wait_then_kill(proc: subprocess.Popen[str], timeout_s: float) -> None:
    if proc.poll() is not None:
        return
    try:
        proc.wait(timeout=timeout_s)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=timeout_s)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        raise
    except Exception as error:
        sys.stderr.write(f"{type(error).__name__}: {error}\n")
        raise SystemExit(1)
