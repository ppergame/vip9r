#!/usr/bin/env node

import {
	closeSync,
	createWriteStream,
	existsSync,
	openSync,
	readdirSync,
	readFileSync,
	readSync,
	statSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { StringDecoder } from "node:string_decoder";

const FOLLOW_SNAPSHOT_EVENTS = 30;
const COMMAND_LIMIT = 220;
const OUTPUT_TAIL_LINES = 8;
const OUTPUT_TAIL_CHARS = 3000;

function usage(exitCode = 2) {
	const out = exitCode === 0 ? process.stdout : process.stderr;
	out.write(`usage:
  grinder-codex-events stream CODEX_JSONL
  grinder-codex-events context ROLLOUT_JSONL
  grinder-codex-events inspect [options] [CODEX_JSONL_OR_TRACE_DIR]

Options:
  -f, --follow     Follow appended events.
      --verbose    Also print successful command output and full unknown-event blocks.
      --repo DIR   Repository root for auto-discovery.
  -h, --help       Show this help.

`);
	process.exit(exitCode);
}

function isObject(value) {
	return typeof value === "object" && value !== null;
}

function inlineText(value) {
	return String(value ?? "").replace(/\s+/g, " ").trim();
}

function limitedInlineText(value, limit) {
	const text = inlineText(value);
	if (text.length <= limit) return text;
	return `${text.slice(0, Math.max(0, limit - 1))}…`;
}

function blockText(value) {
	return String(value ?? "").replace(/\r\n/g, "\n").replace(/\r/g, "\n").trimEnd();
}

function writeIndentedText(out, label, value) {
	const text = blockText(value);
	if (!text) return;
	const lines = text.split("\n");
	if (lines.length === 1) {
		out.write(`  ${label}: ${lines[0]}\n`);
		return;
	}
	out.write(`  ${label}:\n`);
	for (const line of lines) out.write(`    ${line}\n`);
}

function trimmed(value) {
	return String(value ?? "").trim();
}

function parseJsonString(value) {
	if (typeof value !== "string") return undefined;
	const text = value.trim();
	if (!text.startsWith("{") && !text.startsWith("[")) return undefined;
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

function compactErrorText(value, depth = 0) {
	if (depth > 4) return inlineText(value);
	if (typeof value === "string") {
		const parsed = parseJsonString(value);
		return simplifyErrorText(parsed ? compactErrorText(parsed, depth + 1) : value);
	}
	if (!isObject(value)) return simplifyErrorText(String(value ?? ""));

	const labels = [];
	for (const key of ["type", "code", "status"]) {
		const label = value[key];
		if (typeof label === "string" || typeof label === "number") labels.push(String(label));
	}
	const nested = value.error ?? value.message ?? value.param;
	const message = nested === undefined ? JSON.stringify(value) : compactErrorText(nested, depth + 1);
	return simplifyErrorText(`${labels.length > 0 ? `${labels.join("/")}: ` : ""}${message}`);
}

function simplifyErrorText(value) {
	const text = inlineText(value);
	if (text.includes("property_name_above_max_length") || text.includes("Invalid property name")) {
		const length = text.match(/got a string with length ([0-9]+)/i)?.[1];
		const expected = text.match(/maximum length ([0-9]+)/i)?.[1];
		const suffix = length && expected ? ` (${length}/${expected})` : "";
		return `property_name_above_max_length: tool-call argument property name too long${suffix}`;
	}
	return text;
}

function displayCommand(value) {
	return limitedInlineText(value ?? "command", COMMAND_LIMIT);
}

function outputTail(value) {
	const text = blockText(value);
	if (!text) return "";

	const lines = text.split("\n");
	const omittedLines = Math.max(0, lines.length - OUTPUT_TAIL_LINES);
	let tail = lines.slice(-OUTPUT_TAIL_LINES).join("\n");
	const omittedChars = Math.max(0, tail.length - OUTPUT_TAIL_CHARS);
	if (omittedChars > 0) tail = tail.slice(-OUTPUT_TAIL_CHARS);

	const markers = [];
	if (omittedLines > 0) markers.push(`… ${omittedLines} earlier lines omitted`);
	if (omittedChars > 0) markers.push(`… ${omittedChars} earlier chars omitted from tail`);
	if (markers.length === 0) return tail;
	return `${markers.join("\n")}\n${tail}`;
}

function compactNumber(value) {
	if (typeof value !== "number" || !Number.isFinite(value)) return "?";
	if (Math.abs(value) >= 1000) return `${(value / 1000).toFixed(1)}k`;
	return String(value);
}

function compactPercent(ratio) {
	if (typeof ratio !== "number" || !Number.isFinite(ratio)) return "?";
	if (ratio > 0 && ratio < 0.01) return "<1%";
	return `${Math.round(ratio * 100)}%`;
}

function relativeSandboxPath(value) {
	const text = String(value ?? "");
	if (text.startsWith("/run/rust/")) return text.slice("/run/rust/".length);
	return text;
}

function itemKeys(item) {
	return Object.keys(item)
		.filter((key) => !["id", "type"].includes(key))
		.sort()
		.join(",");
}

class CompactRenderer {
	constructor({
		out,
		showCommandStarts = true,
		showReasoning = false,
		verbose = false,
	}) {
		this.out = out;
		this.showCommandStarts = showCommandStarts;
		this.showReasoning = showReasoning;
		this.verbose = verbose;
		this.pendingAgent = undefined;
		this.pendingFileChanges = new Map();
		this.lastTodo = undefined;
	}

	write(line = "") {
		this.out.write(`${line}\n`);
	}

	flushAgent() {
		if (!this.pendingAgent) return;
		this.flushFileChanges();
		this.writeText("agent", this.pendingAgent);
		this.pendingAgent = undefined;
	}

	flushFileChanges() {
		if (this.pendingFileChanges.size === 0) return;
		const parts = [];
		for (const [key, count] of this.pendingFileChanges) {
			const [kind, path] = key.split("\t");
			parts.push(`${kind} ${path}${count > 1 ? ` ×${count}` : ""}`);
		}
		this.pendingFileChanges.clear();
		this.write(`  ✎ ${parts.join(", ")}`);
	}

	writeBlock(label, value) {
		const text = blockText(value);
		if (!text) return;
		this.write(`  ${label}:`);
		for (const line of text.split("\n")) this.write(`    ${line}`);
	}

	writeText(label, value) {
		writeIndentedText(this.out, label, value);
	}

	renderEvent(event) {
		if (!isObject(event) || typeof event.type !== "string") {
			this.flushAgent();
			this.flushFileChanges();
			this.write("? malformed event");
			return;
		}

		switch (event.type) {
			case "thread.started":
				this.flushAgent();
				this.flushFileChanges();
				this.write(`codex ${typeof event.thread_id === "string" ? event.thread_id : "unknown-thread"}`);
				break;
			case "turn.started":
				this.flushAgent();
				this.flushFileChanges();
				this.write("· turn");
				break;
			case "turn.completed":
				this.flushAgent();
				this.flushFileChanges();
				this.renderUsage(event.usage);
				break;
			case "turn.failed":
				this.flushAgent();
				this.flushFileChanges();
				this.writeText("turn failed", compactErrorText(event.error ?? event.message ?? event));
				break;
			case "error":
				this.flushAgent();
				this.flushFileChanges();
				this.writeText("error", compactErrorText(event.message ?? event.error ?? event));
				break;
			case "item.started":
			case "item.updated":
			case "item.completed":
				this.renderItem(event.type, event.item);
				break;
			default:
				this.flushAgent();
				this.flushFileChanges();
				if (this.verbose) this.writeBlock(`? ${event.type}`, JSON.stringify(event, null, 2));
				else this.write(`? ${event.type}`);
				break;
		}
	}

	renderUsage(usage) {
		if (!isObject(usage)) return;
		const input = compactNumber(usage.input_tokens);
		const cached = compactNumber(usage.cached_input_tokens);
		const output = compactNumber(usage.output_tokens);
		const reasoning = compactNumber(usage.reasoning_output_tokens);
		this.write(`  usage: in ${input}, cached ${cached}, out ${output}, reasoning ${reasoning}`);
	}

	renderItem(eventType, item) {
		if (!isObject(item) || typeof item.type !== "string") {
			this.flushAgent();
			this.flushFileChanges();
			this.write(`  ? ${eventType}`);
			return;
		}

		if (item.type === "agent_message") {
			if (typeof item.text === "string" && item.text.trim()) {
				this.renderAgent(item.text);
			}
			return;
		}

		if (item.type === "command_execution") {
			this.flushAgent();
			this.flushFileChanges();
			this.renderCommand(eventType, item);
			return;
		}

		if (item.type === "file_change") {
			this.flushAgent();
			if (eventType === "item.completed") this.recordFileChange(item);
			return;
		}

		if (item.type === "todo_list") {
			this.flushAgent();
			this.flushFileChanges();
			this.renderTodoList(item);
			return;
		}

		if (item.type === "reasoning") {
			if (this.showReasoning && typeof item.text === "string" && item.text.trim()) {
				this.flushAgent();
				this.flushFileChanges();
				this.writeText("think", item.text);
			}
			return;
		}

		if (item.type === "error") {
			this.flushAgent();
			this.flushFileChanges();
			this.writeText("error", compactErrorText(item.message ?? item.error ?? item));
			return;
		}

		this.flushAgent();
		this.flushFileChanges();
		const keys = itemKeys(item);
		if (this.verbose) this.writeBlock(`? ${item.type}${keys ? ` {${keys}}` : ""}`, JSON.stringify(item, null, 2));
		else this.write(`  ? ${item.type}${keys ? ` {${keys}}` : ""}`);
	}

	renderAgent(text) {
		const body = trimmed(text);
		if (!body) return;
		this.flushAgent();
		this.flushFileChanges();
		this.pendingAgent = body;
	}

	renderCommand(eventType, item) {
		const command = displayCommand(item.command ?? "command");
		if (eventType === "item.started") {
			if (this.showCommandStarts) this.write(`  ▸ ${command}`);
			return;
		}
		if (eventType === "item.updated") {
			return;
		}

		const code = item.exit_code;
		const status = typeof code === "number" ? code : item.status ?? "?";
		const mark = code === 0 ? "✓" : typeof code === "number" ? "✗" : "?";
		this.write(`  ${mark} ${command} (${status})`);
		if (this.verbose && typeof item.aggregated_output === "string" && item.aggregated_output.trim()) {
			this.writeBlock("output", item.aggregated_output);
		} else if (mark !== "✓" && typeof item.aggregated_output === "string" && item.aggregated_output.trim()) {
			this.writeBlock("output tail", outputTail(item.aggregated_output));
		}
	}

	recordFileChange(item) {
		const changes = Array.isArray(item.changes) ? item.changes : [];
		if (changes.length === 0) {
			this.pendingFileChanges.set("change\tfile change", (this.pendingFileChanges.get("change\tfile change") ?? 0) + 1);
			return;
		}
		for (const change of changes) {
			if (!isObject(change)) continue;
			const kind = typeof change.kind === "string" ? change.kind : "change";
			const path = relativeSandboxPath(change.path);
			const key = `${kind}\t${path}`;
			this.pendingFileChanges.set(key, (this.pendingFileChanges.get(key) ?? 0) + 1);
		}
	}

	renderTodoList(item) {
		const items = Array.isArray(item.items) ? item.items : [];
		const total = items.length;
		const done = items.filter((entry) => isObject(entry) && entry.completed === true).length;
		const active = items.find((entry) => isObject(entry) && entry.completed !== true);
		const activeText = isObject(active) && typeof active.text === "string" ? inlineText(active.text) : "";
		const line = `plan: ${done}/${total} done${activeText ? ` — ${activeText}` : ""}`;
		if (line === this.lastTodo) return;
		this.lastTodo = line;
		this.write(`  ${line}`);
		if (this.verbose) {
			for (const entry of items) {
				if (!isObject(entry)) continue;
				const mark = entry.completed === true ? "✓" : "·";
				const text = typeof entry.text === "string" ? entry.text : JSON.stringify(entry);
				this.write(`    ${mark} ${text}`);
			}
		}
	}
}

async function streamJsonl(outPath) {
	if (!outPath) usage();
	const raw = createWriteStream(outPath, { flags: "w" });
	const renderer = new CompactRenderer({
		out: process.stderr,
		showCommandStarts: true,
	});
	const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });

	for await (const line of lines) {
		raw.write(`${line}\n`);
		if (!line.trim()) continue;
		try {
			renderer.renderEvent(JSON.parse(line));
		} catch {
			process.stderr.write("  ? malformed jsonl\n");
		}
	}

	renderer.flushAgent();
	renderer.flushFileChanges();
	await new Promise((resolveStream, rejectStream) => {
		raw.end((error) => (error ? rejectStream(error) : resolveStream()));
	});
}

