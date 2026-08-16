const encoder = new TextEncoder();

// --- Shared test state ---

export let passed = 0;
export let failed = 0;

/** Print a suite header. */
export const suite = (name: string): void => {
	console.log(`\n  ${name}`);
};

/** Assert strict equality between actual and expected values. */
export const assert = <T>(actual: T, expected: T, label: string): void => {
	if (actual === expected) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: ${JSON.stringify(expected)}\n      actual:   ${JSON.stringify(actual)}`,
		);
	}
};

/** Assert that a condition is true. */
export const assertTrue = (condition: boolean, label: string): void => {
	if (condition) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(`    ✗ ${label}\n      expected: true\n      actual:   false`);
	}
};

/** Assert that a condition is false. */
export const assertFalse = (condition: boolean, label: string): void => {
	if (!condition) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(`    ✗ ${label}\n      expected: false\n      actual:   true`);
	}
};

/** Assert that a value is not null/undefined. */
export const assertDefined = <T>(
	value: T | null | undefined,
	label: string,
): void => {
	if (value !== null && value !== undefined) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: defined\n      actual:   ${value}`,
		);
	}
};

/** Assert that a value is null or undefined. */
export const assertNull = (value: unknown, label: string): void => {
	if (value === null || value === undefined) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: null/undefined\n      actual:   ${JSON.stringify(value)}`,
		);
	}
};

/** Assert that a numeric value is within ±epsilon of the expected value. */
export const assertClose = (
	actual: number,
	expected: number,
	epsilon: number,
	label: string,
): void => {
	if (Math.abs(actual - expected) < epsilon) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: ≈${expected} (±${epsilon})\n      actual:   ${actual}`,
		);
	}
};

/** Assert that a numeric value is greater than a threshold. */
export const assertGreater = (
	actual: number,
	threshold: number,
	label: string,
): void => {
	if (actual > threshold) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: > ${threshold}\n      actual:   ${actual}`,
		);
	}
};

/** Assert that a numeric value is greater than or equal to a threshold. */
export const assertGte = (
	actual: number,
	threshold: number,
	label: string,
): void => {
	if (actual >= threshold) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: >= ${threshold}\n      actual:   ${actual}`,
		);
	}
};

/** Assert that an array has an expected length. */
export const assertLength = (
	arr: { length: number },
	expected: number,
	label: string,
): void => {
	if (arr.length === expected) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected length: ${expected}\n      actual length:   ${arr.length}`,
		);
	}
};

/** Assert that a function throws an error. */
export const assertThrows = (fn: () => void, label: string): void => {
	try {
		fn();
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: throw\n      actual:   no throw`,
		);
	} catch {
		passed++;
		console.log(`    ✓ ${label}`);
	}
};

/** Assert that an async function throws an error. */
export const assertThrowsAsync = async (
	fn: () => Promise<void>,
	label: string,
): Promise<void> => {
	try {
		await fn();
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: throw\n      actual:   no throw`,
		);
	} catch {
		passed++;
		console.log(`    ✓ ${label}`);
	}
};

/** Increment the passed counter (for tests that only check "no throw"). */
export const pass = (count = 1): void => {
	passed += count;
};

/** Write raw bytes to stdout (for prefixing print-function output). */
export const writeStdout = (text: string): void => {
	Deno.stdout.writeSync(encoder.encode(text));
};

/** Print the final summary line and exit with appropriate code. */
export const summary = (): void => {
	console.log(
		`\n  ${passed + failed} tests: ${passed} passed, ${failed} failed\n`,
	);
	Deno.exit(failed > 0 ? 1 : 0);
};
