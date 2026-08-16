# TodoApp — Self-contained todo list application with empty state message.
#
# Phase 28: Extended with a ConditionalSlot that shows "No items yet —
# add one above!" when the todo list is empty, and hides it when items
# are present.
#
# Migrated to Phase 22 — fully WASM-driven input binding + Enter key:
#   - `bind_value(input_text)` — two-way value binding (M20.4)
#   - `oninput_set_string(input_text)` — inline input→signal binding (M20.3)
#   - `onclick_custom()` — inline custom click handler (M20.5)
#   - `onkeydown_enter_custom()` — inline Enter key handler (Phase 22)
#   - `register_view()` — auto-registers handlers + value bindings
#   - `render_builder()` — auto-populates events + bind_value at render time
#   - `SignalString` for reactive input text (Phase 19)
#   - `begin_item()` replaces manual `create_scope()` + `item_builder()`
#   - `add_custom_event()` replaces manual `register_handler()` + `add_dyn_event()`
#   - `get_action()` replaces manual handler_map lookup loop
#   - `add_class_if()` replaces 4-line if/else class pattern (Phase 18)
#   - `text_when()` replaces 4-line if/else text pattern (Phase 18)
#   - Multi-arg el_* overloads (no [...] list literal wrappers)
#   - KeyedList abstraction (bundles FragmentSlot + scope IDs + template ID + handler map)
#   - Constructor-based setup (all init in __init__)
#   - ctx.use_signal() for automatic scope subscription
#
# Phase 22 — WASM-Driven Add + Enter Key:
#   - Input value is synced to WASM SignalString via oninput_set_string (every keystroke)
#   - Input's value attribute is bound to the signal via bind_value (auto-updated on render)
#   - Add button click dispatches ACTION_CUSTOM → WASM reads signal, adds item, clears signal
#   - Enter key dispatches ACTION_KEY_ENTER_CUSTOM → runtime checks key == "Enter" → Add
#   - JS has NO special-casing — uniform event dispatch for all handlers
#   - main.js is identical to counter's — zero app-specific JS
#
# Architecture:
#   - TodoApp struct holds all state: items list, input text, signals, handlers
#   - Items are stored as a flat list of TodoItem structs (not signals)
#   - A "list_version" signal is bumped on every list mutation to trigger re-render
#   - JS dispatches events uniformly; WASM handles all routing
#   - Flush re-renders app shell (for bind_value updates) and items (for list changes)
#
# Templates (built via DSL with inline events):
#   - "todo-app": The app shell with input field + item list container
#       div > [ input(bind_value + oninput + onkeydown_enter) + button("Add", onclick_custom) + ul > dyn_node[0] ]
#       auto dyn_attr[0] = bind_value (value attr)
#       auto dyn_attr[1] = oninput_set_string handler
#       auto dyn_attr[2] = onkeydown_enter_custom handler (Enter key)
#       auto dyn_attr[3] = onclick_custom handler (Add button)
#   - "todo-item": A single list item (unchanged from Phase 17)
#       li > [ span > dynamic_text[0], button("✓") + button("✕") ]
#       dynamic_attr[0] = click handler for toggle
#       dynamic_attr[1] = click handler for remove
#       dynamic_attr[2] = class on the li (for completed styling)
#
# Compare with Dioxus (Rust):
#
#     fn TodoApp() -> Element {
#         let mut items = use_signal(|| vec![]);
#         let mut text = use_signal(|| String::new());
#         rsx! {
#             div {
#                 input { r#type: "text", value: "{text}",
#                         oninput: move |e| text.set(e.value()),
#                         placeholder: "What needs to be done?" }
#                 button { onclick: move |_| { add_item(&text); text.set(""); }, "Add" }
#                 ul { for item in items.read().iter() {
#                     li { class: if item.completed { "completed" } else { "" },
#                         span { "{item.text}" }
#                         button { onclick: move |_| toggle(item.id), "✓" }
#                         button { onclick: move |_| remove(item.id), "✕" }
#                     }
#                 }}
#             }
#         }
#     }
#
# Mojo equivalent (with Phase 20.5 two-way binding):
#
#     struct TodoApp:
#         var ctx: ComponentContext
#         var list_version: SignalI32
#         var input_text: SignalString
#         var items: KeyedList
#
#         fn __init__(out self):
#             self.ctx = ComponentContext.create()
#             self.list_version = self.ctx.use_signal(0)
#             self.ctx.end_setup()
#             self.input_text = self.ctx.create_signal_string(String(""))
#             self.ctx.register_view(
#                 el_div(
#                     el_input(
#                         attr("type", "text"),
#                         attr("placeholder", "What needs to be done?"),
#                         bind_value(self.input_text),
#                         oninput_set_string(self.input_text),
#                         onkeydown_enter_custom(),
#                     ),
#                     el_button(text("Add"), onclick_custom()),
#                     el_ul(dyn_node(0)),
#                 ),
#                 String("todo-app"),
#             )
#             self.enter_handler = self.ctx.view_event_handler_id(1)
#             self.add_handler = self.ctx.view_event_handler_id(2)
#             self.items = KeyedList(self.ctx.register_extra_template(...))
#
#         fn handle_event(mut self, handler_id: UInt32) -> Bool:
#             if handler_id == self.add_handler:
#                 var t = self.input_text.peek()
#                 if len(t) > 0:
#                     self.add_item(t)
#                     self.input_text.set(String(""))
#                 return True
#             var action = self.items.get_action(handler_id)
#             if action.found:
#                 if action.tag == TODO_ACTION_TOGGLE:
#                     self.toggle_item(action.data)
#                 elif action.tag == TODO_ACTION_REMOVE:
#                     self.remove_item(action.data)
#                 return True
#             return False