function parseJsonlText(text) {
	const events = [];
	let malformed = 0;
	for (const line of text.split("\n")) {
		if (!line.trim()) continue;
		try {
			events.push(JSON.parse(line));
		} catch {
			malformed += 1;
		}
	}
	return { events, malformed };
}

function readJsonl(path) {
	return parseJsonlText(readFileSync(path, "utf8"));
}

function readOptional(path) {
	try {
		return readFileSync(path, "utf8").trimEnd();
	} catch (error) {
		if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") return undefined;
		throw error;
	}
}

function resolveInput(input, repo) {
	const candidates = [];
	if (input) {
		const direct = resolve(input);
		const fromRepo = resolve(repo, input);
		for (const base of [direct, fromRepo]) {
			candidates.push(base, join(base, "codex.jsonl"), join(base, "trace", "codex.jsonl"));
		}
	} else {
		const latest = discoverSessionFiles(repo)[0];
		if (latest) return latest;
	}
	for (const candidate of candidates) {
		const file = statFile(candidate, "input");
		if (file) return file;
	}
	return undefined;
}

function resolveInputPath(input, repo) {
	return resolveInput(input, repo)?.path;
}

function listDir(path) {
	try {
		return readdirSync(path, { withFileTypes: true });
	} catch (error) {
		if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") return [];
		throw error;
	}
}

