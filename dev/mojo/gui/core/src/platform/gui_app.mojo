# GuiApp Trait — App-side lifecycle contract for unified launch() dispatch.
#
# This trait captures the lifecycle that every mojo-gui application must
# implement. It is the app-side counterpart to PlatformApp (which renderers
# implement). Together, they form the abstraction boundary that enables
# shared examples to compile and run on every target from identical source.
#
# The GuiApp trait unifies the free-function patterns currently used by
# each example app (counter_app_rebuild, counter_app_flush, etc.) into
# a single trait interface. This enables:
#
#   1. Generic desktop event loops — desktop_launch[AppType: GuiApp]()
#      drives any app without app-specific branching.
#
#   2. Generic @export WASM wrappers — a single set of @export functions
#      can drive any GuiApp via a compile-time alias.
#
#   3. The launch() entry point — launch[AppType: GuiApp]() dispatches
#      to the correct renderer at compile time.
#
# Lifecycle overview:
#
#   1. The framework creates the app via __init__()
#   2. mount() performs the initial render — emits templates, builds the
#      VNode tree, runs CreateEngine, appends to root, returns mutation
#      byte length
#   3. The renderer applies the mount mutations
#   4. Events arrive — handle_event() dispatches to the HandlerRegistry,
#      which marks scopes dirty
#   5. If dirty scopes exist (has_dirty()), flush() re-renders, diffs,
#      and writes update mutations
#   6. The renderer applies the update mutations
#   7. Repeat 4-6 until the app exits
#   8. destroy() cleans up all resources
#
# The handle_event() method takes a unified `value: String` parameter.
# This merges the two dispatch paths (dispatch_event and
# dispatch_event_with_string) into one — the renderer always passes the
# value through (empty string when not applicable). This resolves the
# input event value binding issue: the desktop event loop no longer needs
# app-specific branching on whether an event has a string payload.
#
# Design notes:
#
#   - render() builds a fresh VNode for the app's root component. It is
#     called by mount() for the initial render and by flush() for updates.
#     It is exposed in the trait so that renderers or test harnesses can
#     invoke it independently if needed.
#
#   - has_dirty() and consume_dirty() allow the event loop to check for
#     pending updates without requiring access to the raw ComponentContext.
#     This is cleaner than exposing a context() pointer, as it keeps the
#     internal implementation private to the app.
#
#   - The trait extends Movable because app instances may be moved into
#     heap allocations (for WASM pointer-based lifecycle) or into renderer
#     event loops (for desktop).
#
# Usage (implementing the trait for a counter app):
#
#     struct CounterApp(GuiApp):
#         var ctx: ComponentContext
#         var count: SignalI32
#         ...
#
#         fn __init__(out self):
#             self.ctx = ComponentContext.create()
#             self.count = self.ctx.use_signal(0)
#             self.ctx.setup_view(...)
#
#         fn render(mut self) -> UInt32:
#             var vb = self.ctx.render_builder()
#             vb.add_dyn_text(String("Count: ") + String(self.count.peek()))
#             return vb.build()
#
#         fn handle_event(
#             mut self, handler_id: UInt32, event_type: UInt8, value: String
#         ) -> Bool:
#             if len(value) > 0:
#                 return self.ctx.dispatch_event_with_string(
#                     handler_id, event_type, value
#                 )
#             return self.ctx.dispatch_event(handler_id, event_type)
#
#         fn mount(
#             mut self,
#             writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
#         ) -> Int32:
#             var vnode_idx = self.render()
#             return self.ctx.mount(writer_ptr, vnode_idx)
#
#         fn flush(
#             mut self,
#             writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
#         ) -> Int32:
#             if not self.ctx.consume_dirty():
#                 return 0
#             var new_idx = self.render()
#             self.ctx.diff(writer_ptr, new_idx)
#             return self.ctx.finalize(writer_ptr)
#
#         fn has_dirty(self) -> Bool:
#             return self.ctx.has_dirty()
#
#         fn consume_dirty(mut self) -> Bool:
#             return self.ctx.consume_dirty()
#
#         fn destroy(mut self):
#             self.ctx.destroy()
#
# Usage (in a shared example with launch()):
#
#     fn main() raises:
#         launch[CounterApp](AppConfig(title="Counter", width=400, height=350))
#
# The same source compiles for web (--target wasm64-wasi) and desktop
# (native). The GuiApp trait ensures the renderer can drive the app
# uniformly regardless of platform.

from memory import UnsafePointer
from bridge import MutationWriter


# ══════════════════════════════════════════════════════════════════════════════
# GuiApp — The app-side lifecycle contract
# ══════════════════════════════════════════════════════════════════════════════


