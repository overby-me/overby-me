import { compare } from "compare-versions";
import platform from "platform";
import { startTransition } from "react";

const checkVersion = () => {
	switch (platform.layout) {
		case "Gecko":
			if (compare(platform.version ?? "0", "94", "<=")) return false;
			return true;
		case "Blink":
			if (
				compare(platform.version ?? "0", "98", "<=") &&
				platform.name !== "Opera"
			)
				return false;
			return true;
		case "WebKit":
			if (compare(platform.version ?? "0", "15.4", "<=")) return false;
			return true;
		default:
			return true;
	}
};

/**
 * Fire a (possibly async) GQty mutation as a transition.
 *
 * The client enables `mutationSuspense`, so an in-flight mutation suspends the
 * calling component. If that happens in response to synchronous input (a click
 * handler that is not a transition) React throws error #426 and snaps the nearest
 * already-revealed Suspense boundary to its fallback.
 *
 * This calls `fn()` synchronously inside `startTransition` so the in-flight
 * suspense is marked non-urgent (old UI stays visible), while still returning the
 * promise so callers can `await` it for sequencing / `awaitRefetchQueries`.
 *
 * Note: you cannot wrap an `async` function body in `startTransition` directly —
 * only updates scheduled before the first `await` are part of the transition.
 * Wrap each mutate call with this helper instead.
 */
const txMutate = <T>(fn: () => Promise<T>): Promise<T> => {
	let promise!: Promise<T>;
	startTransition(() => {
		promise = fn();
	});
	return promise;
};

export { checkVersion, txMutate };
