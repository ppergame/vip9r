#!/usr/bin/env python3

from __future__ import annotations

import codecs
import json
import math
import os
import re
import stat as stat_module
import sys
import time
import traceback
from dataclasses import dataclass
from decimal import Decimal, ROUND_HALF_UP, localcontext
from typing import Any, TextIO


FOLLOW_SNAPSHOT_EVENTS = 30
COMMAND_LIMIT = 220
OUTPUT_TAIL_LINES = 8
OUTPUT_TAIL_CHARS = 3000
MISSING = object()


def usage(exit_code: int = 2) -> None:
    out = sys.stdout if exit_code == 0 else sys.stderr
    out.write(
        """usage:
  codex-events stream [options] CODEX_JSONL
  codex-events context ROLLOUT_JSONL
  codex-events inspect [options] [CODEX_JSONL_OR_TRACE_DIR]

Options:
  -f, --follow     Follow appended events.
      --reasoning  Show reasoning summary events.
      --verbose    Also print successful command output and full unknown-event blocks.
      --repo DIR   Repository root for auto-discovery.
  -h, --help       Show this help.

"""
    )
    raise SystemExit(exit_code)


def is_object(value: Any) -> bool:
    return isinstance(value, (dict, list))


def is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def is_finite_number(value: Any) -> bool:
    return is_number(value) and math.isfinite(value)


def get_prop(value: Any, key: str) -> Any:
    if isinstance(value, dict):
        return value.get(key, MISSING)
    return MISSING


def coalesce(*values: Any) -> Any:
    for value in values:
        if value is not MISSING and value is not None:
            return value
    return MISSING


def js_number_string(value: int | float) -> str:
    if isinstance(value, float):
        if math.isnan(value):
            return "NaN"
        if math.isinf(value):
            return "Infinity" if value > 0 else "-Infinity"
        if value == 0:
            return "0"
        if value.is_integer():
            return str(int(value))
    return str(value)


def js_string(value: Any) -> str:
    if value is None or value is MISSING:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    if is_number(value):
        return js_number_string(value)
    if isinstance(value, list):
        return ",".join(js_string(item) for item in value)
    if isinstance(value, dict):
        return "[object Object]"
    return str(value)


def inline_text(value: Any) -> str:
    return re.sub(r"\s+", " ", js_string(value)).strip()


def limited_inline_text(value: Any, limit: int) -> str:
    text = inline_text(value)
    if len(text) <= limit:
        return text
    return f"{text[: max(0, limit - 1)]}…"


def block_text(value: Any) -> str:
    return js_string(value).replace("\r\n", "\n").replace("\r", "\n").rstrip()


def write_indented_text(out: TextIO, label: str, value: Any) -> None:
    text = block_text(value)
    if not text:
        return
    lines = text.split("\n")
    if len(lines) == 1:
        out.write(f"  {label}: {lines[0]}\n")
        return
    out.write(f"  {label}:\n")
    for line in lines:
        out.write(f"    {line}\n")


def trimmed(value: Any) -> str:
    return js_string(value).strip()


def parse_json_string(value: Any) -> Any:
    if not isinstance(value, str):
        return MISSING
    text = value.strip()
    if not text.startswith("{") and not text.startswith("["):
        return MISSING
    try:
        return parse_json(text)
    except ValueError:
        return MISSING


def reject_json_constant(value: str) -> None:
    raise ValueError(f"invalid JSON constant: {value}")


def parse_json(text: str) -> Any:
    return json.loads(text, parse_constant=reject_json_constant)


