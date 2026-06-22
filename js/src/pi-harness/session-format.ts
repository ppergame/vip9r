import type {
	AgentMessage,
	ContentBlock,
	FileEntry,
	SessionEntry,
	SessionHeader,
	SessionMessageEntry,
	ToolCallContent,
} from "./session-log";

export interface FormatOptions {
	compact?: boolean;
}

const COMPACT_LIMIT = 2000;
const FOLLOW_TEXT_LIMIT = 140;

function isObject(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null;
}

function isToolCall(block: ContentBlock): block is ToolCallContent {
	return block.type === "toolCall" && typeof (block as ToolCallContent).name === "string";
}

function textFromContent(content: unknown): string {
	if (typeof content === "string") return content;
	if (!Array.isArray(content)) return "";
	return content
		.flatMap((block) => {
			if (!isObject(block) || typeof block.type !== "string") return [];
			if (block.type === "text" && typeof block.text === "string") return [block.text];
			if (block.type === "image") return [`[image ${typeof block.mimeType === "string" ? block.mimeType : "unknown"}]`];
			return [];
		})
		.join("\n");
}

function firstLine(text: string, limit = FOLLOW_TEXT_LIMIT): string {
	const line = text.replace(/\s+/g, " ").trim();
	if (line.length <= limit) return line;
	return `${line.slice(0, limit - 1)}…`;
}

function fenced(lang: string, text: string): string[] {
	const fence = text.includes("```") ? "````" : "```";
	return [`${fence}${lang}`, text, fence];
}

function maybeCompact(text: string, options: FormatOptions): string {
	if (!options.compact || text.length <= COMPACT_LIMIT) return text;
	const omitted = text.length - COMPACT_LIMIT;
	return `${text.slice(0, COMPACT_LIMIT)}\n\n[... ${omitted} chars omitted; use --full to show all ...]`;
}

function json(value: unknown, options: FormatOptions): string {
	return maybeCompact(JSON.stringify(value, null, 2), options);
}

function timestamp(entry: { timestamp?: string } | undefined): string {
	return entry?.timestamp ? ` (${entry.timestamp})` : "";
}

function stringField(value: unknown, key: string): string | undefined {
	if (!isObject(value)) return undefined;
	const field = value[key];
	return typeof field === "string" ? field : undefined;
}

function booleanField(value: unknown, key: string): boolean | undefined {
	if (!isObject(value)) return undefined;
	const field = value[key];
	return typeof field === "boolean" ? field : undefined;
}

function formatToolArgsForHeading(tool: ToolCallContent): string {
	const args = tool.arguments ?? {};
	const command = typeof args.command === "string" ? args.command : undefined;
	const path = typeof args.path === "string" ? args.path : undefined;
	const pattern = typeof args.pattern === "string" ? args.pattern : undefined;
	const hint = command ?? pattern ?? path;
	return hint ? ` ${firstLine(hint, 100)}` : "";
}

function formatToolCallMarkdown(tool: ToolCallContent, options: FormatOptions): string[] {
	const lines = [`### tool call: ${tool.name}${formatToolArgsForHeading(tool)}`];
	const args = tool.arguments ?? {};
	if (tool.name === "bash" && typeof args.command === "string") {
		lines.push("", ...fenced("sh", args.command));
	} else {
		lines.push("", ...fenced("json", json(args, options)));
	}
	return lines;
}

function formatToolResultMarkdown(message: AgentMessage, options: FormatOptions): string[] {
	const toolName = stringField(message, "toolName") ?? "tool";
	const status = booleanField(message, "isError") ? "error" : "ok";
	const text = maybeCompact(textFromContent(message.content), options);
	const lines = [`### tool result: ${toolName} ${status}`];
	if (text) lines.push("", ...fenced("", text));
	return lines;
}

function formatMessageMarkdown(entry: SessionMessageEntry, options: FormatOptions): string[] {
	const message = entry.message;
	if (message.role === "user") {
		const text = maybeCompact(textFromContent(message.content), options);
		return [`## user${timestamp(entry)}`, "", text];
	}
	if (message.role === "toolResult") {
		return formatToolResultMarkdown(message, options);
	}
	if (message.role === "assistant") {
		const lines = [`## assistant${timestamp(entry)}`];
		const content = Array.isArray(message.content) ? message.content : [];
		let emitted = false;
		for (const block of content) {
			if (block.type === "text" && typeof block.text === "string" && block.text.trim()) {
				lines.push("", maybeCompact(block.text.trimEnd(), options));
				emitted = true;
			} else if (isToolCall(block)) {
				lines.push("", ...formatToolCallMarkdown(block, options));
				emitted = true;
			} else if (block.type === "thinking") {
				continue;
			}
		}
		if (!emitted) lines.push("", "[assistant produced no visible text]");
		return lines;
	}

	return [`## ${message.role ?? "message"}${timestamp(entry)}`, "", ...fenced("json", json(message, options))];
}

