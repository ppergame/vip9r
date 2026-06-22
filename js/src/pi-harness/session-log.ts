import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

export interface SessionHeader {
	type: "session";
	version?: number;
	id?: string;
	timestamp?: string;
	cwd?: string;
	parentSession?: string;
}

export interface SessionEntryBase {
	type: string;
	id?: string;
	parentId?: string | null;
	timestamp?: string;
}

export interface SessionMessageEntry extends SessionEntryBase {
	type: "message";
	message: AgentMessage;
}

export interface ModelChangeEntry extends SessionEntryBase {
	type: "model_change";
	provider?: string;
	modelId?: string;
}

export interface ThinkingLevelChangeEntry extends SessionEntryBase {
	type: "thinking_level_change";
	thinkingLevel?: string;
}

export interface SessionInfoEntry extends SessionEntryBase {
	type: "session_info";
	name?: string;
}

export type SessionEntry = SessionMessageEntry | ModelChangeEntry | ThinkingLevelChangeEntry | SessionInfoEntry | SessionEntryBase;
export type FileEntry = SessionHeader | SessionEntry;

export interface TextContent {
	type: "text";
	text: string;
}

export interface ThinkingContent {
	type: "thinking";
	thinking?: string;
	thinkingSignature?: string;
	redacted?: boolean;
}

export interface ImageContent {
	type: "image";
	data?: string;
	mimeType?: string;
}

export interface ToolCallContent {
	type: "toolCall";
	id: string;
	name: string;
	arguments: Record<string, unknown>;
}

export type ContentBlock = TextContent | ThinkingContent | ImageContent | ToolCallContent | { type?: string; [key: string]: unknown };

export interface UserMessage {
	role: "user";
	content: string | ContentBlock[];
	timestamp?: number;
}

export interface AssistantMessage {
	role: "assistant";
	content: ContentBlock[];
	api?: string;
	provider?: string;
	model?: string;
	usage?: unknown;
	stopReason?: string;
	errorMessage?: string;
	timestamp?: number;
}

export interface ToolResultMessage {
	role: "toolResult";
	toolCallId?: string;
	toolName?: string;
	content: ContentBlock[];
	details?: unknown;
	isError?: boolean;
	timestamp?: number;
}

export type AgentMessage = UserMessage | AssistantMessage | ToolResultMessage | { role?: string; [key: string]: unknown };

export interface SessionFile {
	header?: SessionHeader;
	entries: SessionEntry[];
	malformedLines: number;
}

export interface CandidateSessionFile {
	path: string;
	size: number;
	mtimeMs: number;
	kind: "live" | "completed" | "preserved";
}

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function toFileEntry(value: unknown): FileEntry | undefined {
	if (!isObject(value) || typeof value.type !== "string") return undefined;
	return value as unknown as FileEntry;
}

export function parseSessionJsonl(text: string): SessionFile {
	const entries: SessionEntry[] = [];
	let header: SessionHeader | undefined;
	let malformedLines = 0;

	for (const line of text.trim().split("\n")) {
		if (!line.trim()) continue;
		let parsed: unknown;
		try {
			parsed = JSON.parse(line);
		} catch {
			malformedLines += 1;
			continue;
		}

		const entry = toFileEntry(parsed);
		if (!entry) {
			malformedLines += 1;
			continue;
		}
		if (entry.type === "session") {
			header = entry as SessionHeader;
		} else {
			entries.push(entry as SessionEntry);
		}
	}

	return { header, entries, malformedLines };
}

export function parseSessionFile(path: string): SessionFile {
	return parseSessionJsonl(readFileSync(path, "utf8"));
}

function statCandidate(path: string, kind: CandidateSessionFile["kind"]): CandidateSessionFile | undefined {
	try {
		const stat = statSync(path);
		if (!stat.isFile()) return undefined;
		return { path, kind, size: stat.size, mtimeMs: stat.mtimeMs };
	} catch {
		return undefined;
	}
}

function listDir(path: string): string[] {
	try {
		return readdirSync(path);
	} catch {
		return [];
	}
}

function collectJsonlFiles(dir: string, kind: CandidateSessionFile["kind"], out: CandidateSessionFile[]): void {
	for (const name of listDir(dir)) {
		if (!name.endsWith(".jsonl")) continue;
		const candidate = statCandidate(join(dir, name), kind);
		if (candidate) out.push(candidate);
	}
}

export function discoverSessionFiles(repo: string): CandidateSessionFile[] {
	const root = resolve(repo);
	const tempDir = join(root, "temp");
	const candidates: CandidateSessionFile[] = [];

	for (const name of listDir(tempDir)) {
		if (!name.startsWith("grinder.")) continue;
		const traceDir = join(tempDir, name, "trace");
		const completed = statCandidate(join(traceDir, "session.jsonl"), "completed");
		if (completed) candidates.push(completed);
		collectJsonlFiles(join(traceDir, "pi-sessions"), "live", candidates);
	}

	for (const name of listDir(join(tempDir, "traces"))) {
		const preserved = statCandidate(join(tempDir, "traces", name, "session.jsonl"), "preserved");
		if (preserved) candidates.push(preserved);
	}

	candidates.sort((a, b) => b.mtimeMs - a.mtimeMs || a.path.localeCompare(b.path));
	return candidates;
}

export function latestSessionFile(repo: string): CandidateSessionFile | undefined {
	return discoverSessionFiles(repo)[0];
}

export function discoverSessionFilesInPath(path: string): CandidateSessionFile[] {
	const resolved = resolve(path);
	const direct = statCandidate(resolved, "completed");
	if (direct) return [direct];

	const candidates: CandidateSessionFile[] = [];
	collectJsonlFiles(resolved, "completed", candidates);
	collectJsonlFiles(join(resolved, "pi-sessions"), "live", candidates);
	collectJsonlFiles(join(resolved, "trace", "pi-sessions"), "live", candidates);
	const completed = statCandidate(join(resolved, "trace", "session.jsonl"), "completed");
	if (completed) candidates.push(completed);
	const nested = statCandidate(join(resolved, "session.jsonl"), "completed");
	if (nested) candidates.push(nested);
	candidates.sort((a, b) => b.mtimeMs - a.mtimeMs || a.path.localeCompare(b.path));
	return candidates;
}