def json_compact(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def json_pretty(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2)


def compact_error_text(value: Any, depth: int = 0) -> str:
    if depth > 4:
        return inline_text(value)
    if isinstance(value, str):
        parsed = parse_json_string(value)
        return simplify_error_text(
            compact_error_text(parsed, depth + 1) if parsed is not MISSING else value
        )
    if not is_object(value):
        return simplify_error_text(js_string(value))

    labels: list[str] = []
    for key in ("type", "code", "status"):
        label = get_prop(value, key)
        if isinstance(label, str) or is_number(label):
            labels.append(js_string(label))

    nested = coalesce(get_prop(value, "error"), get_prop(value, "message"), get_prop(value, "param"))
    message = json_compact(value) if nested is MISSING else compact_error_text(nested, depth + 1)
    prefix = f"{'/'.join(labels)}: " if labels else ""
    return simplify_error_text(f"{prefix}{message}")


def simplify_error_text(value: Any) -> str:
    text = inline_text(value)
    if "property_name_above_max_length" in text or "Invalid property name" in text:
        length_match = re.search(r"got a string with length ([0-9]+)", text, re.IGNORECASE)
        expected_match = re.search(r"maximum length ([0-9]+)", text, re.IGNORECASE)
        suffix = (
            f" ({length_match.group(1)}/{expected_match.group(1)})"
            if length_match and expected_match
            else ""
        )
        return f"property_name_above_max_length: tool-call argument property name too long{suffix}"
    return text


def display_command(value: Any) -> str:
    return limited_inline_text("command" if value is MISSING or value is None else value, COMMAND_LIMIT)


def output_tail(value: Any) -> str:
    text = block_text(value)
    if not text:
        return ""

    lines = text.split("\n")
    omitted_lines = max(0, len(lines) - OUTPUT_TAIL_LINES)
    tail = "\n".join(lines[-OUTPUT_TAIL_LINES:])
    omitted_chars = max(0, len(tail) - OUTPUT_TAIL_CHARS)
    if omitted_chars > 0:
        tail = tail[-OUTPUT_TAIL_CHARS:]

    markers: list[str] = []
    if omitted_lines > 0:
        markers.append(f"… {omitted_lines} earlier lines omitted")
    if omitted_chars > 0:
        markers.append(f"… {omitted_chars} earlier chars omitted from tail")
    if not markers:
        return tail
    marker_text = "\n".join(markers)
    return f"{marker_text}\n{tail}"


def js_to_fixed(value: int | float, digits: int) -> str:
    scale = 10**digits
    sign = "-" if value < 0 else ""
    with localcontext() as context:
        context.prec = 100
        scaled = (
            Decimal.from_float(abs(float(value)))
            * Decimal(scale)
        ).to_integral_value(rounding=ROUND_HALF_UP)
    integer = int(scaled // scale)
    fraction = int(scaled % scale)
    return f"{sign}{integer}.{fraction:0{digits}d}"


def compact_number(value: Any) -> str:
    if not is_finite_number(value):
        return "?"
    if abs(value) >= 1000:
        return f"{js_to_fixed(value / 1000, 1)}k"
    return js_number_string(value)


def compact_percent(ratio: Any) -> str:
    if not is_finite_number(ratio):
        return "?"
    if ratio > 0 and ratio < 0.01:
        return "<1%"
    return f"{math.floor(ratio * 100 + 0.5)}%"


def relative_sandbox_path(value: Any) -> str:
    text = js_string(value)
    if text.startswith("/run/rust/"):
        return text[len("/run/rust/") :]
    return text


def item_keys(item: Any) -> str:
    if isinstance(item, dict):
        keys = [key for key in item.keys() if key not in ("id", "type")]
    elif isinstance(item, list):
        keys = [str(index) for index in range(len(item))]
    else:
        keys = []
    return ",".join(sorted(keys))


class CompactRenderer:
    def __init__(
        self,
        *,
        out: TextIO,
        show_command_starts: bool = True,
        show_reasoning: bool = False,
        verbose: bool = False,
    ) -> None:
        self.out = out
        self.show_command_starts = show_command_starts
        self.show_reasoning = show_reasoning
        self.verbose = verbose
        self.pending_agent: str | None = None
        self.pending_file_changes: dict[str, int] = {}
        self.last_todo: str | None = None

    def write(self, line: str = "") -> None:
        self.out.write(f"{line}\n")

    def flush_agent(self) -> None:
        if not self.pending_agent:
            return
        self.flush_file_changes()
        self.write_text("agent", self.pending_agent)
        self.pending_agent = None

    def flush_file_changes(self) -> None:
        if not self.pending_file_changes:
            return
        parts: list[str] = []
        for key, count in self.pending_file_changes.items():
            split = key.split("\t")
            kind = split[0] if split else ""
            path = split[1] if len(split) > 1 else ""
            parts.append(f"{kind} {path}{f' ×{count}' if count > 1 else ''}")
        self.pending_file_changes.clear()
        self.write(f"  ✎ {', '.join(parts)}")

    def write_block(self, label: str, value: Any) -> None:
        text = block_text(value)
        if not text:
            return
        self.write(f"  {label}:")
        for line in text.split("\n"):
            self.write(f"    {line}")

    def write_text(self, label: str, value: Any) -> None:
        write_indented_text(self.out, label, value)

    def render_event(self, event: Any) -> None:
        event_type = get_prop(event, "type")
        if not is_object(event) or not isinstance(event_type, str):
            self.flush_agent()
            self.flush_file_changes()
            self.write("? malformed event")
            return

        if event_type == "thread.started":
            self.flush_agent()
            self.flush_file_changes()
            thread_id = get_prop(event, "thread_id")
            self.write(f"codex {thread_id if isinstance(thread_id, str) else 'unknown-thread'}")
        elif event_type == "turn.started":
            self.flush_agent()
            self.flush_file_changes()
            self.write("· turn")
        elif event_type == "turn.completed":
            self.flush_agent()
            self.flush_file_changes()
            self.render_usage(get_prop(event, "usage"))
        elif event_type == "turn.failed":
            self.flush_agent()
            self.flush_file_changes()
            self.write_text(
                "turn failed",
                compact_error_text(coalesce(get_prop(event, "error"), get_prop(event, "message"), event)),
            )
        elif event_type == "error":
            self.flush_agent()
            self.flush_file_changes()
            self.write_text(
                "error",
                compact_error_text(coalesce(get_prop(event, "message"), get_prop(event, "error"), event)),
            )
        elif event_type in ("item.started", "item.updated", "item.completed"):
            self.render_item(event_type, get_prop(event, "item"))
        else:
            self.flush_agent()
            self.flush_file_changes()
            if self.verbose:
                self.write_block(f"? {event_type}", json_pretty(event))
            else:
                self.write(f"? {event_type}")

    def render_usage(self, usage: Any) -> None:
        if not is_object(usage):
            return
        input_tokens = compact_number(get_prop(usage, "input_tokens"))
        cached = compact_number(get_prop(usage, "cached_input_tokens"))
        output = compact_number(get_prop(usage, "output_tokens"))
        reasoning = compact_number(get_prop(usage, "reasoning_output_tokens"))
        self.write(f"  usage: in {input_tokens}, cached {cached}, out {output}, reasoning {reasoning}")

    def render_item(self, event_type: str, item: Any) -> None:
        item_type = get_prop(item, "type")
        if not is_object(item) or not isinstance(item_type, str):
            self.flush_agent()
            self.flush_file_changes()
            self.write(f"  ? {event_type}")
            return

        if item_type == "agent_message":
            text = get_prop(item, "text")
            if isinstance(text, str) and text.strip():
                self.render_agent(text)
            return

        if item_type == "command_execution":
            self.flush_agent()
            self.flush_file_changes()
            self.render_command(event_type, item)
            return

        if item_type == "file_change":
            self.flush_agent()
            if event_type == "item.completed":
                self.record_file_change(item)
            return

        if item_type == "todo_list":
            self.flush_agent()
            self.flush_file_changes()
            self.render_todo_list(item)
            return

        if item_type == "reasoning":
            text = get_prop(item, "text")
            if self.show_reasoning and isinstance(text, str) and text.strip():
                self.flush_agent()
                self.flush_file_changes()
                self.write_text("think", text)
            return

        if item_type == "error":
            self.flush_agent()
            self.flush_file_changes()
            self.write_text(
                "error",
                compact_error_text(coalesce(get_prop(item, "message"), get_prop(item, "error"), item)),
            )
            return

        self.flush_agent()
        self.flush_file_changes()
        keys = item_keys(item)
        suffix = f" {{{keys}}}" if keys else ""
        if self.verbose:
            self.write_block(f"? {item_type}{suffix}", json_pretty(item))
        else:
            self.write(f"  ? {item_type}{suffix}")

    def render_agent(self, text: Any) -> None:
        body = trimmed(text)
        if not body:
            return
        self.flush_agent()
        self.flush_file_changes()
        self.pending_agent = body

    def render_command(self, event_type: str, item: Any) -> None:
        command = display_command(coalesce(get_prop(item, "command"), "command"))
        if event_type == "item.started":
            if self.show_command_starts:
                self.write(f"  ▸ {command}")
            return
        if event_type == "item.updated":
            return

        code = get_prop(item, "exit_code")
        status = code if is_number(code) else coalesce(get_prop(item, "status"), "?")
        mark = "✓" if is_number(code) and code == 0 else "✗" if is_number(code) else "?"
        self.write(f"  {mark} {command} ({js_string(status)})")
        aggregated_output = get_prop(item, "aggregated_output")
        if self.verbose and isinstance(aggregated_output, str) and aggregated_output.strip():
            self.write_block("output", aggregated_output)
        elif mark != "✓" and isinstance(aggregated_output, str) and aggregated_output.strip():
            self.write_block("output tail", output_tail(aggregated_output))

    def record_file_change(self, item: Any) -> None:
        changes = get_prop(item, "changes")
        if not isinstance(changes, list):
            changes = []
        if len(changes) == 0:
            key = "change\tfile change"
            self.pending_file_changes[key] = self.pending_file_changes.get(key, 0) + 1
            return
        for change in changes:
            if not is_object(change):
                continue
            kind = get_prop(change, "kind")
            if not isinstance(kind, str):
                kind = "change"
            path = relative_sandbox_path(get_prop(change, "path"))
            key = f"{kind}\t{path}"
            self.pending_file_changes[key] = self.pending_file_changes.get(key, 0) + 1

    def render_todo_list(self, item: Any) -> None:
        items = get_prop(item, "items")
        if not isinstance(items, list):
            items = []
        total = len(items)
        done = sum(1 for entry in items if is_object(entry) and get_prop(entry, "completed") is True)
        active = next(
            (entry for entry in items if is_object(entry) and get_prop(entry, "completed") is not True),
            MISSING,
        )
        active_text_value = get_prop(active, "text")
        active_text = inline_text(active_text_value) if isinstance(active_text_value, str) else ""
        line = f"plan: {done}/{total} done{f' — {active_text}' if active_text else ''}"
        if line == self.last_todo:
            return
        self.last_todo = line
        self.write(f"  {line}")
        if self.verbose:
            for entry in items:
                if not is_object(entry):
                    continue
                mark = "✓" if get_prop(entry, "completed") is True else "·"
                text = get_prop(entry, "text") if isinstance(get_prop(entry, "text"), str) else json_compact(entry)
                self.write(f"    {mark} {text}")


@dataclass
class SessionFile:
    path: str
    kind: str
    size: int
    mtime_ms: float


def parse_stream_args(argv: list[str]) -> dict[str, Any]:
    options: dict[str, Any] = {"out_path": None, "show_reasoning": False}
    paths: list[str] = []
    for arg in argv:
        if arg in ("-h", "--help"):
            usage(0)
        elif arg == "--reasoning":
            options["show_reasoning"] = True
        elif arg.startswith("-"):
            sys.stderr.write(f"unknown option: {arg}\n")
            usage()
        else:
            paths.append(arg)
    if len(paths) != 1:
        usage()
    options["out_path"] = paths[0]
    return options


def stream_jsonl(options: dict[str, Any]) -> None:
    out_path = options.get("out_path")
    show_reasoning = bool(options.get("show_reasoning"))
    if not out_path:
        usage()

    renderer = CompactRenderer(out=sys.stderr, show_command_starts=True, show_reasoning=show_reasoning)
    with open(out_path, "w", encoding="utf-8", newline="\n") as raw:
        for raw_line in sys.stdin.buffer:
            line = raw_line.decode("utf-8", errors="replace")
            if line.endswith("\n"):
                line = line[:-1]
                if line.endswith("\r"):
                    line = line[:-1]
            elif line.endswith("\r"):
                line = line[:-1]
            raw.write(f"{line}\n")
            if not line.strip():
                continue
            try:
                renderer.render_event(parse_json(line))
            except Exception:
                sys.stderr.write("  ? malformed jsonl\n")

        renderer.flush_agent()
        renderer.flush_file_changes()


def parse_jsonl_text(text: str) -> tuple[list[Any], int]:
    events: list[Any] = []
    malformed = 0
    for line in text.split("\n"):
        if not line.strip():
            continue
        try:
            events.append(parse_json(line))
        except ValueError:
            malformed += 1
    return events, malformed


def read_jsonl(path: str) -> tuple[list[Any], int]:
    with open(path, "r", encoding="utf-8", errors="replace") as file:
        return parse_jsonl_text(file.read())


def read_optional(path: str) -> str | None:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as file:
            return file.read().rstrip()
    except FileNotFoundError:
        return None


def resolve_path(*parts: str) -> str:
    return os.path.abspath(os.path.join(*parts))


def resolve_input(input_path: str | None, repo: str) -> SessionFile | None:
    candidates: list[str] = []
    if input_path:
        direct = resolve_path(input_path)
        from_repo = resolve_path(repo, input_path)
        for base in (direct, from_repo):
            candidates.extend(
                [
                    base,
                    os.path.join(base, "codex.jsonl"),
                    os.path.join(base, "trace", "codex.jsonl"),
                ]
            )
    else:
        latest = next(iter(discover_session_files(repo)), None)
        if latest:
            return latest
    for candidate in candidates:
        file = stat_file(candidate, "input")
        if file:
            return file
    return None


def resolve_input_path(input_path: str | None, repo: str) -> str | None:
    resolved = resolve_input(input_path, repo)
    return resolved.path if resolved else None


def list_dir(path: str) -> list[os.DirEntry[str]]:
    try:
        with os.scandir(path) as entries:
            return list(entries)
    except FileNotFoundError:
        return []


def missing_path_error(error: OSError) -> bool:
    return isinstance(error, (FileNotFoundError, NotADirectoryError))


def file_exists(path: str) -> bool:
    try:
        return stat_module.S_ISREG(os.stat(path).st_mode)
    except OSError as error:
        if missing_path_error(error):
            return False
        raise


def stat_file(path: str, kind: str) -> SessionFile | None:
    try:
        stat = os.stat(path)
        if not stat_module.S_ISREG(stat.st_mode):
            return None
        return SessionFile(path=path, kind=kind, size=stat.st_size, mtime_ms=stat.st_mtime * 1000)
    except OSError as error:
        if missing_path_error(error):
            return None
        raise


def discover_session_files(repo: str) -> list[SessionFile]:
    root = resolve_path(repo)
    temp = os.path.join(root, "temp")
    sessions: list[SessionFile] = []

    for entry in list_dir(temp):
        if not entry.is_dir(follow_symlinks=False) or not entry.name.startswith("grinder."):
            continue
        candidate = stat_file(os.path.join(temp, entry.name, "trace", "codex.jsonl"), "live")
        if candidate:
            sessions.append(candidate)

    traces = os.path.join(temp, "traces")
    for entry in list_dir(traces):
        if not entry.is_dir(follow_symlinks=False):
            continue
        candidate = stat_file(os.path.join(traces, entry.name, "codex.jsonl"), "preserved")
        if candidate:
            sessions.append(candidate)

    sessions.sort(key=lambda session: (-session.mtime_ms, session.path))
    return sessions


def grinder_run_name(path: str) -> str | None:
    match = re.search(r"grinder\.[^/\\]+", str(path))
    return match.group(0) if match else None


def trace_sibling_paths(jsonl_path: str, filename: str) -> list[str]:
    paths = [os.path.join(os.path.dirname(jsonl_path), filename)]
    name = grinder_run_name(jsonl_path)
    temp_marker = "/temp/"
    temp_index = jsonl_path.find(temp_marker)
    if not name or temp_index == -1:
        return paths

    temp_root = jsonl_path[: temp_index + len("/temp")]
    paths.append(os.path.join(temp_root, name, "trace", filename))
    for entry in list_dir(os.path.join(temp_root, "traces")):
        if entry.is_dir(follow_symlinks=False) and entry.name.endswith(f"-{name}"):
            paths.append(os.path.join(temp_root, "traces", entry.name, filename))

    return list(dict.fromkeys(paths))


def final_text_for(jsonl_path: str) -> str | None:
    for path in trace_sibling_paths(jsonl_path, "final.md"):
        sibling = read_optional(path)
        if sibling:
            return sibling
    return None


def is_terminal_event(event: Any) -> bool:
    return is_object(event) and get_prop(event, "type") in ("turn.completed", "turn.failed")


def thread_id_for(events: list[Any]) -> str | None:
    started = next((event for event in events if is_object(event) and get_prop(event, "type") == "thread.started"), None)
    thread_id = get_prop(started, "thread_id")
    return thread_id if isinstance(thread_id, str) else None


def status_for(jsonl_path: str) -> str | None:
    for path in trace_sibling_paths(jsonl_path, "exit-status"):
        status = read_optional(path)
        if status is not None:
            return status
    return None


def token_count_sample(event: Any) -> tuple[int | float, int | float] | None:
    payload = get_prop(event, "payload") if is_object(event) else MISSING
    if not is_object(payload) or get_prop(payload, "type") != "token_count":
        return None
    info = get_prop(payload, "info")
    if not is_object(info):
        return None
    usage = get_prop(info, "last_token_usage")
    if not is_object(usage):
        return None
    tokens = get_prop(usage, "total_tokens")
    window = get_prop(info, "model_context_window")
    if (
        not is_finite_number(tokens)
        or not is_finite_number(window)
        or tokens <= 0
        or window <= 0
    ):
        return None
    return tokens, window


def context_summary_for_events(events: list[Any]) -> str | None:
    peak: tuple[int | float, int | float] | None = None
    for event in events:
        sample = token_count_sample(event)
        if not sample:
            continue
        tokens, window = sample
        if not peak or tokens / window > peak[0] / peak[1]:
            peak = sample
    if not peak:
        return None
    tokens, window = peak
    return f"peak {compact_percent(tokens / window)} ({compact_number(tokens)}/{compact_number(window)})"


def context_summary_for_file(path: str) -> str | None:
    if not file_exists(path):
        return None
    events, _malformed = read_jsonl(path)
    return context_summary_for_events(events)


def context_summary_for_trace(jsonl_path: str) -> str | None:
    for path in trace_sibling_paths(jsonl_path, "rollout.jsonl"):
        summary = context_summary_for_file(path)
        if summary:
            return summary
    return None


def inspect_jsonl(jsonl_path: str, options: dict[str, Any]) -> None:
    events, malformed = read_jsonl(jsonl_path)
    final_text = final_text_for(jsonl_path)
    sys.stdout.write("# grinder trace\n\n")
    status = status_for(jsonl_path)
    state = "incomplete" if not status and not any(is_terminal_event(event) for event in events) else None
    thread_id = thread_id_for(events)
    context_summary = context_summary_for_trace(jsonl_path)

    renderer = CompactRenderer(
        out=sys.stdout,
        show_command_starts=bool(options.get("follow")),
        show_reasoning=True,
        verbose=bool(options.get("verbose")),
    )
    for event in events:
        renderer.render_event(event)
    renderer.flush_agent()
    renderer.flush_file_changes()

    sys.stdout.write("\n── grinder ──\n")
    if status:
        sys.stdout.write(f"status: {status}\n")
    if state:
        sys.stdout.write(f"state: {state}\n")
    if context_summary:
        sys.stdout.write(f"context: {context_summary}\n")
    sys.stdout.write(f"file: {jsonl_path}\n")
    if thread_id:
        sys.stdout.write(f"thread: {thread_id}\n")
    if final_text:
        sys.stdout.write(f"final: {limited_inline_text(final_text, 220)}\n")
    if malformed > 0:
        sys.stdout.write(f"malformed lines skipped: {malformed}\n")


def parse_inspect_args(argv: list[str]) -> dict[str, Any]:
    options: dict[str, Any] = {
        "repo": os.getcwd(),
        "input": None,
        "follow": False,
        "verbose": False,
    }
    paths: list[str] = []
    index = 0
    while index < len(argv):
        arg = argv[index]
        if arg in ("-h", "--help"):
            usage(0)
        elif arg in ("-f", "--follow"):
            options["follow"] = True
        elif arg == "--verbose":
            options["verbose"] = True
        elif arg == "--repo":
            index += 1
            if index >= len(argv):
                usage()
            options["repo"] = resolve_path(argv[index])
        elif arg.startswith("-"):
            sys.stderr.write(f"unknown option: {arg}\n")
            usage()
        else:
            paths.append(arg)
        index += 1
    if len(paths) > 1:
        sys.stderr.write("expected at most one session path\n")
        usage()
    options["input"] = paths[0] if paths else None
    return options


def same_file(left: SessionFile | None, right: SessionFile | None) -> bool:
    return (left.path if left else None) == (right.path if right else None)


def read_file_range(path: str, start: int, length: int) -> bytes:
    with open(path, "rb") as file:
        file.seek(start)
        return file.read(length)


def render_follow_line(path: str, line: str, renderer: CompactRenderer) -> None:
    if not line.strip():
        return
    try:
        renderer.render_event(parse_json(line))
        renderer.flush_agent()
        renderer.flush_file_changes()
        renderer.out.flush()
    except Exception:
        sys.stderr.write(f"skipping malformed codex jsonl line from {path}\n")
        sys.stderr.flush()


def print_follow_snapshot(path: str, renderer: CompactRenderer) -> int:
    with open(path, "rb") as file:
        data = file.read()
    last_newline = data.rfind(b"\n")
    complete_bytes = 0 if last_newline == -1 else last_newline + 1
    events, malformed = parse_jsonl_text(data[:complete_bytes].decode("utf-8", errors="replace"))
    for event in events[-FOLLOW_SNAPSHOT_EVENTS:]:
        renderer.render_event(event)
    renderer.flush_agent()
    renderer.flush_file_changes()
    renderer.out.flush()
    if malformed > 0:
        sys.stderr.write(f"skipping {malformed} malformed codex jsonl lines from {path}\n")
        sys.stderr.flush()
    return complete_bytes


def follow(options: dict[str, Any]) -> None:
    current: SessionFile | None = None
    offset = 0
    buffered = ""
    decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
    renderer: CompactRenderer | None = None
    warned_waiting = False

    while True:
        input_path = options.get("input")
        next_file = resolve_input(input_path, options["repo"]) if input_path else next(
            iter(discover_session_files(options["repo"])), None
        )
        if (
            not input_path
            and current
            and current.kind == "live"
            and (not next_file or next_file.kind != "live")
            and os.path.exists(current.path)
        ):
            next_file = current
        if not next_file:
            if not warned_waiting:
                sys.stderr.write("waiting for grinder codex jsonl...\n")
                sys.stderr.flush()
                warned_waiting = True
            time.sleep(1)
            continue

        if not same_file(current, next_file):
            current = next_file
            buffered = ""
            decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
            renderer = CompactRenderer(
                out=sys.stdout,
                show_command_starts=True,
                show_reasoning=True,
                verbose=bool(options.get("verbose")),
            )
            sys.stderr.write(f"==> {current.path} <==\n")
            sys.stderr.flush()
            offset = print_follow_snapshot(current.path, renderer) if os.path.exists(current.path) else 0
            warned_waiting = False

        if not current or not os.path.exists(current.path):
            time.sleep(1)
            continue

        stat = os.stat(current.path)
        if not stat_module.S_ISREG(stat.st_mode):
            time.sleep(1)
            continue
        if stat.st_size < offset:
            offset = 0
            buffered = ""
            decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
            renderer = CompactRenderer(
                out=sys.stdout,
                show_command_starts=True,
                show_reasoning=True,
                verbose=bool(options.get("verbose")),
            )

        if stat.st_size > offset:
            data = read_file_range(current.path, offset, stat.st_size - offset)
            offset += len(data)
            buffered += decoder.decode(data, final=False)
            lines = buffered.split("\n")
            buffered = lines.pop() if lines else ""
            if renderer is None:
                renderer = CompactRenderer(
                    out=sys.stdout,
                    show_command_starts=True,
                    show_reasoning=True,
                    verbose=bool(options.get("verbose")),
                )
            for line in lines:
                render_follow_line(current.path, line, renderer)

        time.sleep(1)


def inspect(argv: list[str]) -> None:
    options = parse_inspect_args(argv)
    if options["follow"]:
        follow(options)
        return
    jsonl_path = resolve_input_path(options["input"], options["repo"])
    if not jsonl_path:
        sys.stderr.write(f"no grinder codex jsonl found under {resolve_path(options['repo'], 'temp')}\n")
        raise SystemExit(1)
    inspect_jsonl(jsonl_path, options)


def context(argv: list[str]) -> None:
    if len(argv) != 1:
        usage()
    summary = context_summary_for_file(resolve_path(argv[0]))
    if summary:
        sys.stdout.write(f"{summary}\n")


def main(argv: list[str]) -> None:
    mode = argv[0] if argv else None
    args = argv[1:] if argv else []
    if mode == "stream":
        stream_jsonl(parse_stream_args(args))
        return
    if mode == "context":
        context(args)
        return
    if mode == "inspect":
        inspect(args)
        return
    usage(0 if mode in ("-h", "--help") else 2)


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except SystemExit:
        raise
    except Exception:
        traceback.print_exc(file=sys.stderr)
        raise SystemExit(1)