function missingPathError(error) {
	if (!error || typeof error !== "object" || !("code" in error)) return false;
	return error.code === "ENOENT" || error.code === "ENOTDIR";
}

function fileExists(path) {
	try {
		return statSync(path).isFile();
	} catch (error) {
		if (missingPathError(error)) return false;
		throw error;
	}
}

function statFile(path, kind) {
	try {
		const stat = statSync(path);
		if (!stat.isFile()) return undefined;
		return { path, kind, size: stat.size, mtimeMs: stat.mtimeMs };
	} catch (error) {
		if (missingPathError(error)) return undefined;
		throw error;
	}
}

function discoverSessionFiles(repo) {
	const root = resolve(repo);
	const temp = join(root, "temp");
	const sessions = [];

	for (const entry of listDir(temp)) {
		if (!entry.isDirectory() || !entry.name.startsWith("grinder.")) continue;
		const candidate = statFile(join(temp, entry.name, "trace", "codex.jsonl"), "live");
		if (candidate) sessions.push(candidate);
	}

	for (const entry of listDir(join(temp, "traces"))) {
		if (!entry.isDirectory()) continue;
		const candidate = statFile(join(temp, "traces", entry.name, "codex.jsonl"), "preserved");
		if (candidate) sessions.push(candidate);
	}

	sessions.sort((left, right) => right.mtimeMs - left.mtimeMs || left.path.localeCompare(right.path));
	return sessions;
}

