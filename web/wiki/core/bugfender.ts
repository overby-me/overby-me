export let isInitialized = false;

// Cache fetched source maps to avoid re-fetching
// biome-ignore lint/suspicious/noExplicitAny: source-map-js types vary across versions
const sourceMapCache = new Map<string, any>();

export const initBugfender = async () => {
	if (
		typeof window !== "undefined" &&
		!isInitialized &&
		process.env.PUBLIC_BUGFENDER_APP_KEY
	) {
		const { Bugfender } = await import("@bugfender/sdk");
		Bugfender.init({
			appKey: process.env.PUBLIC_BUGFENDER_APP_KEY,
			// Override console methods to capture all console.log, console.error, etc.
			overrideConsoleMethods: true,
			// Print logs to browser console as well (useful for development)
			printToConsole: process.env.NODE_ENV === "development",
			// Automatically register global error handlers for unhandled errors
			registerErrorHandler: true,
			// Log browser events (page loads, navigation, etc.)
			logBrowserEvents: true,
			// Log UI events (clicks, form submissions, etc.)
			logUIEvents: true,
			build: process.env.PUBLIC_GIT_COMMIT_SHA ?? "dev",
		});

		// Enhanced error context: log URL, session state and resolved stack for uncaught errors
		window.addEventListener("error", (event) => {
			const error = event.error;
			logErrorContext({
				bugfender: Bugfender,
				handler: "Error",
				message: event.message,
				error,
				filename: event.filename,
				line: event.lineno,
				col: event.colno,
			});
		});

		// Enhanced error context: log URL, session state and resolved stack for unhandled promise rejections
		window.addEventListener("unhandledrejection", (event) => {
			const reason = event.reason;
			const message =
				reason instanceof Error ? reason.message : String(reason);
			const error = reason instanceof Error ? reason : undefined;
			logErrorContext({
				bugfender: Bugfender,
				handler: "UnhandledRejection",
				message,
				error,
			});
		});

		isInitialized = true;
	}
};

interface ParsedFrame {
	functionName: string | null;
	fileName: string;
	lineNumber: number;
	columnNumber: number;
}

const STACK_FRAME_RE =
	/^\s*at\s+(.+?)\s+\((.+?):(\d+):(\d+)\)\s*$|^\s*at\s+(.+?):(\d+):(\d+)\s*$|^(.+?)@(.+?):(\d+):(\d+)\s*$/;

const parseStack = (stack: string): ParsedFrame[] => {
	const frames: ParsedFrame[] = [];
	for (const line of stack.split("\n")) {
		const m = STACK_FRAME_RE.exec(line);
		if (!m) continue;
		if (m[1] && m[2]) {
			// "at functionName (file:line:col)"
			frames.push({
				functionName: m[1],
				fileName: m[2],
				lineNumber: Number.parseInt(m[3], 10),
				columnNumber: Number.parseInt(m[4], 10),
			});
		} else if (m[5]) {
			// "at file:line:col"
			frames.push({
				functionName: null,
				fileName: m[5],
				lineNumber: Number.parseInt(m[6], 10),
				columnNumber: Number.parseInt(m[7], 10),
			});
		} else if (m[8] && m[9]) {
			// "functionName@file:line:col" (Firefox)
			frames.push({
				functionName: m[8],
				fileName: m[9],
				lineNumber: Number.parseInt(m[10], 10),
				columnNumber: Number.parseInt(m[11], 10),
			});
		}
	}
	return frames;
};

const fetchSourceMapForUrl = async (jsUrl: string) => {
	if (sourceMapCache.has(jsUrl)) {
		return sourceMapCache.get(jsUrl) ?? null;
	}

	try {
		// First try the conventional .map URL
		const mapUrl = `${jsUrl}.map`;
		const res = await fetch(mapUrl);
		if (!res.ok) {
			sourceMapCache.set(jsUrl, null);
			return null;
		}
		const rawMap = await res.text();
		const { SourceMapConsumer } = await import("source-map-js");
		const consumer = new SourceMapConsumer(JSON.parse(rawMap));
		sourceMapCache.set(jsUrl, consumer);
		return consumer;
	} catch {
		sourceMapCache.set(jsUrl, null);
		return null;
	}
};

const resolveStack = async (stack: string): Promise<string> => {
	const frames = parseStack(stack);
	if (frames.length === 0) return stack;

	const resolvedLines: string[] = [];

	for (const frame of frames) {
		try {
			const consumer = await fetchSourceMapForUrl(frame.fileName);
			if (consumer) {
				const pos = consumer.originalPositionFor({
					line: frame.lineNumber,
					column: frame.columnNumber - 1, // source-map uses 0-based columns
				});
				if (pos.source) {
					const fn = pos.name ?? frame.functionName ?? "<anonymous>";
					resolvedLines.push(
						`  at ${fn} (${pos.source}:${pos.line}:${(pos.column ?? 0) + 1})`,
					);
					continue;
				}
			}
		} catch {
			// fall through to raw frame
		}

		const fn = frame.functionName ?? "<anonymous>";
		resolvedLines.push(
			`  at ${fn} (${frame.fileName}:${frame.lineNumber}:${frame.columnNumber})`,
		);
	}

	return resolvedLines.join("\n");
};

const logErrorContext = async ({
	bugfender,
	handler,
	message,
	error,
	filename,
	line,
	col,
}: {
	bugfender: { error: (...args: unknown[]) => void };
	handler: string;
	message: string;
	error?: Error;
	filename?: string;
	line?: number;
	col?: number;
}) => {
	let resolvedStack: string | undefined;

	if (error?.stack) {
		try {
			resolvedStack = await resolveStack(error.stack);
		} catch {
			resolvedStack = error.stack;
		}
	}

	const context = {
		url: window.location.href,
		handler,
		message,
		filename,
		line,
		col,
		stack: resolvedStack ?? error?.stack,
		session: safeGetSession(),
	};

	bugfender.error(
		`[ErrorContext] ${message}`,
		JSON.stringify(context, null, 2),
	);
};

const safeGetSession = (): unknown => {
	try {
		const raw = localStorage.getItem("session");
		return raw ? JSON.parse(raw) : null;
	} catch {
		return null;
	}
};