export function formatSessionMarkdown(
	path: string,
	header: SessionHeader | undefined,
	entries: SessionEntry[],
	malformedLines: number,
	options: FormatOptions = {},
): string {
	const lines = ["# grinder session", "", `- file: ${path}`];
	if (header?.id) lines.push(`- id: ${header.id}`);
	if (header?.timestamp) lines.push(`- started: ${header.timestamp}`);
	if (header?.cwd) lines.push(`- cwd: ${header.cwd}`);
	if (malformedLines > 0) lines.push(`- malformed lines skipped: ${malformedLines}`);
	lines.push("");

	for (const entry of entries) {
		if (entry.type === "message") {
			lines.push(...formatMessageMarkdown(entry as SessionMessageEntry, options), "");
		} else if (entry.type === "model_change") {
			const provider = stringField(entry, "provider") ?? "unknown-provider";
			const model = stringField(entry, "modelId") ?? "unknown-model";
			lines.push(`## model${timestamp(entry)}`, "", `${provider}/${model}`, "");
		} else if (entry.type === "thinking_level_change") {
			const level = stringField(entry, "thinkingLevel") ?? "unknown";
			lines.push(`## thinking level${timestamp(entry)}`, "", level, "");
		} else if (entry.type === "session_info") {
			const name = stringField(entry, "name") ?? "";
			lines.push(`## session info${timestamp(entry)}`, "", name, "");
		} else {
			lines.push(`## ${entry.type}${timestamp(entry)}`, "", ...fenced("json", json(entry, options)), "");
		}
	}

	return `${lines.join("\n").replace(/\n{4,}/g, "\n\n\n").trimEnd()}\n`;
}

function followTime(entry: FileEntry): string {
	const raw = "timestamp" in entry && typeof entry.timestamp === "string" ? entry.timestamp : undefined;
	return raw ? raw.slice(11, 19) : "--:--:--";
}

export function formatEntryFollow(entry: FileEntry): string[] {
	if (entry.type === "session") {
		const cwd = stringField(entry, "cwd");
		return [`${followTime(entry)} session ${entry.id ?? ""}${cwd ? ` cwd=${cwd}` : ""}`.trimEnd()];
	}
	if (entry.type === "model_change") {
		const provider = stringField(entry, "provider") ?? "unknown-provider";
		const model = stringField(entry, "modelId") ?? "unknown-model";
		return [`${followTime(entry)} model ${provider}/${model}`];
	}
	if (entry.type === "thinking_level_change") {
		const level = stringField(entry, "thinkingLevel") ?? "unknown";
		return [`${followTime(entry)} thinking ${level}`];
	}
	if (entry.type !== "message") {
		return [`${followTime(entry)} ${entry.type}`];
	}

	const message = (entry as SessionMessageEntry).message;
	if (message.role === "user") {
		return [`${followTime(entry)} user ${firstLine(textFromContent(message.content))}`];
	}
	if (message.role === "toolResult") {
		const text = textFromContent(message.content);
		const toolName = typeof message.toolName === "string" ? message.toolName : "tool";
		const status = message.isError ? "error" : "ok";
		const detail = firstLine(text);
		return [`${followTime(entry)} ${toolName} ${status} ${text.length} chars${detail ? `: ${detail}` : ""}`];
	}
	if (message.role === "assistant") {
		const lines: string[] = [];
		const content = Array.isArray(message.content) ? message.content : [];
		for (const block of content) {
			if (block.type === "text" && typeof block.text === "string" && block.text.trim()) {
				lines.push(`${followTime(entry)} assistant ${firstLine(block.text)}`);
			} else if (isToolCall(block)) {
				lines.push(`${followTime(entry)} assistant → ${block.name}${formatToolArgsForHeading(block)}`);
			}
		}
		return lines.length > 0 ? lines : [`${followTime(entry)} assistant [no visible text]`];
	}

	return [`${followTime(entry)} ${message.role ?? "message"}`];
}