function grinderRunName(path) {
	return String(path).match(/grinder\.[^/\\]+/)?.[0];
}

function traceSiblingPaths(jsonlPath, filename) {
	const paths = [join(dirname(jsonlPath), filename)];
	const name = grinderRunName(jsonlPath);
	const tempMarker = "/temp/";
	const tempIndex = jsonlPath.indexOf(tempMarker);
	if (!name || tempIndex === -1) return paths;

	const tempRoot = jsonlPath.slice(0, tempIndex + "/temp".length);
	paths.push(join(tempRoot, name, "trace", filename));
	for (const entry of listDir(join(tempRoot, "traces"))) {
		if (entry.isDirectory() && entry.name.endsWith(`-${name}`)) {
			paths.push(join(tempRoot, "traces", entry.name, filename));
		}
	}

	return [...new Set(paths)];
}

function finalTextFor(jsonlPath) {
	for (const path of traceSiblingPaths(jsonlPath, "final.md")) {
		const sibling = readOptional(path);
		if (sibling) return sibling;
	}
	return undefined;
}

function isTerminalEvent(event) {
	return isObject(event) && ["turn.completed", "turn.failed"].includes(event.type);
}

function threadIdFor(events) {
	const started = events.find((event) => isObject(event) && event.type === "thread.started");
	return isObject(started) && typeof started.thread_id === "string" ? started.thread_id : undefined;
}

