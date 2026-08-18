import { instantiate } from "../runtime/mod.ts";
import { testAllocator, testAllocatorReuse } from "./allocator.test.ts";
import { testBatchDemo } from "./batch_demo.test.ts";
import { testBench } from "./bench.test.ts";
import { testChildComponent } from "./child_component.test.ts";
import { testChildContext } from "./child_context.test.ts";
import { testConformance } from "./conformance.test.ts";
import { testContext } from "./context.test.ts";
import { testCounter } from "./counter.test.ts";
import { testDataLoader } from "./data_loader.test.ts";
import { testDsl } from "./dsl.test.ts";
import { testEffect } from "./effect.test.ts";
import { testEffectDemo } from "./effect_demo.test.ts";
import { testEffectMemo } from "./effect_memo.test.ts";
import { testEqualityDemo } from "./equality_demo.test.ts";
import { testErrorNest } from "./error_nest.test.ts";
import { testEvents } from "./events.test.ts";
import { summary } from "./harness.ts";
import { testInterpreter } from "./interpreter.test.ts";
import { testLifecycle } from "./lifecycle.test.ts";
import { testMemo } from "./memo.test.ts";
import { testMemoChain } from "./memo_chain.test.ts";
import { testMemoForm } from "./memo_form.test.ts";
import { testMutations } from "./mutations.test.ts";
import { testPhase8 } from "./phase8.test.ts";
import { testPropsCounter } from "./props_counter.test.ts";
import { testProtocol } from "./protocol.test.ts";
import { testRouting } from "./routing.test.ts";
import { testSafeCounter } from "./safe_counter.test.ts";
import { testSuspenseNest } from "./suspense_nest.test.ts";
import { testThemeCounter } from "./theme_counter.test.ts";
import { testTodo } from "./todo.test.ts";

async function run(): Promise<void> {
	const wasmPath = new URL("../build/out.wasm", import.meta.url);
	const fns = await instantiate(wasmPath);

	console.log("mojo-wasm JS runtime tests\n");

	testAllocator();
	testProtocol(fns);
	testMutations(fns);
	testInterpreter(fns);
	testCounter(fns);
	testTodo(fns);
	testPhase8(fns);
	testBench(fns);
	testMemo(fns);
	testEffect(fns);
	testDsl(fns);
	testEvents(fns);
	testAllocatorReuse(fns);
	testLifecycle(fns);
	testChildComponent(fns);
	testContext(fns);
	testChildContext(fns);
	testPropsCounter(fns);
	testThemeCounter(fns);
	testSafeCounter(fns);
	testErrorNest(fns);
	testDataLoader(fns);
	testSuspenseNest(fns);
	testEffectDemo(fns);
	testEffectMemo(fns);
	testMemoForm(fns);
	testMemoChain(fns);
	testEqualityDemo(fns);
	testBatchDemo(fns);
	testRouting(fns);
	testConformance(fns);

	summary();
}

run().catch((err: unknown) => {
	console.error("Fatal error:", err);
	Deno.exit(2);
});
