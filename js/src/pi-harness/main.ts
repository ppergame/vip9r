import { join } from "node:path";
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

type AssistantLike = {
	role?: unknown;
	stopReason?: unknown;
	errorMessage?: unknown;
	content?: unknown;
};

function usage(): string {
	return [
		"Usage: node dist/pi-harness/pi-harness.mjs [prompt]",
		"",
		"If no prompt argument is provided, the harness reads the prompt from stdin.",
		"",
		"Fixed configuration:",
		"  cwd: current working directory",
		"  agent dir: <cwd>/.pi-harness",
		"  auth: <cwd>/.pi-harness/auth.json",
		`  model: ${CODEX_PROVIDER}/${CODEX_MODEL}`,
		`  thinking: ${THINKING_LEVEL}`,
	].join("\n");
}

async function readPrompt(): Promise<string> {
	const args = process.argv.slice(2);
	if (args.includes("--help") || args.includes("-h")) {
		process.stdout.write(`${usage()}\n`);
		process.exit(0);
	}

	const positional = args.filter((arg) => arg !== "--");
	if (positional.length > 0) {
		return positional.join(" ");
	}

	if (process.stdin.isTTY) {
		throw new Error("No prompt provided. Pass it as argv or pipe it on stdin.");
	}

	const chunks: Buffer[] = [];
	for await (const chunk of process.stdin) {
		chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
	}
	const prompt = Buffer.concat(chunks).toString("utf8").trim();
	if (!prompt) {
		throw new Error("stdin did not contain a prompt");
	}
	return prompt;
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

async function main(): Promise<number> {
	const prompt = await readPrompt();
	const cwd = process.cwd();
	const agentDir = join(cwd, ".pi-harness");
	const authPath = join(agentDir, "auth.json");

	const authStorage = AuthStorage.create(authPath);
	const authError = authStorage.drainErrors()[0];
	if (authError) {
		throw new Error(`${authPath}: ${authError.message}`);
	}
	if (!authStorage.has(CODEX_PROVIDER)) {
		throw new Error(`Missing Codex credentials in ${authPath}`);
	}

	const modelRegistry = ModelRegistry.inMemory(authStorage);

	const settingsManager = SettingsManager.inMemory({
		defaultProvider: CODEX_PROVIDER,
		defaultModel: CODEX_MODEL,
		transport: "auto",
		quietStartup: true,
	});

	const sessionManager = SessionManager.inMemory(cwd);

	const resourceLoader = new DefaultResourceLoader({
		cwd,
		agentDir,
		settingsManager,
		noExtensions: true,
		noSkills: true,
		noPromptTemplates: true,
		noThemes: true,
		noContextFiles: false,
	});
	await resourceLoader.reload();

	const model = modelRegistry.find(CODEX_PROVIDER, CODEX_MODEL);
	if (!model) {
		throw new Error(`Pi model not found: ${CODEX_PROVIDER}/${CODEX_MODEL}`);
	}

	const { session } = await createAgentSession({
		cwd,
		agentDir,
		authStorage,
		modelRegistry,
		settingsManager,
		sessionManager,
		resourceLoader,
		model,
		thinkingLevel: THINKING_LEVEL,
		tools: [...ALL_BUILT_IN_PI_TOOLS],
	});

	try {
		await session.prompt(prompt);
		const result = finalAssistantText(session.state.messages);
		if (result.error) {
			process.stderr.write(`${result.error}\n`);
			return 1;
		}
		process.stdout.write(`${result.text}\n`);
		return 0;
	} finally {
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
