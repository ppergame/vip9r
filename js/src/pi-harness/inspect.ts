import { closeSync, existsSync, openSync, readFileSync, readSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { StringDecoder } from "node:string_decoder";
import {
	type CandidateSessionFile,
	type FileEntry,
	type SessionEntry,
	discoverSessionFiles,
	discoverSessionFilesInPath,
	latestSessionFile,
	parseSessionFile,
	parseSessionJsonl,
} from "./session-log";
import { formatEntryFollow, formatSessionMarkdown } from "./session-format";

interface Options {
	repo: string;
	follow: boolean;
	list: boolean;
	compact: boolean;
	verbose: boolean;
	path?: string;
}

function usage(exitCode: number): never {
	const out = exitCode === 0 ? process.stdout : process.stderr;
	out.write(`usage: grinder inspect [options] [SESSION_JSONL_OR_DIR]

Options:
  -f, --follow     Follow appended session entries.
      --list       List discovered grinder session logs.
      --compact    Truncate large blocks in markdown output.
      --full       Do not truncate large blocks.
      --verbose    In --follow, print markdown blocks for new entries.
      --repo DIR   Repository root for auto-discovery.
  -h, --help       Show this help.
`);
	process.exit(exitCode);
}

function parseArgs(argv: string[]): Options {
	const options: Options = {
		repo: process.cwd(),
		follow: false,
		list: false,
		compact: false,
		verbose: false,
	};
	const paths: string[] = [];

	for (let i = 0; i < argv.length; i += 1) {
		const arg = argv[i]!;
		if (arg === "-h" || arg === "--help") usage(0);
		else if (arg === "-f" || arg === "--follow") options.follow = true;
		else if (arg === "--list") options.list = true;
		else if (arg === "--compact") options.compact = true;
		else if (arg === "--full") options.compact = false;
		else if (arg === "--verbose") options.verbose = true;
		else if (arg === "--repo") {
			const value = argv[++i];
			if (!value) usage(2);
			options.repo = resolve(value);
		} else if (arg.startsWith("-")) {
			process.stderr.write(`unknown option: ${arg}\n`);
			usage(2);
		} else {
			paths.push(arg);
		}
	}

	if (paths.length > 1) {
		process.stderr.write("expected at most one session path\n");
		usage(2);
	}
	options.path = paths[0];
	if (options.follow && !argv.includes("--full") && !argv.includes("--compact")) {
		options.compact = true;
	}
	return options;
}

function resolveInputPath(input: string, repo: string): CandidateSessionFile | undefined {
	const direct = discoverSessionFilesInPath(resolve(input))[0];
	if (direct) return direct;
	const fromRepo = discoverSessionFilesInPath(resolve(repo, input))[0];
	return fromRepo;
}

function resolveSession(options: Options): CandidateSessionFile | undefined {
	if (options.path) return resolveInputPath(options.path, options.repo);
	return latestSessionFile(options.repo);
}

function listSessions(repo: string): void {
	for (const candidate of discoverSessionFiles(repo)) {
		process.stdout.write(
			`${new Date(candidate.mtimeMs).toISOString()}\t${candidate.kind}\t${candidate.size}\t${candidate.path}\n`,
		);
	}
}

function printWholeSession(candidate: CandidateSessionFile, options: Options): void {
	const session = parseSessionFile(candidate.path);
	process.stdout.write(
		formatSessionMarkdown(candidate.path, session.header, session.entries, session.malformedLines, {
			compact: options.compact,
		}),
	);
}

function sleep(ms: number): Promise<void> {
	return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

function sameFile(a: CandidateSessionFile | undefined, b: CandidateSessionFile | undefined): boolean {
	return a?.path === b?.path;
}

function printFollowSnapshot(path: string, options: Options): number {
	const bytes = readFileSync(path);
	const lastNewline = bytes.lastIndexOf(0x0a);
	const completeBytes = lastNewline === -1 ? 0 : lastNewline + 1;
	const session = parseSessionJsonl(bytes.subarray(0, completeBytes).toString("utf8"));
	const entries = [{ type: "session" as const, ...session.header }, ...session.entries].slice(-30);
	for (const entry of entries) {
		if (options.verbose) {
			if (entry.type === "session") continue;
			const rendered = formatSessionMarkdown(path, undefined, [entry as SessionEntry], 0, { compact: options.compact });
			process.stdout.write(rendered.replace(/^# grinder session\n\n- file: .+\n\n/s, ""));
		} else {
			for (const line of formatEntryFollow(entry)) {
				process.stdout.write(`${line}\n`);
			}
		}
	}
	return completeBytes;
}

function printFollowLine(path: string, line: string, options: Options): void {
	if (!line.trim()) return;
	let entry: unknown;
	try {
		entry = JSON.parse(line);
	} catch {
		process.stderr.write(`skipping malformed session line from ${path}\n`);
		return;
	}
	if (!entry || typeof entry !== "object" || !("type" in entry) || typeof entry.type !== "string") return;
	if (options.verbose && entry.type !== "session") {
		const rendered = formatSessionMarkdown(path, undefined, [entry as SessionEntry], 0, { compact: options.compact });
		process.stdout.write(rendered.replace(/^# grinder session\n\n- file: .+\n\n/s, ""));
		return;
	}
	for (const out of formatEntryFollow(entry as FileEntry)) {
		process.stdout.write(`${out}\n`);
	}
}

function readFileRange(path: string, start: number, length: number): Buffer {
	const buffer = Buffer.allocUnsafe(length);
	const fd = openSync(path, "r");
	try {
		const bytesRead = readSync(fd, buffer, 0, length, start);
		return buffer.subarray(0, bytesRead);
	} finally {
		closeSync(fd);
	}
}

async function follow(options: Options): Promise<void> {
	let current: CandidateSessionFile | undefined;
	let offset = 0;
	let buffered = "";
	let decoder = new StringDecoder("utf8");
	let warnedWaiting = false;

	for (;;) {
		let next = options.path ? resolveSession(options) : latestSessionFile(options.repo);
		if (
			!options.path &&
			current?.kind === "live" &&
			next?.kind !== "live" &&
			existsSync(current.path)
		) {
			next = current;
		}
		if (!next) {
			if (!warnedWaiting) {
				process.stderr.write("waiting for grinder session log...\n");
				warnedWaiting = true;
			}
			await sleep(1000);
			continue;
		}

		if (!sameFile(current, next)) {
			current = next;
			buffered = "";
			decoder = new StringDecoder("utf8");
			process.stderr.write(`==> ${current.path} <==\n`);
			offset = existsSync(current.path) ? printFollowSnapshot(current.path, options) : 0;
		}

		if (!current || !existsSync(current.path)) {
			await sleep(1000);
			continue;
		}

		const stat = statSync(current.path);
		if (stat.size < offset) {
			offset = 0;
			buffered = "";
			decoder = new StringDecoder("utf8");
		}

		if (stat.size > offset) {
			const bytes = readFileRange(current.path, offset, stat.size - offset);
			const chunk = decoder.write(bytes);
			offset += bytes.length;
			buffered += chunk;
			const lines = buffered.split("\n");
			buffered = lines.pop() ?? "";
			for (const line of lines) {
				printFollowLine(current.path, line, options);
			}
		}

		await sleep(1000);
	}
}

async function main(): Promise<number> {
	const options = parseArgs(process.argv.slice(2));
	if (options.list) {
		listSessions(options.repo);
		return 0;
	}
	if (options.follow) {
		await follow(options);
		return 0;
	}

	const candidate = resolveSession(options);
	if (!candidate) {
		process.stderr.write(`no grinder session logs found under ${resolve(options.repo, "temp")}\n`);
		return 1;
	}
	printWholeSession(candidate, options);
	return 0;
}

main().then(
	(code) => {
		process.exitCode = code;
	},
	(error: unknown) => {
		process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
		process.exitCode = 1;
	},
);
