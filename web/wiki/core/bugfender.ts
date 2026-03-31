import StackTrace from "stacktrace-js";

export let isInitialized = false;

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

	if (error) {
		try {
			const frames = await StackTrace.fromError(error);
			const isResolved = frames.some(
				(frame) =>
					frame.fileName &&
					!frame.fileName.startsWith("http") &&
					!frame.fileName.includes("/static/js/"),
			);
			resolvedStack = frames
				.map(
					(frame) =>
						`  at ${frame.functionName ?? "<anonymous>"} (${frame.fileName}:${frame.lineNumber}:${frame.columnNumber})`,
				)
				.join("\n");
			if (!isResolved) {
				bugfender.warn(
					"[SourceMapDebug] stacktrace-js did not resolve source maps. Raw frames:",
					JSON.stringify(
						frames.map((f) => ({
							fn: f.functionName,
							file: f.fileName,
							line: f.lineNumber,
							col: f.columnNumber,
							source: f.source,
						})),
						null,
						2,
					),
				);
			}
		} catch (resolveError) {
			resolvedStack = error.stack;
			bugfender.warn(
				"[SourceMapDebug] stacktrace-js threw an error:",
				resolveError instanceof Error
					? `${resolveError.message}\n${resolveError.stack}`
					: String(resolveError),
			);
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