function statusFor(jsonlPath) {
	for (const path of traceSiblingPaths(jsonlPath, "exit-status")) {
		const status = readOptional(path);
		if (status !== undefined) return status;
	}
	return undefined;
}

function tokenCountSample(event) {
	const payload = isObject(event) ? event.payload : undefined;
	if (!isObject(payload) || payload.type !== "token_count") return undefined;
	const info = payload.info;
	if (!isObject(info)) return undefined;
	const usage = info.last_token_usage;
	if (!isObject(usage)) return undefined;
	const tokens = usage.total_tokens;
	const window = info.model_context_window;
	if (
		typeof tokens !== "number" ||
		typeof window !== "number" ||
		!Number.isFinite(tokens) ||
		!Number.isFinite(window) ||
		tokens <= 0 ||
		window <= 0
	) {
		return undefined;
	}
	return { tokens, window };
}

function contextSummaryForEvents(events) {
	let peak;
	for (const event of events) {
		const sample = tokenCountSample(event);
		if (!sample) continue;
		const ratio = sample.tokens / sample.window;
		if (!peak || ratio > peak.tokens / peak.window) peak = sample;
	}
	if (!peak) return undefined;
	return `peak ${compactPercent(peak.tokens / peak.window)} (${compactNumber(peak.tokens)}/${compactNumber(peak.window)})`;
}

function contextSummaryForFile(path) {
	if (!fileExists(path)) return undefined;
	const { events } = readJsonl(path);
	return contextSummaryForEvents(events);
}

function contextSummaryForTrace(jsonlPath) {
	for (const path of traceSiblingPaths(jsonlPath, "rollout.jsonl")) {
		const summary = contextSummaryForFile(path);
		if (summary) return summary;
	}
	return undefined;
}

function inspectJsonl(jsonlPath, options) {
	const { events, malformed } = readJsonl(jsonlPath);
	const finalText = finalTextFor(jsonlPath);
	process.stdout.write("# grinder trace\n\n");
	const status = statusFor(jsonlPath);
	const state = !status && !events.some(isTerminalEvent) ? "incomplete" : undefined;
	const threadId = threadIdFor(events);
	const contextSummary = contextSummaryForTrace(jsonlPath);

	const renderer = new CompactRenderer({
		out: process.stdout,
		showCommandStarts: options.follow,
		showReasoning: true,
		verbose: options.verbose,
	});
	for (const event of events) renderer.renderEvent(event);
	renderer.flushAgent();
	renderer.flushFileChanges();

	process.stdout.write("\n── grinder ──\n");
	if (status) process.stdout.write(`status: ${status}\n`);
	if (state) process.stdout.write(`state: ${state}\n`);
	if (contextSummary) process.stdout.write(`context: ${contextSummary}\n`);
	process.stdout.write(`file: ${jsonlPath}\n`);
	if (threadId) process.stdout.write(`thread: ${threadId}\n`);
	if (finalText) process.stdout.write(`final: ${limitedInlineText(finalText, 220)}\n`);
	if (malformed > 0) process.stdout.write(`malformed lines skipped: ${malformed}\n`);
}

function parseInspectArgs(argv) {
	const options = { repo: process.cwd(), input: undefined, follow: false, verbose: false };
	const paths = [];
	for (let index = 0; index < argv.length; index += 1) {
		const arg = argv[index];
		if (arg === "-h" || arg === "--help") usage(0);
		else if (arg === "-f" || arg === "--follow") options.follow = true;
		else if (arg === "--verbose") options.verbose = true;
		else if (arg === "--repo") {
			const value = argv[++index];
			if (!value) usage();
			options.repo = resolve(value);
		} else if (arg.startsWith("-")) {
			process.stderr.write(`unknown option: ${arg}\n`);
			usage();
		} else {
			paths.push(arg);
		}
	}
	if (paths.length > 1) {
		process.stderr.write("expected at most one session path\n");
		usage();
	}
	options.input = paths[0];
	return options;
}

