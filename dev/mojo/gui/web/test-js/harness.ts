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

/** Assert that a value is NaN (since NaN !== NaN, strict equality won't work). */
export const assertNaN = (actual: number, label: string): void => {
	if (Number.isNaN(actual)) {
		passed++;
		console.log(`    ✓ ${label}`);
	} else {
		failed++;
		console.log(
			`    ✗ ${label}\n      expected: NaN\n      actual:   ${actual}`,
		);
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
