# CounterApp — Self-contained counter application with conditional rendering.
#
# Phase 28: Extended with a "Toggle detail" button and a conditional
# detail section that shows/hides based on a `show_detail: SignalBool`.
# Uses ConditionalSlot to manage the conditional DOM content.
#
# Phase 3.9 (Step 3.9.4): Refactored to implement the GuiApp trait.
# Free functions have been removed — the @export wrappers in
# web/src/main.mojo now use the generic gui_app_exports helpers
# which call GuiApp trait methods directly (Step 3.9.5).
#
# This version achieves maximum Dioxus-like ergonomics by using:
#   - setup_view() — combines end_setup + register_view in one call
#   - dyn_text()   — auto-numbered dynamic text (no manual index tracking)
#   - flush()      — combines diff + finalize in one call
#   - __init__     — all setup happens in the constructor
#   - Multi-arg el_* overloads — no [...] list literal wrappers needed
#   - ConditionalSlot — manages show/hide of detail section
#   - GuiApp trait — unified lifecycle for cross-platform launch()
#
# Compare with the Dioxus equivalent:
#
#     fn App() -> Element {
#         let mut count = use_signal(|| 0);
#         let mut show_detail = use_signal(|| false);
#         rsx! {
#             h1 { "High-Five counter: {count}" }
#             button { onclick: move |_| count += 1, "Up high!" }
#             button { onclick: move |_| count -= 1, "Down low!" }
#             button { onclick: move |_| show_detail.toggle(), "Toggle detail" }
#             if show_detail() {
#                 div {
#                     p { "Count is {if count() % 2 == 0 { "even" } else { "odd" }}" }
#                     p { "Doubled: {count() * 2}" }
#                 }
#             }
#         }
#     }
#
# Mojo equivalent:
#
#     struct CounterApp(GuiApp):
#         var ctx: ComponentContext
#         var count: SignalI32
#         var show_detail: SignalBool
#         var detail_tmpl: UInt32
#         var cond_slot: ConditionalSlot
#
#         fn __init__(out self):
#             self.ctx = ComponentContext.create()
#             self.count = self.ctx.use_signal(0)
#             self.show_detail = self.ctx.use_signal_bool(False)
#             self.ctx.setup_view(
#                 el_div(
#                     el_h1(dyn_text()),
#                     el_button(text("Up high!"), onclick_add(self.count, 1)),
#                     el_button(text("Down low!"), onclick_sub(self.count, 1)),
#                     el_button(text("Toggle detail"), onclick_toggle(self.show_detail)),
#                     dyn_node(1),
#                 ),
#                 String("counter"),
#             )
#             self.detail_tmpl = self.ctx.register_extra_template(
#                 el_div(el_p(dyn_text(0)), el_p(dyn_text(1))),
#                 String("counter-detail"),
#             )
#             self.cond_slot = ConditionalSlot()
#
# Template structure (built via setup_view with inline events):
#   div
#     h1
#       dynamic_text[0]      ← "High-Five counter: N"  (auto-numbered)
#     button  (text: "Up high!")
#       dynamic_attr[0]      ← onclick → increment handler (auto-registered)
#     button  (text: "Down low!")
#       dynamic_attr[1]      ← onclick → decrement handler (auto-registered)
#     button  (text: "Toggle detail")
#       dynamic_attr[2]      ← onclick → toggle handler (auto-registered)
#     dyn_node[1]            ← conditional detail slot
#
# Detail template ("counter-detail"):
#   div
#     p > dynamic_text[0]   ← "Count is even" / "Count is odd"
#     p > dynamic_text[1]   ← "Doubled: N"

from memory import UnsafePointer
from bridge import MutationWriter
from component import ComponentContext, ConditionalSlot
from signals import SignalI32, SignalBool
from signals.runtime import Runtime
from platform import GuiApp
from html import (
    Node,
    VNodeBuilder,
    el_div,
    el_h1,
    el_p,
    el_button,
    text,
    dyn_text,
    dyn_node,
    onclick_add,
    onclick_sub,
    onclick_toggle,
)