function sleep(ms) {
	return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function sameFile(left, right) {
	return left?.path === right?.path;
}

function readFileRange(path, start, length) {
	const buffer = Buffer.allocUnsafe(length);
	const fd = openSync(path, "r");
	try {
		const bytesRead = readSync(fd, buffer, 0, length, start);
		return buffer.subarray(0, bytesRead);
	} finally {
		closeSync(fd);
	}
}

function renderFollowLine(path, line, renderer) {
	if (!line.trim()) return;
	try {
		renderer.renderEvent(JSON.parse(line));
		renderer.flushAgent();
		renderer.flushFileChanges();
	} catch {
		process.stderr.write(`skipping malformed codex jsonl line from ${path}\n`);
	}
}

function printFollowSnapshot(path, renderer) {
	const bytes = readFileSync(path);
	const lastNewline = bytes.lastIndexOf(0x0a);
	const completeBytes = lastNewline === -1 ? 0 : lastNewline + 1;
	const { events, malformed } = parseJsonlText(bytes.subarray(0, completeBytes).toString("utf8"));
	for (const event of events.slice(-FOLLOW_SNAPSHOT_EVENTS)) renderer.renderEvent(event);
	renderer.flushAgent();
	renderer.flushFileChanges();
	if (malformed > 0) process.stderr.write(`skipping ${malformed} malformed codex jsonl lines from ${path}\n`);
	return completeBytes;
}

async function follow(options) {
	let current;
	let offset = 0;
	let buffered = "";
	let decoder = new StringDecoder("utf8");
	let renderer;
	let warnedWaiting = false;

	for (;;) {
		let next = options.input ? resolveInput(options.input, options.repo) : discoverSessionFiles(options.repo)[0];
		if (!options.input && current?.kind === "live" && next?.kind !== "live" && existsSync(current.path)) {
			next = current;
		}
		if (!next) {
			if (!warnedWaiting) {
				process.stderr.write("waiting for grinder codex jsonl...\n");
				warnedWaiting = true;
			}
			await sleep(1000);
			continue;
		}

		if (!sameFile(current, next)) {
			current = next;
			buffered = "";
			decoder = new StringDecoder("utf8");
			renderer = new CompactRenderer({
				out: process.stdout,
				showCommandStarts: true,
				showReasoning: true,
				verbose: options.verbose,
			});
			process.stderr.write(`==> ${current.path} <==\n`);
			offset = existsSync(current.path) ? printFollowSnapshot(current.path, renderer) : 0;
			warnedWaiting = false;
		}

		if (!current || !existsSync(current.path)) {
			await sleep(1000);
			continue;
		}

		const stat = statSync(current.path);
		if (!stat.isFile()) {
			await sleep(1000);
			continue;
		}
		if (stat.size < offset) {
			offset = 0;
			buffered = "";
			decoder = new StringDecoder("utf8");
			renderer = new CompactRenderer({
				out: process.stdout,
				showCommandStarts: true,
				showReasoning: true,
				verbose: options.verbose,
			});
		}

		if (stat.size > offset) {
			const bytes = readFileRange(current.path, offset, stat.size - offset);
			offset += bytes.length;
			buffered += decoder.write(bytes);
			const lines = buffered.split("\n");
			buffered = lines.pop() ?? "";
			for (const line of lines) renderFollowLine(current.path, line, renderer);
		}

		await sleep(1000);
	}
}

function inspect(argv) {
	const options = parseInspectArgs(argv);
	if (options.follow) {
		return follow(options);
	}
	const jsonlPath = resolveInputPath(options.input, options.repo);
	if (!jsonlPath) {
		process.stderr.write(`no grinder codex jsonl found under ${resolve(options.repo, "temp")}\n`);
		process.exitCode = 1;
		return;
	}
	inspectJsonl(jsonlPath, options);
}

function context(argv) {
	if (argv.length !== 1) usage();
	const summary = contextSummaryForFile(resolve(argv[0]));
	if (summary) process.stdout.write(`${summary}\n`);
}

async function main() {
	const [mode, ...args] = process.argv.slice(2);
	if (mode === "stream") {
		await streamJsonl(args[0]);
		return;
	}
	if (mode === "context") {
		context(args);
		return;
	}
	if (mode === "inspect") {
		await inspect(args);
		return;
	}
	usage(mode === "-h" || mode === "--help" ? 0 : 2);
}

main().catch((error) => {
	process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
	process.exitCode = 1;
});