trait GuiApp(Movable):
    """Lifecycle trait that every mojo-gui application implements.

    This is the app-side counterpart to `PlatformApp` (renderer-side).
    Together they form the abstraction boundary that enables shared
    examples to compile and run on every target from identical source.

    Every method has a clear role in the app lifecycle:

    - `__init__`: Create the app — set up context, signals, views.
    - `render`: Build a fresh VNode tree for the root component.
    - `handle_event`: Dispatch a user interaction to the handler registry.
    - `mount`: Perform the initial render and write mount mutations.
    - `flush`: Re-render dirty scopes and write update mutations.
    - `has_dirty`: Check if any scopes need re-rendering.
    - `consume_dirty`: Collect and consume dirty scopes.
    - `destroy`: Release all resources.

    Implementors should follow the existing app patterns — the refactor
    from free functions to trait methods is mechanical:

    | Free function pattern     | GuiApp method          |
    |---------------------------|------------------------|
    | `counter_app_rebuild()`   | `fn mount()`           |
    | `counter_app_flush()`     | `fn flush()`           |
    | `counter_app_handle_event()` | `fn handle_event()` |
    | `app.ctx.has_dirty()`     | `fn has_dirty()`       |
    | `app.ctx.consume_dirty()` | `fn consume_dirty()`   |
    | `app.ctx.destroy()`       | `fn destroy()`         |
    """

    # ── Construction ─────────────────────────────────────────────────

    fn __init__(out self):
        """Create and fully initialize the application.

        All setup — context creation, signal creation, view registration,
        event handler binding — must happen here. After __init__ returns,
        the app must be ready for mount().

        This is the equivalent of the existing pattern where the struct's
        __init__ does all setup (e.g., CounterApp.__init__ creates the
        ComponentContext, signals, and calls setup_view()).
        """
        ...

    # ── Rendering ────────────────────────────────────────────────────

    fn render(mut self) -> UInt32:
        """Build a fresh VNode tree for the root component.

        Returns the VNode index in the store. The caller (mount or flush)
        uses this to create or diff the DOM.

        This method should:
        1. Create a VNodeBuilder (via render_builder() or manually)
        2. Populate dynamic text, nodes, and attributes
        3. Return the built VNode index

        This is called by mount() for the initial render and by flush()
        for subsequent updates.

        Returns:
            VNode index in the VNodeStore.
        """
        ...

    # ── Event dispatch ───────────────────────────────────────────────

    fn handle_event(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch a user interaction event to the app's handler registry.

        This is the unified event entry point. The `value` parameter
        carries the string payload for input/change events (e.g.,
        event.target.value). For events without a string payload (clicks,
        keyboard shortcuts), `value` is an empty string.

        Implementations should dispatch to the appropriate path:

            if len(value) > 0:
                return self.ctx.dispatch_event_with_string(
                    handler_id, event_type, value
                )
            return self.ctx.dispatch_event(handler_id, event_type)

        Or, for apps with custom event routing (e.g., bench toolbar
        operations, multi-view navigation), perform app-specific logic
        before or after the standard dispatch.

        Args:
            handler_id: The handler to invoke (from HandlerRegistry).
            event_type: The event type tag (EVT_CLICK, EVT_INPUT, etc.).
            value: String payload from the event. Empty string when the
                   event has no string value.

        Returns:
            True if an action was executed, False otherwise.
        """
        ...

    # ── Mount lifecycle ──────────────────────────────────────────────

    fn mount(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Perform the initial render and write mount mutations.

        This is called once after __init__ to render the app for the
        first time. It should:

        1. Emit RegisterTemplate mutations for all templates
        2. Build the initial VNode tree via render()
        3. Run CreateEngine to generate DOM creation mutations
        4. Extract anchor ElementIds for dynamic slots (ConditionalSlot,
           KeyedList) if applicable
        5. Append the root nodes to the DOM root (element id 0)
        6. Finalize the mutation buffer

        The mutation buffer is owned by the caller (renderer). The app
        writes mutations into it via the provided MutationWriter pointer.

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer. The writer must be initialized with a
                        valid buffer and capacity before this call.

        Returns:
            Byte length of mutation data written. The renderer reads
            this many bytes from the mutation buffer to apply the
            initial DOM.
        """
        ...

    # ── Flush lifecycle ──────────────────────────────────────────────

    fn flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Re-render dirty scopes and write update mutations.

        Called after event dispatch when has_dirty() returns True (or
        after consume_dirty() returns True). This method should:

        1. Re-render the root component via render()
        2. Diff the old and new VNode trees
        3. Handle any conditional slots or keyed list transitions
        4. Finalize the mutation buffer

        The method should call consume_dirty() internally if it hasn't
        been called externally, or check its own dirty state. If nothing
        is dirty, it should return 0 without writing any mutations.

        Note: The MutationWriter should be reset before each flush call.
        The caller is responsible for resetting the writer (setting
        offset back to 0) between mount and flush, and between
        successive flush calls.

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer (same buffer as mount, reset to offset 0).

        Returns:
            Byte length of mutation data written, or 0 if nothing was
            dirty and no mutations were emitted.
        """
        ...

    # ── Dirty state queries ──────────────────────────────────────────

    fn has_dirty(self) -> Bool:
        """Check if any scopes need re-rendering.

        This is a non-consuming check — it does not collect or clear
        the dirty set. Use this in the event loop to decide whether to
        call flush():

            if app.has_dirty():
                var flush_len = app.flush(writer_ptr)
                if flush_len > 0:
                    renderer.apply_mutations(flush_len)

        Returns:
            True if at least one scope is marked dirty.
        """
        ...

    fn consume_dirty(mut self) -> Bool:
        """Collect and consume all dirty scopes.

        This drains the scheduler's dirty queue and prepares for
        re-rendering. After this call, has_dirty() will return False
        (until new events mark scopes dirty again).

        Typically called at the start of flush() to determine whether
        any work needs to be done. Some implementations call this
        internally in flush(); others let the event loop call it
        explicitly.

        Returns:
            True if any scopes were dirty (and consumed).
        """
        ...

    # ── Cleanup ──────────────────────────────────────────────────────

    fn destroy(mut self):
        """Release all resources held by the application.

        Called once when the app is shutting down. After this call,
        the app is in an invalid state and must not be used.

        Implementations should call self.ctx.destroy() to release
        the ComponentContext and all associated reactive state
        (signals, memos, effects, scopes, VNode store, etc.).
        """
        ...