struct CounterApp(GuiApp):
    """Self-contained counter application state with conditional detail.

    Implements the GuiApp trait for unified cross-platform launch().

    All setup — context creation, signal creation, view registration,
    and event handler binding — happens in __init__.  The lifecycle
    methods (mount, handle_event, flush) encapsulate all app-specific
    logic so that a generic event loop can drive the app without
    app-specific branching.

    Phase 28: Added `show_detail` toggle and `ConditionalSlot` for the
    detail section.  The detail is a separate template rendered when
    `show_detail` is True, managed by ConditionalSlot transitions.

    Phase 3.9: Implements GuiApp trait — mount(), handle_event(), flush(),
    has_dirty(), consume_dirty(), destroy() are now struct methods instead
    of free functions.
    """

    var ctx: ComponentContext
    var count: SignalI32
    var show_detail: SignalBool
    var detail_tmpl: UInt32
    var cond_slot: ConditionalSlot

    fn __init__(out self):
        """Initialize the counter app with all reactive state and view.

        Creates: ComponentContext (runtime, VNode store, element ID
        allocator, scheduler), root scope, count signal, show_detail
        signal, the main template with inline event handlers, and the
        detail template for conditional rendering.

        setup_view() combines end_setup() + register_view():
          - Closes the render bracket (hook registration)
          - Processes the Node tree: auto-numbers dyn_text() slots,
            collects inline event handlers, builds the template,
            and registers handlers

        dyn_text() uses auto-numbering — no manual index needed.
        dyn_node(1) is at index 1 because dyn_text[0] occupies index 0
        in the shared dynamic_nodes index space.

        Multi-arg el_* overloads eliminate [...] list literal wrappers,
        bringing the DSL closer to Dioxus's rsx! macro.
        """
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.show_detail = self.ctx.use_signal_bool(False)
        self.ctx.setup_view(
            el_div(
                el_h1(dyn_text()),
                el_button(
                    text(String("Up high!")),
                    onclick_add(self.count, 1),
                ),
                el_button(
                    text(String("Down low!")),
                    onclick_sub(self.count, 1),
                ),
                el_button(
                    text(String("Toggle detail")),
                    onclick_toggle(self.show_detail),
                ),
                dyn_node(1),
            ),
            String("counter"),
        )
        # Register the detail template separately (not part of the main view)
        self.detail_tmpl = self.ctx.register_extra_template(
            el_div(
                el_p(dyn_text(0)),
                el_p(dyn_text(1)),
            ),
            String("counter-detail"),
        )
        self.cond_slot = ConditionalSlot()

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.count = other.count^
        self.show_detail = other.show_detail^
        self.detail_tmpl = other.detail_tmpl
        self.cond_slot = other.cond_slot.copy()

    # ── GuiApp trait: Rendering ──────────────────────────────────────

    fn render(mut self) -> UInt32:
        """Build a fresh VNode for the counter component.

        Uses render_builder() which auto-populates the event handler
        attributes registered by setup_view().  The component only
        needs to provide dynamic text values (in tree-walk order).

        dyn_node[1] always gets a placeholder — the ConditionalSlot
        manages the actual detail content separately (just like
        KeyedList manages list content via FragmentSlot).

        Returns the VNode index in the store.
        """
        var vb = self.ctx.render_builder()
        vb.add_dyn_text(
            String("High-Five counter: ") + String(self.count.peek())
        )
        # dyn_node[1] — placeholder for conditional detail
        vb.add_dyn_placeholder()
        return vb.build()

    # ── GuiApp trait: Event dispatch ─────────────────────────────────

    fn handle_event(
        mut self, handler_id: UInt32, event_type: UInt8, value: String
    ) -> Bool:
        """Dispatch an event to the counter app.

        The counter app only has click events (increment, decrement,
        toggle) which don't carry string values. The `value` parameter
        is accepted for GuiApp trait conformance but is not used.

        If a non-empty value is provided, it is passed through via
        dispatch_event_with_string for forward compatibility.

        Args:
            handler_id: The handler to invoke (from HandlerRegistry).
            event_type: The event type tag (EVT_CLICK, etc.).
            value: String payload from the event. Empty for click events.

        Returns:
            True if an action was executed, False otherwise.
        """
        if len(value) > 0:
            return self.ctx.dispatch_event_with_string(
                handler_id, event_type, value
            )
        return self.ctx.dispatch_event(handler_id, event_type)

    # ── GuiApp trait: Mount lifecycle ────────────────────────────────

    fn mount(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Initial render (mount) of the counter app.

        Emits RegisterTemplate mutations for all templates, then builds the
        VNode tree, runs CreateEngine, emits AppendChildren to mount to
        root (id 0), and finalizes the mutation buffer.

        After mount, extracts the anchor ElementId from dyn_node_ids[1]
        (the conditional slot) to initialize the ConditionalSlot.

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer.

        Returns the byte offset (length) of the mutation data written.
        """
        var vnode_idx = self.render()
        var result = self.ctx.mount(writer_ptr, vnode_idx)

        # Extract the anchor ElementId for the conditional slot (dyn_node[1]).
        # dyn_node_ids[0] is the dyn_text node, dyn_node_ids[1] is the
        # conditional placeholder.
        var anchor_id: UInt32 = 0
        var app_vnode_ptr = self.ctx.store_ptr()[0].get_ptr(vnode_idx)
        if app_vnode_ptr[0].dyn_node_id_count() > 1:
            anchor_id = app_vnode_ptr[0].get_dyn_node_id(1)
        self.cond_slot = ConditionalSlot(anchor_id)

        return result

    # ── GuiApp trait: Flush lifecycle ────────────────────────────────

    fn flush(
        mut self,
        writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
    ) -> Int32:
        """Flush pending updates after event dispatch.

        If dirty scopes exist, re-renders the counter component, diffs the
        old and new VNode trees, writes mutations for the app shell, then
        handles the conditional detail section via ConditionalSlot.

        The ConditionalSlot manages three transitions:
          - show_detail goes True: create detail VNode, replace placeholder
          - show_detail stays True: diff old detail vs new detail
          - show_detail goes False: create new placeholder, remove detail

        Args:
            writer_ptr: Pointer to the MutationWriter for the mutation
                        buffer (reset to offset 0 by the caller).

        Returns the byte offset (length) of the mutation data written,
        or 0 if there was nothing to update.
        """
        if not self.ctx.consume_dirty():
            return 0

        # 1. Re-render and diff the app shell
        var new_idx = self.render()
        self.ctx.diff(writer_ptr, new_idx)

        # 2. Handle conditional detail section
        if self.show_detail.get():
            # Show or update the detail section
            var detail_idx = self.build_detail()
            self.cond_slot = self.ctx.flush_conditional_slot(
                writer_ptr, self.cond_slot, detail_idx
            )
        else:
            # Hide the detail section (back to placeholder)
            self.cond_slot = self.ctx.flush_conditional_slot_empty(
                writer_ptr, self.cond_slot
            )

        # 3. Finalize the mutation buffer
        return self.ctx.finalize(writer_ptr)

    # ── GuiApp trait: Dirty state queries ────────────────────────────

    fn has_dirty(self) -> Bool:
        """Check if any scopes need re-rendering.

        Returns:
            True if at least one scope is marked dirty.
        """
        return self.ctx.has_dirty()

    fn consume_dirty(mut self) -> Bool:
        """Collect and consume all dirty scopes.

        Returns:
            True if any scopes were dirty (and consumed).
        """
        return self.ctx.consume_dirty()

    # ── GuiApp trait: Cleanup ────────────────────────────────────────

    fn destroy(mut self):
        """Release all resources held by the counter app."""
        self.ctx.destroy()

    # ── App-specific helpers (not part of GuiApp) ────────────────────

    fn build_detail(mut self) -> UInt32:
        """Build the detail VNode (even/odd + doubled value).

        Only called when show_detail is True.

        Returns the VNode index in the store.
        """
        var count_val = self.count.peek()
        var vb = VNodeBuilder(self.detail_tmpl, self.ctx.store_ptr())
        if count_val % 2 == 0:
            vb.add_dyn_text(String("Count is even"))
        else:
            vb.add_dyn_text(String("Count is odd"))
        vb.add_dyn_text(String("Doubled: ") + String(count_val * 2))
        return vb.index()
