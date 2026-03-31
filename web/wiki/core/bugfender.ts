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

		// Enhanced error context: log URL, session state and stack for uncaught errors
		window.addEventListener("error", (event) => {
			const context = {
				url: window.location.href,
				handler: "Error",
				message: event.message,
				filename: event.filename,
				line: event.lineno,
				col: event.colno,
				stack: event.error?.stack,
				session: safeGetSession(),
			};
			Bugfender.error(
				`[ErrorContext] ${event.message}`,
				JSON.stringify(context, null, 2),
			);
		});

		// Enhanced error context: log URL, session state and stack for unhandled promise rejections
		window.addEventListener("unhandledrejection", (event) => {
			const reason = event.reason;
			const message = reason instanceof Error ? reason.message : String(reason);
			const stack = reason instanceof Error ? reason.stack : undefined;
			const context = {
				url: window.location.href,
				handler: "UnhandledRejection",
				message,
				stack,
				session: safeGetSession(),
			};
			Bugfender.error(
				`[ErrorContext] ${message}`,
				JSON.stringify(context, null, 2),
			);
		});

		isInitialized = true;
	}
};

const safeGetSession = (): unknown => {
	try {
		const raw = localStorage.getItem("session");
		return raw ? JSON.parse(raw) : null;
	} catch {
		return null;
	}
};
