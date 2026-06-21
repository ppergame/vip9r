import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
	AuthStorage,
	DefaultResourceLoader,
	ModelRegistry,
	SessionManager,
	SettingsManager,
	createAgentSession,
} from "@earendil-works/pi-coding-agent";

const CODEX_PROVIDER = "openai-codex";
const CODEX_MODEL = "gpt-5.5";
const THINKING_LEVEL = "xhigh" as const;
const ALL_BUILT_IN_PI_TOOLS = ["read", "bash", "edit", "write", "grep", "find", "ls"] as const;
const RUN_ROOT = "/run";
const WORK_DIR = `${RUN_ROOT}/rust`;
const TASK_PATH = `${RUN_ROOT}/task.md`;
const SYSTEM_PROMPT_PATH = `${RUN_ROOT}/system.md`;
const TRACE_DIR = `${RUN_ROOT}/trace`;
const SESSION_DIR = `${TRACE_DIR}/pi-sessions`;
const AGENT_DIR = `${RUN_ROOT}/home/.pi-harness`;
const AUTH_PATH = "/auth/auth.json";

type AssistantLike = {
	role?: unknown;
	stopReason?: unknown;
	errorMessage?: unknown;
	content?: unknown;
};

type TraceSession = {
	sessionFile: string | undefined;
};

function readRequiredFile(path: string, label: string): string {
	const text = readFileSync(path, "utf8");
	if (!text.trim()) {
		throw new Error(`${label} is empty: ${path}`);
	}
	return text;
}

function readPrompt(): string {
	return readRequiredFile(TASK_PATH, "task prompt");
}

function finalAssistantText(messages: unknown[]): { text?: string; error?: string } {
	const message = [...messages].reverse().find((candidate): candidate is AssistantLike => {
		return typeof candidate === "object" && candidate !== null && (candidate as AssistantLike).role === "assistant";
	});
	if (!message) {
		return { error: "No assistant message was produced" };
	}
	if (message.stopReason === "error" || message.stopReason === "aborted") {
		return {
			error:
				typeof message.errorMessage === "string" && message.errorMessage.length > 0
					? message.errorMessage
					: `Request ${String(message.stopReason)}`,
		};
	}
	if (!Array.isArray(message.content)) {
		return { error: "Assistant message had no content array" };
	}

	const text = message.content
		.filter((content): content is { type: "text"; text: string } => {
			return (
				typeof content === "object" &&
				content !== null &&
				(content as { type?: unknown }).type === "text" &&
				typeof (content as { text?: unknown }).text === "string"
			);
		})
		.map((content) => content.text)
		.join("\n")
		.trimEnd();

	return text ? { text } : { error: "Assistant message contained no text" };
}

function errorText(error: unknown): string {
	if (error instanceof Error) {
		return error.stack ?? error.message;
	}
	return String(error);
}

function writeSessionTrace(session: TraceSession, error: string | undefined): void {
	mkdirSync(TRACE_DIR, { recursive: true });

	if (session.sessionFile) {
		try {
			copyFileSync(session.sessionFile, `${TRACE_DIR}/session.jsonl`);
		} catch (copyError) {
			process.stderr.write(`session.jsonl: ${errorText(copyError)}\n`);
		}
	}

	if (error) {
		writeFileSync(`${TRACE_DIR}/error.txt`, `${error}\n`);
	}
}

async function main(): Promise<number> {
	const prompt = readPrompt();
	const systemPrompt = readRequiredFile(SYSTEM_PROMPT_PATH, "system prompt");
	const cwd = WORK_DIR;

	const authStorage = AuthStorage.create(AUTH_PATH);
	const authError = authStorage.drainErrors()[0];
	if (authError) {
		throw new Error(`${AUTH_PATH}: ${authError.message}`);
	}
	if (!authStorage.has(CODEX_PROVIDER)) {
		throw new Error(`Missing Codex credentials in ${AUTH_PATH}`);
	}

	const modelRegistry = ModelRegistry.inMemory(authStorage);

	const settingsManager = SettingsManager.inMemory({
		defaultProvider: CODEX_PROVIDER,
		defaultModel: CODEX_MODEL,
		transport: "auto",
		quietStartup: true,
	});

	mkdirSync(SESSION_DIR, { recursive: true });
	const sessionManager = SessionManager.create(cwd, SESSION_DIR);

	const resourceLoader = new DefaultResourceLoader({
		cwd,
		agentDir: AGENT_DIR,
		settingsManager,
		noExtensions: true,
		noSkills: true,
		noPromptTemplates: true,
		noThemes: true,
		noContextFiles: true,
		systemPrompt,
	});
	await resourceLoader.reload();

	const model = modelRegistry.find(CODEX_PROVIDER, CODEX_MODEL);
	if (!model) {
		throw new Error(`Pi model not found: ${CODEX_PROVIDER}/${CODEX_MODEL}`);
	}

	const { session } = await createAgentSession({
		cwd,
		agentDir: AGENT_DIR,
		authStorage,
		modelRegistry,
		settingsManager,
		sessionManager,
		resourceLoader,
		model,
		thinkingLevel: THINKING_LEVEL,
		tools: [...ALL_BUILT_IN_PI_TOOLS],
	});

	let turn = 0;
	const unsubscribe = session.subscribe((event) => {
		if (event.type === "turn_start") {
			process.stderr.write(`· turn ${(turn += 1)}\n`);
			return;
		}
		if (event.type === "tool_execution_start") {
			const args = (event.args ?? {}) as Record<string, unknown>;
			const hint = String(args.command ?? args.pattern ?? args.path ?? "")
				.split("\n", 1)[0]
				.slice(0, 80);
			process.stderr.write(`  ▸ ${event.toolName}${hint ? ` ${hint}` : ""}\n`);
		}
	});

	let agentError: string | undefined;
	try {
		await session.prompt(prompt);
		const result = finalAssistantText(session.state.messages);
		if (result.error) {
			agentError = result.error;
			process.stderr.write(`${result.error}\n`);
			return 1;
		}
		process.stdout.write(`${result.text}\n`);
		return 0;
	} catch (error) {
		agentError = errorText(error);
		throw error;
	} finally {
		unsubscribe();
		try {
			writeSessionTrace(session, agentError);
		} catch (error) {
			process.stderr.write(`session trace: ${errorText(error)}\n`);
		}
		session.dispose();
		await settingsManager.flush();
	}
}

main().then(
	(exitCode) => {
		process.exitCode = exitCode;
	},
	(error: unknown) => {
		process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
		process.exitCode = 1;
	},
);
