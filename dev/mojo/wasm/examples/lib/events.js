// EventBridge — Automatic event listener → WASM dispatch wiring.
//
// When the Interpreter processes a NewEventListener mutation, the bridge
// uses the handler ID from the mutation protocol to create a DOM event
// listener that calls the app's `dispatch` callback.  No manual
// handler-to-element mapping is needed.
//
// Usage:
//
//   const bridge = new EventBridge(interp, (handlerId, eventName, domEvent) => {
//     fns.app_handle_event(appPtr, handlerId, 0);
//     const len = fns.app_flush(appPtr, bufPtr, capacity);
//     if (len > 0) applyMutations(interp, bufPtr, len);
//   });
//
// The dispatch callback signature is:
//   (handlerId: number, eventName: string, domEvent: Event) => void

/**
 * EventBridge — Wires interpreter event mutations to a WASM dispatch callback.
 *
 * Hooks into `interpreter.onNewListener` so that every `NewEventListener`
 * mutation automatically produces a DOM listener that invokes `dispatch`
 * with the handler ID, event name, and raw DOM event.
 */
export class EventBridge {
	/**
	 * @param {import('./interpreter.js').Interpreter} interpreter
	 * @param {(handlerId: number, eventName: string, domEvent: Event) => void} dispatch
	 */
	constructor(interpreter, dispatch) {
		this.dispatch = dispatch;
		interpreter.onNewListener = (_elementId, eventName, handlerId) => {
			return (domEvent) => {
				this.dispatch(handlerId, eventName, domEvent);
			};
		};
	}
}