from memory import UnsafePointer, alloc
from bridge import MutationWriter
from mutations import CreateEngine
from events import HandlerEntry
from component import ComponentContext, KeyedList, ConditionalSlot
from signals import SignalI32, SignalString
from vdom import (
    VNode,
    VNodeStore,
)
from html import (
    Node,
    el_div,
    el_span,
    el_button,
    el_input,
    el_ul,
    el_li,
    el_p,
    text,
    dyn_text,
    dyn_node,
    dyn_attr,
    attr,
    to_template,
    text_when,
    VNodeBuilder,
    # Phase 20 — inline event/binding helpers
    bind_value,
    oninput_set_string,
    onclick_custom,
    onkeydown_enter_custom,
)


struct TodoItem(Copyable):
    """A single todo list item."""

    var id: Int32
    var text: String
    var completed: Bool

    fn __init__(out self, id: Int32, text: String, completed: Bool):
        self.id = id
        self.text = text
        self.completed = completed

    fn __copyinit__(out self, other: Self):
        self.id = other.id
        self.text = other.text
        self.completed = other.completed

    fn __moveinit__(out self, deinit other: Self):
        self.id = other.id
        self.text = other.text^
        self.completed = other.completed


# App-defined action tags for ItemBuilder.add_custom_event() dispatch.
# These are retrieved via KeyedList.get_action().
comptime TODO_ACTION_TOGGLE: UInt8 = 1
comptime TODO_ACTION_REMOVE: UInt8 = 2


