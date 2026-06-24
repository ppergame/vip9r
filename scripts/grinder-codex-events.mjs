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

const TEXT_LIMIT = 140;
const OUTPUT_LIMIT = 180;
const FOLLOW_SNAPSHOT_EVENTS = 30;

function usage(exitCode = 2) {
	const out = exitCode === 0 ? process.stdout : process.stderr;
	out.write(`usage:
  grinder-codex-events stream CODEX_JSONL
  grinder-codex-events context ROLLOUT_JSONL
  grinder-codex-events inspect [options] [CODEX_JSONL_OR_TRACE_DIR]

Options:
  -f, --follow     Follow appended events.
      --verbose    Print full message and command-output blocks.
      --repo DIR   Repository root for auto-discovery.
  -h, --help       Show this help.

`);
	process.exit(exitCode);
}

function isObject(value) {
	return typeof value === "object" && value !== null;
}

function firstLine(value, limit = TEXT_LIMIT) {
	const text = String(value ?? "").replace(/\s+/g, " ").trim();
	if (text.length <= limit) return text;
	return `${text.slice(0, Math.max(0, limit - 1))}…`;
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
	constructor({ out, suppressFinalAgent, verbose = false }) {
		this.out = out;
		this.suppressFinalAgent = suppressFinalAgent;
		this.verbose = verbose;
		this.pendingAgent = undefined;
		this.lastTodo = undefined;
	}

	write(line = "") {
		this.out.write(`${line}\n`);
	}

	flushAgent() {
		if (!this.pendingAgent) return;
		if (this.suppressFinalAgent) {
			this.discardAgent();
			return;
		}
		if (this.verbose) this.writeBlock("agent", this.pendingAgent);
		else this.write(`  agent: ${firstLine(this.pendingAgent)}`);
		this.pendingAgent = undefined;
	}

	discardAgent() {
		this.pendingAgent = undefined;
	}

	writeBlock(label, value) {
		const text = String(value ?? "").replace(/\r\n/g, "\n").replace(/\r/g, "\n").trimEnd();
		if (!text) return;
		this.write(`  ${label}:`);
		for (const line of text.split("\n")) this.write(`    ${line}`);
	}

	renderEvent(event) {
		if (!isObject(event) || typeof event.type !== "string") {
			this.flushAgent();
			this.write("? malformed event");
			return;
		}

		switch (event.type) {
			case "thread.started":
				this.flushAgent();
				this.write(`codex ${typeof event.thread_id === "string" ? event.thread_id : "unknown-thread"}`);
				break;
			case "turn.started":
				this.flushAgent();
				this.write("· turn");
				break;
			case "turn.completed":
				if (this.suppressFinalAgent) this.discardAgent();
				else this.flushAgent();
				this.renderUsage(event.usage);
				break;
			case "turn.failed":
				this.flushAgent();
				this.write(`  turn failed: ${firstLine(event.error ?? event.message ?? JSON.stringify(event), OUTPUT_LIMIT)}`);
				break;
			case "error":
				this.flushAgent();
				this.write(`  error: ${firstLine(event.message ?? event.error ?? JSON.stringify(event), OUTPUT_LIMIT)}`);
				break;
			case "item.started":
			case "item.completed":
				this.renderItem(event.type, event.item);
				break;
			default:
				this.flushAgent();
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
			this.write(`  ? ${eventType}`);
			return;
		}

		if (item.type === "agent_message") {
			if (typeof item.text === "string" && item.text.trim()) {
				this.flushAgent();
				this.pendingAgent = item.text;
			}
			return;
		}

		if (item.type === "command_execution") {
			this.flushAgent();
			this.renderCommand(eventType, item);
			return;
		}

		if (item.type === "file_change") {
			this.flushAgent();
			if (eventType === "item.completed") this.renderFileChange(item);
			return;
		}

		if (item.type === "todo_list") {
			this.flushAgent();
			this.renderTodoList(item);
			return;
		}

		if (item.type === "reasoning") {
			if (typeof item.text === "string" && item.text.trim()) {
				this.flushAgent();
				if (this.verbose) this.writeBlock("think", item.text);
				else this.write(`  think: ${firstLine(item.text)}`);
			}
			return;
		}

		this.flushAgent();
		const keys = itemKeys(item);
		if (this.verbose) this.writeBlock(`? ${item.type}${keys ? ` {${keys}}` : ""}`, JSON.stringify(item, null, 2));
		else this.write(`  ? ${item.type}${keys ? ` {${keys}}` : ""}`);
	}

	renderCommand(eventType, item) {
		const command = firstLine(item.command ?? "command", this.verbose ? 240 : 120);
		if (eventType === "item.started") {
			this.write(`  ▸ ${command}`);
			return;
		}

		const code = item.exit_code;
		const status = typeof code === "number" ? code : item.status ?? "?";
		const mark = code === 0 ? "✓" : typeof code === "number" ? "✗" : "?";
		this.write(`  ${mark} ${command} (${status})`);
		if (this.verbose && typeof item.aggregated_output === "string" && item.aggregated_output.trim()) {
			this.writeBlock("output", item.aggregated_output);
		} else if (mark !== "✓" && typeof item.aggregated_output === "string" && item.aggregated_output.trim()) {
			this.write(`    ${firstLine(item.aggregated_output, OUTPUT_LIMIT)}`);
		}
	}

	renderFileChange(item) {
		const changes = Array.isArray(item.changes) ? item.changes : [];
		if (changes.length === 0) {
			this.write("  ✎ file change");
			return;
		}
		for (const change of changes) {
			if (!isObject(change)) continue;
			const kind = typeof change.kind === "string" ? change.kind : "change";
			this.write(`  ✎ ${kind} ${relativeSandboxPath(change.path)}`);
		}
	}

	renderTodoList(item) {
		const items = Array.isArray(item.items) ? item.items : [];
		const total = items.length;
		const done = items.filter((entry) => isObject(entry) && entry.completed === true).length;
		const active = items.find((entry) => isObject(entry) && entry.completed !== true);
		const activeText = isObject(active) && typeof active.text === "string" ? firstLine(active.text, 90) : "";
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
	const renderer = new CompactRenderer({ out: process.stderr, suppressFinalAgent: true });
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

	renderer.discardAgent();
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

function finalTextFor(jsonlPath) {
	const sibling = readOptional(join(dirname(jsonlPath), "final.md"));
	if (sibling) return sibling;
	return undefined;
}

function isTerminalEvent(event) {
	return isObject(event) && ["turn.completed", "turn.failed", "error"].includes(event.type);
}

function threadIdFor(events) {
	const started = events.find((event) => isObject(event) && event.type === "thread.started");
	return isObject(started) && typeof started.thread_id === "string" ? started.thread_id : undefined;
}

function statusFor(jsonlPath) {
	return readOptional(join(dirname(jsonlPath), "exit-status"));
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
	return contextSummaryForFile(join(dirname(jsonlPath), "rollout.jsonl"));
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
		suppressFinalAgent: Boolean(finalText) && !options.verbose,
		verbose: options.verbose,
	});
	for (const event of events) renderer.renderEvent(event);
	if (finalText && !options.verbose) renderer.discardAgent();
	else renderer.flushAgent();

	process.stdout.write("\n── grinder ──\n");
	if (status) process.stdout.write(`status: ${status}\n`);
	if (state) process.stdout.write(`state: ${state}\n`);
	if (contextSummary) process.stdout.write(`context: ${contextSummary}\n`);
	process.stdout.write(`file: ${jsonlPath}\n`);
	if (threadId) process.stdout.write(`thread: ${threadId}\n`);
	if (finalText) process.stdout.write(`final: ${firstLine(finalText, 180)}\n`);
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
			renderer = new CompactRenderer({ out: process.stdout, suppressFinalAgent: false, verbose: options.verbose });
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
			renderer = new CompactRenderer({ out: process.stdout, suppressFinalAgent: false, verbose: options.verbose });
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