struct TodoApp(Movable):
    """Self-contained todo list application state.

    All setup — context creation, signal creation, template registration,
    and event handler binding — happens in __init__.  The lifecycle
    functions are thin delegations to ComponentContext.

    Uses KeyedList with Phase 17 ItemBuilder for ergonomic per-item
    building and HandlerAction for dispatch.

    Phase 20.5: Fully WASM-driven Add flow.  The input element has
    `bind_value(input_text)` for value → signal binding and
    `oninput_set_string(input_text)` for signal ← input binding.
    The Add button uses `onclick_custom()` — WASM reads the signal,
    adds the item, and clears the signal.  JS has no special-casing.

    The item list lives inside the <ul> element of the app template.
    On initial mount, a placeholder comment node occupies the <ul>.
    KeyedList tracks the placeholder/anchor, current fragment,
    and mounted state for the item list transitions.

    Phase 28: A ConditionalSlot manages an empty-state message shown
    when the list has no items.  The message occupies dyn_node[1] in
    the app template (after the <ul> which has dyn_node[0]).
    """

    var ctx: ComponentContext
    var list_version: SignalI32
    var items: KeyedList  # bundles template_id + FragmentSlot + scope_ids + handler_map
    var data: List[TodoItem]
    var next_id: Int32
    var input_text: SignalString  # Phase 19: reactive string signal (no subscription)
    # Handler IDs for the app-level actions (auto-registered by register_view)
    var add_handler: UInt32
    var enter_handler: UInt32  # Phase 22: Enter key on input → same as Add
    # Phase 28: Empty state message
    var empty_msg_tmpl: UInt32
    var empty_msg_slot: ConditionalSlot

    fn __init__(out self):
        """Initialize the todo app with all reactive state, templates, and handlers.

        Creates: ComponentContext (runtime, VNode store, element ID
        allocator, scheduler), root scope, list_version signal, the
        app shell and item templates.

        Phase 22: Uses register_view() for the app template with inline
        event/binding helpers.  The Add button and Enter key handlers are
        auto-registered and retrieved via view_event_handler_id().

        Template "todo-app" (via register_view with auto dyn_attr):
            div > [ input(bind_value + oninput_set_string + onkeydown_enter),
                    button("Add", onclick_custom),
                    ul > dyn_node[0],
                    dyn_node[1] ]          ← empty state message slot (Phase 28)
            auto dyn_attr[0] = bind_value (value attr from signal)
            auto dyn_attr[1] = oninput_set_string handler
            auto dyn_attr[2] = onkeydown_enter_custom handler (Enter key)
            auto dyn_attr[3] = onclick_custom handler (Add button)

        Template "todo-empty" (via register_extra_template):
            p > "No items yet — add one above!"

        Template "todo-item" (via register_extra_template):
            li + dyn_attr[2] > [ span > dyn_text[0],
                                  button("✓") + dyn_attr[0],
                                  button("✕") + dyn_attr[1] ]

        Uses multi-arg el_* overloads — no [...] list literal wrappers needed.
        """
        # 1. Create context and signal
        self.ctx = ComponentContext.create()
        self.list_version = self.ctx.use_signal(0)
        self.ctx.end_setup()

        # 2. Create input_text SignalString (before register_view, since
        #    bind_value/oninput_set_string read the signal's keys)
        self.input_text = self.ctx.create_signal_string(String(""))

        # 3. Register the "todo-app" template via register_view()
        #    with inline bindings and event handlers.
        #    register_view() auto-assigns dyn_attr indices and registers
        #    handlers for NODE_EVENT and NODE_BIND_VALUE nodes.
        self.ctx.register_view(
            el_div(
                el_input(
                    attr(String("type"), String("text")),
                    attr(
                        String("placeholder"),
                        String("What needs to be done?"),
                    ),
                    bind_value(self.input_text),
                    oninput_set_string(self.input_text),
                    onkeydown_enter_custom(),
                ),
                el_button(text(String("Add")), onclick_custom()),
                el_ul(dyn_node(0)),
                dyn_node(1),
            ),
            String("todo-app"),
        )

        # 4. Extract handler IDs from tree-walk order:
        #    oninput_set_string is 0th, onkeydown_enter_custom is 1st,
        #    onclick_custom is 2nd.
        self.enter_handler = self.ctx.view_event_handler_id(1)
        self.add_handler = self.ctx.view_event_handler_id(2)

        # 5. Register the "todo-item" template via KeyedList
        self.items = KeyedList(
            self.ctx.register_extra_template(
                el_li(
                    dyn_attr(2),  # class attr on li
                    el_span(dyn_text(0)),
                    el_button(text(String("✓")), dyn_attr(0)),
                    el_button(text(String("✕")), dyn_attr(1)),
                ),
                String("todo-item"),
            )
        )

        # 6. Register the "todo-empty" message template (Phase 28)
        self.empty_msg_tmpl = self.ctx.register_extra_template(
            el_p(text(String("No items yet -- add one above!"))),
            String("todo-empty"),
        )
        self.empty_msg_slot = ConditionalSlot()

        # 7. Initialize remaining state
        self.data = List[TodoItem]()
        self.next_id = 1

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.list_version = other.list_version^
        self.items = other.items^
        self.data = other.data^
        self.next_id = other.next_id
        self.input_text = (
            other.input_text.copy()
        )  # SignalString is Copyable (not ImplicitlyCopyable)
        self.add_handler = other.add_handler
        self.enter_handler = other.enter_handler
        self.empty_msg_tmpl = other.empty_msg_tmpl
        self.empty_msg_slot = other.empty_msg_slot.copy()

    fn add_item(mut self, text: String):
        """Add a new item and bump the list version signal."""
        if len(text) == 0:
            return
        self.data.append(TodoItem(self.next_id, text, False))
        self.next_id += 1
        self._bump_version()

    fn remove_item(mut self, item_id: Int32):
        """Remove an item by ID and bump the list version signal."""
        for i in range(len(self.data)):
            if self.data[i].id == item_id:
                # Swap-remove for O(1)
                var last = len(self.data) - 1
                if i != last:
                    self.data[i] = self.data[last].copy()
                _ = self.data.pop()
                self._bump_version()
                return

    fn toggle_item(mut self, item_id: Int32):
        """Toggle an item's completed status and bump the list version signal.
        """
        for i in range(len(self.data)):
            if self.data[i].id == item_id:
                self.data[i].completed = not self.data[i].completed
                self._bump_version()
                return

    fn _bump_version(mut self):
        """Increment the list version signal to trigger re-render."""
        self.list_version += 1

    fn build_item_vnode(mut self, item: TodoItem) -> UInt32:
        """Build a keyed VNode for a single todo item.

        Uses Phase 17 ItemBuilder + Phase 18 conditional helpers:
          - `begin_item()` creates child scope + keyed VNodeBuilder
          - `add_custom_event()` registers handler + maps action + adds event attr
          - `add_class_if()` replaces 4-line if/else class pattern
          - `text_when()` replaces 4-line if/else text pattern

        Template "todo-item": li > [ span > dynamic_text[0], button("✓"), button("✕") ]
          dynamic_text[0] = item text (possibly with completion indicator)
          dynamic_attr[0] = click on toggle button → TODO_ACTION_TOGGLE
          dynamic_attr[1] = click on remove button → TODO_ACTION_REMOVE
          dynamic_attr[2] = class on the li element
        """
        var ib = self.items.begin_item(String(item.id), self.ctx)

        # Dynamic text: item text with completion indicator
        ib.add_dyn_text(
            text_when(
                item.completed,
                String("✓ ") + item.text,
                item.text,
            )
        )

        # Dynamic attr 0: toggle handler (click on ✓ button)
        ib.add_custom_event(String("click"), TODO_ACTION_TOGGLE, item.id)

        # Dynamic attr 1: remove handler (click on ✕ button)
        ib.add_custom_event(String("click"), TODO_ACTION_REMOVE, item.id)

        # Dynamic attr 2: class on the li element
        ib.add_class_if(item.completed, String("completed"))

        return ib.index()

    fn build_items_fragment(mut self) -> UInt32:
        """Build a Fragment VNode containing keyed item children.

        Uses KeyedList.begin_rebuild() to destroy old child scopes,
        clear the handler map, and create a new empty fragment, then
        builds each item VNode and pushes it as a fragment child.
        """
        var frag_idx = self.items.begin_rebuild(self.ctx)
        for i in range(len(self.data)):
            var item_idx = self.build_item_vnode(self.data[i].copy())
            self.items.push_child(self.ctx, frag_idx, item_idx)
        return frag_idx

    fn handle_event(mut self, handler_id: UInt32) -> Bool:
        """Dispatch an event by handler ID.

        Phase 22: Both the Add button (onclick_custom) and Enter key
        (onkeydown_enter_custom) trigger the same Add logic — reads
        the input text from the SignalString, adds the item, and clears
        the signal.  JS no longer needs any app-specific wiring.

        Uses Phase 17 `get_action()` to look up toggle/remove handlers
        in the KeyedList's handler map.

        Returns True if the handler was found and the action executed,
        False otherwise.
        """
        if handler_id == self.add_handler or handler_id == self.enter_handler:
            # Phase 20.5: WASM-driven Add — read signal, add item, clear
            var input = self.input_text.peek()
            if len(input) > 0:
                self.add_item(input)
                self.input_text.set(String(""))
            return True

        var action = self.items.get_action(handler_id)
        if action.found:
            if action.tag == TODO_ACTION_TOGGLE:
                self.toggle_item(action.data)
                return True
            elif action.tag == TODO_ACTION_REMOVE:
                self.remove_item(action.data)
                return True
        return False

    fn render(mut self) -> UInt32:
        """Build the app shell VNode using render_builder (auto-populates).

        Phase 20.5: Uses render_builder() which auto-populates all
        bindings registered by register_view() in tree-walk order:
          auto dyn_attr[0] = bind_value (reads input_text signal → "value" attr)
          auto dyn_attr[1] = oninput_set_string event listener
          auto dyn_attr[2] = onclick_custom event listener (Add button)
          dyn_node[0]      = placeholder (item list managed by KeyedList)
          dyn_node[1]      = placeholder (empty message managed by ConditionalSlot)
        """
        var vb = self.ctx.render_builder()

        # Dynamic node 0: placeholder in the <ul>
        vb.add_dyn_placeholder()

        # Dynamic node 1: placeholder for empty state message (Phase 28)
        vb.add_dyn_placeholder()

        return vb.build()

    fn build_empty_message(mut self) -> UInt32:
        """Build the empty state message VNode (Phase 28).

        Returns the VNode index in the store.
        """
        var vb = VNodeBuilder(self.empty_msg_tmpl, self.ctx.store_ptr())
        return vb.index()


fn todo_app_init() -> UnsafePointer[TodoApp, MutExternalOrigin]:
    """Initialize the todo app.  Returns a pointer to the app state.

    All setup happens in TodoApp.__init__() — this function just
    allocates the heap slot and moves the app into it.
    """
    var app_ptr = alloc[TodoApp](1)
    app_ptr.init_pointee_move(TodoApp())
    return app_ptr


fn todo_app_destroy(app_ptr: UnsafePointer[TodoApp, MutExternalOrigin]):
    """Destroy the todo app and free all resources."""
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()


fn todo_app_rebuild(
    mut app: TodoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Initial render (mount) of the todo app.

    Builds the app shell VNode and mounts it.  The <ul> starts with a
    placeholder comment node whose ElementId we save for later use.

    Phase 20.5: Uses render() which auto-populates bind_value and
    event handlers via render_builder().

    Returns the byte offset (length) of the mutation data written.
    """
    # Emit all registered templates so JS can build DOM from mutations
    app.ctx.shell.emit_templates(writer_ptr)

    # Build the app shell VNode (no items yet — just the template)
    var app_vnode_idx = app.render()
    app.ctx.current_vnode = Int(app_vnode_idx)

    # Build an empty items fragment and store it
    var frag_idx = app.build_items_fragment()

    # Create the app template via CreateEngine.
    # This emits LoadTemplate, AssignId, NewEventListener, and
    # CreatePlaceholder + ReplacePlaceholder for dynamic[0].
    var engine = CreateEngine(
        writer_ptr,
        app.ctx.shell.eid_alloc,
        app.ctx.shell.runtime,
        app.ctx.shell.store,
    )
    var num_roots = engine.create_node(app_vnode_idx)

    # After CreateEngine, dynamic[0]'s placeholder has an ElementId.
    # Initialize the KeyedList's slot with the anchor and empty fragment.
    var anchor_id: UInt32 = 0
    var app_vnode_ptr = app.ctx.store_ptr()[0].get_ptr(app_vnode_idx)
    if app_vnode_ptr[0].dyn_node_id_count() > 0:
        anchor_id = app_vnode_ptr[0].get_dyn_node_id(0)
    app.items.init_slot(anchor_id, frag_idx)

    # Phase 28: Extract the anchor for the empty message slot (dyn_node[1])
    var msg_anchor_id: UInt32 = 0
    if app_vnode_ptr[0].dyn_node_id_count() > 1:
        msg_anchor_id = app_vnode_ptr[0].get_dyn_node_id(1)
    app.empty_msg_slot = ConditionalSlot(msg_anchor_id)

    # Append the app shell to root element (id 0)
    writer_ptr[0].append_children(0, num_roots)

    # Phase 28: Show empty message on initial mount (list starts empty)
    var msg_idx = app.build_empty_message()
    app.empty_msg_slot = app.ctx.flush_conditional_slot(
        writer_ptr, app.empty_msg_slot, msg_idx
    )

    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)


fn todo_app_flush(
    mut app: TodoApp,
    writer_ptr: UnsafePointer[MutationWriter, MutExternalOrigin],
) -> Int32:
    """Flush pending updates after a list mutation or input clear.

    Phase 22: Re-renders the app shell VNode (to update bind_value
    attribute when the signal changes, e.g. after Add clears input)
    and flushes the item list via KeyedList.

    Uses KeyedList.flush() which delegates fragment transitions
    (empty↔populated) to the reusable flush_fragment lifecycle helper.

    Returns the byte offset (length) of mutation data, or 0 if nothing dirty.
    """
    # Collect and consume dirty scopes via the scheduler
    if not app.ctx.consume_dirty():
        return 0

    # Phase 20.5: Re-render app shell to pick up bind_value changes
    # (e.g. input cleared after Add).  The diff detects changes in
    # dynamic attrs (value binding) and emits SetAttribute mutations.
    # dyn_node(0) stays as placeholder — diff sees placeholder vs
    # placeholder and does nothing (KeyedList manages it separately).
    var new_app_idx = app.render()
    app.ctx.diff(writer_ptr, new_app_idx)

    # Build a new items fragment from the current item list
    var new_frag_idx = app.build_items_fragment()

    # Flush via KeyedList (handles all three transitions)
    app.items.flush(app.ctx, writer_ptr, new_frag_idx)

    # Phase 28: Show or hide the empty state message
    if len(app.data) == 0:
        # List is empty → show the message
        if not app.empty_msg_slot.mounted:
            var msg_idx = app.build_empty_message()
            app.empty_msg_slot = app.ctx.flush_conditional_slot(
                writer_ptr, app.empty_msg_slot, msg_idx
            )
    else:
        # List has items → hide the message
        if app.empty_msg_slot.mounted:
            app.empty_msg_slot = app.ctx.flush_conditional_slot_empty(
                writer_ptr, app.empty_msg_slot
            )

    writer_ptr[0].finalize()
    return Int32(writer_ptr[0].offset)
