//! Integration tests for the mojo-blitz-shim.
//!
//! These tests exercise the shim's public API through the BlitzContext
//! methods directly (not through the C FFI wrappers). They use headless
//! mode (no event loop, no window, no GPU) so they can run in CI without
//! a display server.
//!
//! Test categories:
//!   1. Headless context lifecycle
//!   2. DOM element creation & manipulation
//!   3. Text node operations
//!   4. Attribute get/set/remove
//!   5. Tree structure (append, insert before/after, replace, remove)
//!   6. Template registration & cloning
//!   7. Event injection & polling
//!   8. Mutation batch markers
//!   9. DOM serialization
//!  10. Node ID mapping & stack operations
//!  11. DOM inspection helpers
//!  12. FFI poll_event_into (output-pointer polling)

use mojo_blitz::BlitzContext;

// FFI imports for poll_event_into tests
unsafe extern "C" {
    fn mblitz_poll_event_into(
        ctx: *mut BlitzContext,
        out_handler_id: *mut u32,
        out_event_type: *mut u8,
        out_value_ptr: *mut *const u8,
        out_value_len: *mut u32,
    ) -> i32;
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Create a fresh headless context for each test.
fn headless() -> BlitzContext {
    BlitzContext::new_headless(800, 600)
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. Headless context lifecycle
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn headless_context_is_alive() {
    let ctx = headless();
    assert!(ctx.alive, "headless context should start alive");
}

#[test]
fn headless_context_mount_point_exists() {
    let ctx = headless();
    // Element ID 0 should resolve to the body (mount point)
    let body_id = ctx.resolve_id(0);
    assert!(body_id.is_some(), "mount point (id 0) should resolve");
    assert_eq!(body_id.unwrap(), ctx.mount_point_id);
}

#[test]
fn headless_context_has_no_event_loop() {
    let ctx = headless();
    assert!(
        ctx.event_loop.is_none(),
        "headless context should not have an event loop"
    );
}

#[test]
fn headless_get_node_tag_of_mount_point() {
    let ctx = headless();
    let tag = ctx.get_node_tag(0);
    assert_eq!(tag, "body", "mount point should be <body>");
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. DOM element creation & manipulation
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn create_element_returns_valid_id() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    assert!(div_id > 0, "created element should have a non-zero slab ID");
}

#[test]
fn create_element_and_assign_id() {
    let mut ctx = headless();
    let div_blitz_id = ctx.create_element("div");
    ctx.assign_id(1, div_blitz_id);

    let tag = ctx.get_node_tag(1);
    assert_eq!(tag, "div");
}

#[test]
fn create_multiple_elements() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    let h1_id = ctx.create_element("h1");
    let btn_id = ctx.create_element("button");

    assert_ne!(div_id, h1_id);
    assert_ne!(h1_id, btn_id);
    assert_ne!(div_id, btn_id);

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, h1_id);
    ctx.assign_id(3, btn_id);

    assert_eq!(ctx.get_node_tag(1), "div");
    assert_eq!(ctx.get_node_tag(2), "h1");
    assert_eq!(ctx.get_node_tag(3), "button");
}

#[test]
fn create_element_various_tags() {
    let mut ctx = headless();
    let tags = [
        "div", "span", "p", "h1", "h2", "h3", "ul", "li", "input", "form", "section", "article",
        "nav", "header", "footer", "main", "a", "img", "table", "tr", "td",
    ];
    for (i, tag) in tags.iter().enumerate() {
        let blitz_id = ctx.create_element(tag);
        ctx.assign_id((i + 1) as u32, blitz_id);
        assert_eq!(ctx.get_node_tag((i + 1) as u32), *tag);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. Text node operations
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn create_text_node() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("Hello, World!");
    ctx.assign_id(1, text_id);

    let tag = ctx.get_node_tag(1);
    assert_eq!(tag, "#text", "text nodes should report tag #text");
}

#[test]
fn text_node_content() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("Hello, World!");
    ctx.assign_id(1, text_id);

    let content = ctx.get_text_content(1);
    assert_eq!(content, "Hello, World!");
}

#[test]
fn set_text_content_updates_node() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("original");
    ctx.assign_id(1, text_id);

    assert_eq!(ctx.get_text_content(1), "original");

    ctx.set_text_content(text_id, "updated");
    assert_eq!(ctx.get_text_content(1), "updated");
}

#[test]
fn text_node_with_unicode() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("Héllo 🌍 wörld — 日本語");
    ctx.assign_id(1, text_id);

    assert_eq!(ctx.get_text_content(1), "Héllo 🌍 wörld — 日本語");
}

#[test]
fn text_node_empty_string() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("");
    ctx.assign_id(1, text_id);

    let content = ctx.get_text_content(1);
    assert!(
        content.is_empty(),
        "empty text node should return empty string"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. Attribute get/set/remove
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn set_and_get_attribute() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    ctx.set_attribute(div_id, "class", "container");
    let value = ctx.get_attribute_value(1, "class");
    assert_eq!(value, Some("container".to_string()));
}

#[test]
fn get_nonexistent_attribute_returns_none() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    let value = ctx.get_attribute_value(1, "data-missing");
    assert_eq!(value, None);
}

#[test]
fn set_attribute_overwrites_existing() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    ctx.set_attribute(div_id, "id", "first");
    assert_eq!(ctx.get_attribute_value(1, "id"), Some("first".to_string()));

    ctx.set_attribute(div_id, "id", "second");
    assert_eq!(ctx.get_attribute_value(1, "id"), Some("second".to_string()));
}

#[test]
fn remove_attribute() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    ctx.set_attribute(div_id, "class", "highlight");
    assert!(ctx.get_attribute_value(1, "class").is_some());

    ctx.remove_attribute(div_id, "class");
    assert_eq!(ctx.get_attribute_value(1, "class"), None);
}

#[test]
fn multiple_attributes_on_same_element() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    ctx.set_attribute(div_id, "class", "container");
    ctx.set_attribute(div_id, "id", "main");
    ctx.set_attribute(div_id, "data-count", "42");

    assert_eq!(
        ctx.get_attribute_value(1, "class"),
        Some("container".to_string())
    );
    assert_eq!(ctx.get_attribute_value(1, "id"), Some("main".to_string()));
    assert_eq!(
        ctx.get_attribute_value(1, "data-count"),
        Some("42".to_string())
    );
}

#[test]
fn get_attribute_on_nonexistent_node_returns_none() {
    let ctx = headless();
    let value = ctx.get_attribute_value(999, "class");
    assert_eq!(value, None, "unmapped ID should return None");
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. Tree structure — append, insert, replace, remove
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn append_child_to_mount_point() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    let body_blitz_id = ctx.resolve_id(0).unwrap();
    ctx.append_children(body_blitz_id, &[div_id]);

    let child_count = ctx.get_child_count(0);
    assert_eq!(
        child_count, 1,
        "mount point should have 1 child after append"
    );
}

#[test]
fn append_multiple_children() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let h1_id = ctx.create_element("h1");
    let p_id = ctx.create_element("p");
    let btn_id = ctx.create_element("button");

    ctx.assign_id(1, h1_id);
    ctx.assign_id(2, p_id);
    ctx.assign_id(3, btn_id);

    ctx.append_children(body_blitz_id, &[h1_id, p_id, btn_id]);

    assert_eq!(ctx.get_child_count(0), 3);
    assert_eq!(ctx.get_child_mojo_id(0, 0), 1);
    assert_eq!(ctx.get_child_mojo_id(0, 1), 2);
    assert_eq!(ctx.get_child_mojo_id(0, 2), 3);
}

#[test]
fn nested_tree_structure() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Build: <div><h1><text>"Hello"</text></h1><button><text>"Click"</text></button></div>
    let div_id = ctx.create_element("div");
    let h1_id = ctx.create_element("h1");
    let btn_id = ctx.create_element("button");
    let text1_id = ctx.create_text_node("Hello");
    let text2_id = ctx.create_text_node("Click");

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, h1_id);
    ctx.assign_id(3, btn_id);
    ctx.assign_id(4, text1_id);
    ctx.assign_id(5, text2_id);

    ctx.append_children(h1_id, &[text1_id]);
    ctx.append_children(btn_id, &[text2_id]);
    ctx.append_children(div_id, &[h1_id, btn_id]);
    ctx.append_children(body_blitz_id, &[div_id]);

    // Verify structure
    assert_eq!(ctx.get_child_count(0), 1); // body has 1 child (div)
    assert_eq!(ctx.get_child_count(1), 2); // div has 2 children (h1, button)
    assert_eq!(ctx.get_child_count(2), 1); // h1 has 1 child (text)
    assert_eq!(ctx.get_child_count(3), 1); // button has 1 child (text)

    // Verify text content traverses down
    assert_eq!(ctx.get_text_content(2), "Hello");
    assert_eq!(ctx.get_text_content(3), "Click");
    assert_eq!(ctx.get_text_content(1), "HelloClick"); // div collects all descendant text
}

#[test]
fn insert_before() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let p1_id = ctx.create_element("p");
    let p2_id = ctx.create_element("p");
    let p_new_id = ctx.create_element("h2");

    ctx.assign_id(1, p1_id);
    ctx.assign_id(2, p2_id);
    ctx.assign_id(3, p_new_id);

    ctx.append_children(body_blitz_id, &[p1_id, p2_id]);
    assert_eq!(ctx.get_child_count(0), 2);

    // Insert h2 before p2
    ctx.insert_before(p2_id, &[p_new_id]);
    assert_eq!(ctx.get_child_count(0), 3);

    // Order should be: p1, h2, p2
    assert_eq!(ctx.get_child_mojo_id(0, 0), 1); // p1
    assert_eq!(ctx.get_child_mojo_id(0, 1), 3); // h2
    assert_eq!(ctx.get_child_mojo_id(0, 2), 2); // p2
}

#[test]
fn insert_after() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let p1_id = ctx.create_element("p");
    let p2_id = ctx.create_element("p");
    let p_new_id = ctx.create_element("h2");

    ctx.assign_id(1, p1_id);
    ctx.assign_id(2, p2_id);
    ctx.assign_id(3, p_new_id);

    ctx.append_children(body_blitz_id, &[p1_id, p2_id]);

    // Insert h2 after p1
    ctx.insert_after(p1_id, &[p_new_id]);
    assert_eq!(ctx.get_child_count(0), 3);

    // Order should be: p1, h2, p2
    assert_eq!(ctx.get_child_mojo_id(0, 0), 1); // p1
    assert_eq!(ctx.get_child_mojo_id(0, 1), 3); // h2
    assert_eq!(ctx.get_child_mojo_id(0, 2), 2); // p2
}

#[test]
fn replace_with() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let old_id = ctx.create_element("p");
    let new_id = ctx.create_element("div");

    ctx.assign_id(1, old_id);
    ctx.assign_id(2, new_id);

    ctx.append_children(body_blitz_id, &[old_id]);
    assert_eq!(ctx.get_child_count(0), 1);
    assert_eq!(ctx.get_child_mojo_id(0, 0), 1); // p

    // Replace p with div
    ctx.replace_with(old_id, &[new_id]);
    assert_eq!(ctx.get_child_count(0), 1);
    assert_eq!(ctx.get_child_mojo_id(0, 0), 2); // div
}

#[test]
fn replace_with_multiple_nodes() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let old_id = ctx.create_element("p");
    let new1_id = ctx.create_element("h1");
    let new2_id = ctx.create_element("h2");

    ctx.assign_id(1, old_id);
    ctx.assign_id(2, new1_id);
    ctx.assign_id(3, new2_id);

    ctx.append_children(body_blitz_id, &[old_id]);

    ctx.replace_with(old_id, &[new1_id, new2_id]);
    assert_eq!(ctx.get_child_count(0), 2);
    assert_eq!(ctx.get_child_mojo_id(0, 0), 2); // h1
    assert_eq!(ctx.get_child_mojo_id(0, 1), 3); // h2
}

#[test]
fn remove_node() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);

    ctx.append_children(body_blitz_id, &[div_id]);
    assert_eq!(ctx.get_child_count(0), 1);

    ctx.remove_node(div_id);
    assert_eq!(ctx.get_child_count(0), 0);
}

#[test]
fn remove_node_cleans_up_id_mappings() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);
    ctx.append_children(body_blitz_id, &[div_id]);

    // ID 1 should resolve before removal
    assert!(ctx.resolve_id(1).is_some());

    ctx.remove_node(div_id);

    // ID 1 should no longer resolve after removal
    assert!(ctx.resolve_id(1).is_none());
}

#[test]
fn placeholder_node() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let placeholder_id = ctx.create_placeholder();
    ctx.assign_id(1, placeholder_id);

    ctx.append_children(body_blitz_id, &[placeholder_id]);

    let tag = ctx.get_node_tag(1);
    assert_eq!(tag, "#comment", "placeholders should be comment nodes");
    assert_eq!(ctx.get_child_count(0), 1);
}

#[test]
fn replace_placeholder_with_element() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let placeholder_id = ctx.create_placeholder();
    ctx.assign_id(1, placeholder_id);
    ctx.append_children(body_blitz_id, &[placeholder_id]);

    // Replace placeholder with a real element
    let div_id = ctx.create_element("div");
    ctx.assign_id(2, div_id);
    ctx.replace_with(placeholder_id, &[div_id]);

    assert_eq!(ctx.get_child_count(0), 1);
    assert_eq!(ctx.get_child_mojo_id(0, 0), 2);
    assert_eq!(ctx.get_node_tag(2), "div");
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. Template registration & cloning
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn register_and_clone_template() {
    let mut ctx = headless();

    // Build a template: <div><h1></h1><button></button></div>
    let tmpl_div = ctx.create_element("div");
    let tmpl_h1 = ctx.create_element("h1");
    let tmpl_btn = ctx.create_element("button");

    ctx.append_children(tmpl_div, &[tmpl_h1, tmpl_btn]);

    // Register as template ID 0
    ctx.templates.insert(0, tmpl_div);

    // Clone it
    let clone_id = ctx.deep_clone_node(tmpl_div);
    ctx.assign_id(1, clone_id);

    // Verify the clone has the same structure
    assert_eq!(ctx.get_node_tag(1), "div");
    // The clone's children won't have mojo IDs assigned, but we can check
    // the underlying Blitz node's children count
    let clone_node = ctx.doc.get_node(clone_id).unwrap();
    assert_eq!(clone_node.children.len(), 2);
}

#[test]
fn clone_template_creates_independent_copy() {
    let mut ctx = headless();

    // Build template: <p><text>"template"</text></p>
    let tmpl_p = ctx.create_element("p");
    let tmpl_text = ctx.create_text_node("template");
    ctx.append_children(tmpl_p, &[tmpl_text]);

    ctx.templates.insert(0, tmpl_p);

    // Clone twice
    let clone1 = ctx.deep_clone_node(tmpl_p);
    let clone2 = ctx.deep_clone_node(tmpl_p);

    assert_ne!(clone1, clone2, "clones should be different slab IDs");

    ctx.assign_id(1, clone1);
    ctx.assign_id(2, clone2);

    // Both should have "template" text
    assert_eq!(ctx.get_text_content(1), "template");
    assert_eq!(ctx.get_text_content(2), "template");

    // Modifying clone1's text shouldn't affect clone2
    let clone1_node = ctx.doc.get_node(clone1).unwrap();
    let clone1_text_id = clone1_node.children[0];
    ctx.set_text_content(clone1_text_id, "modified");

    assert_eq!(ctx.get_text_content(1), "modified");
    assert_eq!(ctx.get_text_content(2), "template");
}

#[test]
fn node_at_path_traverses_children() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Build: body > div > [h1, p > [span]]
    let div_id = ctx.create_element("div");
    let h1_id = ctx.create_element("h1");
    let p_id = ctx.create_element("p");
    let span_id = ctx.create_element("span");

    ctx.append_children(p_id, &[span_id]);
    ctx.append_children(div_id, &[h1_id, p_id]);
    ctx.append_children(body_blitz_id, &[div_id]);

    // Navigate: body[0] = div
    let result = ctx.node_at_path(body_blitz_id, &[0]);
    assert_eq!(result, div_id);

    // Navigate: body[0][0] = h1
    let result = ctx.node_at_path(body_blitz_id, &[0, 0]);
    assert_eq!(result, h1_id);

    // Navigate: body[0][1] = p
    let result = ctx.node_at_path(body_blitz_id, &[0, 1]);
    assert_eq!(result, p_id);

    // Navigate: body[0][1][0] = span
    let result = ctx.node_at_path(body_blitz_id, &[0, 1, 0]);
    assert_eq!(result, span_id);
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. Event injection & polling
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn inject_and_poll_click_event() {
    let mut ctx = headless();

    ctx.queue_event(42, 0 /* EVT_CLICK */, String::new());

    let event = ctx.poll_event();
    assert!(event.is_some());
    let event = event.unwrap();
    assert_eq!(event.handler_id, 42);
    assert_eq!(event.event_type, 0);
    assert!(event.value.is_empty());
}

#[test]
fn inject_and_poll_input_event_with_value() {
    let mut ctx = headless();

    ctx.queue_event(7, 1 /* EVT_INPUT */, "hello".to_string());

    let event = ctx.poll_event();
    assert!(event.is_some());
    let event = event.unwrap();
    assert_eq!(event.handler_id, 7);
    assert_eq!(event.event_type, 1);
    assert_eq!(event.value, "hello");
}

#[test]
fn poll_empty_queue_returns_none() {
    let mut ctx = headless();
    assert!(ctx.poll_event().is_none());
}

#[test]
fn multiple_events_polled_in_order() {
    let mut ctx = headless();

    ctx.queue_event(1, 0, "first".to_string());
    ctx.queue_event(2, 0, "second".to_string());
    ctx.queue_event(3, 0, "third".to_string());

    let e1 = ctx.poll_event().unwrap();
    assert_eq!(e1.handler_id, 1);
    assert_eq!(e1.value, "first");

    let e2 = ctx.poll_event().unwrap();
    assert_eq!(e2.handler_id, 2);
    assert_eq!(e2.value, "second");

    let e3 = ctx.poll_event().unwrap();
    assert_eq!(e3.handler_id, 3);
    assert_eq!(e3.value, "third");

    assert!(ctx.poll_event().is_none());
}

#[test]
fn event_handler_registration_and_lookup() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let btn_id = ctx.create_element("button");
    ctx.assign_id(1, btn_id);
    ctx.append_children(body_blitz_id, &[btn_id]);

    // Register a click handler
    ctx.add_event_listener(btn_id, 10, "click");

    // Verify handler exists
    let handlers = ctx.event_handlers.get(&btn_id);
    assert!(handlers.is_some());
    assert_eq!(handlers.unwrap().len(), 1);
    assert_eq!(handlers.unwrap()[0].handler_id, 10);
    assert_eq!(handlers.unwrap()[0].event_name, "click");
}

#[test]
fn event_handler_removal() {
    let mut ctx = headless();
    let btn_id = ctx.create_element("button");

    ctx.add_event_listener(btn_id, 10, "click");
    ctx.add_event_listener(btn_id, 11, "input");
    assert_eq!(ctx.event_handlers.get(&btn_id).unwrap().len(), 2);

    ctx.remove_event_listener(btn_id, "click");
    assert_eq!(ctx.event_handlers.get(&btn_id).unwrap().len(), 1);
    assert_eq!(
        ctx.event_handlers.get(&btn_id).unwrap()[0].event_name,
        "input"
    );

    ctx.remove_event_listener(btn_id, "input");
    // After removing all handlers, the entry should be cleaned up
    assert!(ctx.event_handlers.get(&btn_id).is_none());
}

#[test]
fn inject_event_with_unicode_value() {
    let mut ctx = headless();

    ctx.queue_event(1, 1, "Héllo 🌍".to_string());

    let event = ctx.poll_event().unwrap();
    assert_eq!(event.value, "Héllo 🌍");
}

// ═══════════════════════════════════════════════════════════════════════════
// 8. Mutation batch markers
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn mutation_batch_flag() {
    let mut ctx = headless();

    assert!(!ctx.in_mutation_batch, "should start outside batch");

    ctx.in_mutation_batch = true;
    assert!(ctx.in_mutation_batch);

    ctx.in_mutation_batch = false;
    assert!(!ctx.in_mutation_batch);
}

// ═══════════════════════════════════════════════════════════════════════════
// 9. DOM serialization
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn serialize_empty_mount_point() {
    let ctx = headless();
    let html = ctx.serialize_subtree(0);
    assert_eq!(html, "<body></body>");
}

#[test]
fn serialize_single_child() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);
    ctx.append_children(body_blitz_id, &[div_id]);

    let html = ctx.serialize_subtree(0);
    assert_eq!(html, "<body><div></div></body>");
}

#[test]
fn serialize_element_with_text() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let h1_id = ctx.create_element("h1");
    let text_id = ctx.create_text_node("Hello");
    ctx.assign_id(1, h1_id);

    ctx.append_children(h1_id, &[text_id]);
    ctx.append_children(body_blitz_id, &[h1_id]);

    let html = ctx.serialize_subtree(0);
    assert_eq!(html, r#"<body><h1>#text("Hello")</h1></body>"#);
}

#[test]
fn serialize_nested_tree() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Build: <div><h1>Hello</h1><button>Click</button></div>
    let div_id = ctx.create_element("div");
    let h1_id = ctx.create_element("h1");
    let btn_id = ctx.create_element("button");
    let text1_id = ctx.create_text_node("Hello");
    let text2_id = ctx.create_text_node("Click");

    ctx.assign_id(1, div_id);

    ctx.append_children(h1_id, &[text1_id]);
    ctx.append_children(btn_id, &[text2_id]);
    ctx.append_children(div_id, &[h1_id, btn_id]);
    ctx.append_children(body_blitz_id, &[div_id]);

    let html = ctx.serialize_subtree(0);
    assert_eq!(
        html,
        r#"<body><div><h1>#text("Hello")</h1><button>#text("Click")</button></div></body>"#
    );
}

#[test]
fn serialize_element_with_attributes() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let div_id = ctx.create_element("div");
    ctx.assign_id(1, div_id);
    ctx.set_attribute(div_id, "class", "container");
    ctx.set_attribute(div_id, "id", "main");

    ctx.append_children(body_blitz_id, &[div_id]);

    let html = ctx.serialize_subtree(0);
    // Attributes may come in any order, so check both
    assert!(
        html.contains(r#"class="container""#),
        "serialized HTML should contain class attribute: {html}"
    );
    assert!(
        html.contains(r#"id="main""#),
        "serialized HTML should contain id attribute: {html}"
    );
}

#[test]
fn serialize_placeholder() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let placeholder_id = ctx.create_placeholder();
    ctx.assign_id(1, placeholder_id);
    ctx.append_children(body_blitz_id, &[placeholder_id]);

    let html = ctx.serialize_subtree(0);
    assert_eq!(html, "<body><!----></body>");
}

#[test]
fn serialize_subtree_of_child() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let div_id = ctx.create_element("div");
    let p_id = ctx.create_element("p");
    let text_id = ctx.create_text_node("Inner");

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, p_id);

    ctx.append_children(p_id, &[text_id]);
    ctx.append_children(div_id, &[p_id]);
    ctx.append_children(body_blitz_id, &[div_id]);

    // Serialize just the div subtree
    let html = ctx.serialize_subtree(1);
    assert_eq!(html, r#"<div><p>#text("Inner")</p></div>"#);

    // Serialize just the p subtree
    let html = ctx.serialize_subtree(2);
    assert_eq!(html, r#"<p>#text("Inner")</p>"#);
}

#[test]
fn serialize_text_with_quotes() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let p_id = ctx.create_element("p");
    let text_id = ctx.create_text_node(r#"He said "hello""#);

    ctx.assign_id(1, p_id);

    ctx.append_children(p_id, &[text_id]);
    ctx.append_children(body_blitz_id, &[p_id]);

    let html = ctx.serialize_subtree(1);
    assert_eq!(html, r#"<p>#text("He said \"hello\"")</p>"#);
}

#[test]
fn serialize_nonexistent_node_returns_empty() {
    let ctx = headless();
    let html = ctx.serialize_subtree(999);
    assert!(html.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════
// 10. Node ID mapping & stack operations
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn assign_id_creates_bidirectional_mapping() {
    let mut ctx = headless();
    let div_id = ctx.create_element("div");

    ctx.assign_id(42, div_id);

    // mojo → blitz
    assert_eq!(ctx.resolve_id(42), Some(div_id));

    // blitz → mojo
    assert_eq!(ctx.node_to_id.get(&div_id), Some(&42));
}

#[test]
fn stack_push_and_pop() {
    let mut ctx = headless();

    ctx.stack_push(10);
    ctx.stack_push(20);
    ctx.stack_push(30);

    let popped = ctx.stack_pop_n(2);
    assert_eq!(popped, vec![20, 30]);
    assert_eq!(ctx.stack.len(), 1);

    let remaining = ctx.stack_pop_n(1);
    assert_eq!(remaining, vec![10]);
    assert!(ctx.stack.is_empty());
}

#[test]
fn stack_pop_n_with_zero() {
    let mut ctx = headless();
    ctx.stack_push(10);
    let popped = ctx.stack_pop_n(0);
    assert!(popped.is_empty());
    assert_eq!(ctx.stack.len(), 1);
}

#[test]
fn stack_pop_n_more_than_available() {
    let mut ctx = headless();
    ctx.stack_push(10);

    let popped = ctx.stack_pop_n(5);
    // saturating_sub ensures we don't panic; we get whatever is available
    assert_eq!(popped, vec![10]);
    assert!(ctx.stack.is_empty());
}

#[test]
fn get_child_mojo_id_out_of_bounds() {
    let ctx = headless();
    // Mount point has no children
    let result = ctx.get_child_mojo_id(0, 0);
    assert_eq!(result, u32::MAX);
}

#[test]
fn get_child_mojo_id_unmapped_child() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Create an element but DON'T assign it a mojo ID
    let div_id = ctx.create_element("div");
    ctx.append_children(body_blitz_id, &[div_id]);

    // Child exists but has no mojo ID mapping
    let result = ctx.get_child_mojo_id(0, 0);
    assert_eq!(result, u32::MAX);
}

#[test]
fn get_child_count_on_nonexistent_node() {
    let ctx = headless();
    let count = ctx.get_child_count(999);
    assert_eq!(count, 0);
}

// ═══════════════════════════════════════════════════════════════════════════
// 11. DOM inspection helpers
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn get_node_tag_for_various_types() {
    let mut ctx = headless();

    let div_id = ctx.create_element("div");
    let text_id = ctx.create_text_node("hello");
    let comment_id = ctx.create_placeholder();

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, text_id);
    ctx.assign_id(3, comment_id);

    assert_eq!(ctx.get_node_tag(1), "div");
    assert_eq!(ctx.get_node_tag(2), "#text");
    assert_eq!(ctx.get_node_tag(3), "#comment");
}

#[test]
fn get_node_tag_nonexistent_returns_empty() {
    let ctx = headless();
    assert!(ctx.get_node_tag(999).is_empty());
}

#[test]
fn get_text_content_recursive() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Build: <div><span>Hello </span><span>World</span></div>
    let div_id = ctx.create_element("div");
    let span1 = ctx.create_element("span");
    let span2 = ctx.create_element("span");
    let t1 = ctx.create_text_node("Hello ");
    let t2 = ctx.create_text_node("World");

    ctx.assign_id(1, div_id);

    ctx.append_children(span1, &[t1]);
    ctx.append_children(span2, &[t2]);
    ctx.append_children(div_id, &[span1, span2]);
    ctx.append_children(body_blitz_id, &[div_id]);

    assert_eq!(ctx.get_text_content(1), "Hello World");
}

#[test]
fn get_text_content_nonexistent_returns_empty() {
    let ctx = headless();
    assert!(ctx.get_text_content(999).is_empty());
}

#[test]
fn get_attribute_value_on_text_node_returns_none() {
    let mut ctx = headless();
    let text_id = ctx.create_text_node("hello");
    ctx.assign_id(1, text_id);

    assert_eq!(ctx.get_attribute_value(1, "class"), None);
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. Counter-like integration scenario
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn counter_like_mount_and_update() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Simulate mounting a counter app's DOM:
    //   <div>
    //     <h1>"High-Five counter: 0"</h1>
    //     <button>"Up high!"</button>
    //     <button>"Down low!"</button>
    //   </div>
    let div_id = ctx.create_element("div");
    let h1_id = ctx.create_element("h1");
    let btn1_id = ctx.create_element("button");
    let btn2_id = ctx.create_element("button");
    let count_text = ctx.create_text_node("High-Five counter: 0");
    let btn1_text = ctx.create_text_node("Up high!");
    let btn2_text = ctx.create_text_node("Down low!");

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, h1_id);
    ctx.assign_id(3, btn1_id);
    ctx.assign_id(4, btn2_id);
    ctx.assign_id(5, count_text);

    ctx.append_children(h1_id, &[count_text]);
    ctx.append_children(btn1_id, &[btn1_text]);
    ctx.append_children(btn2_id, &[btn2_text]);
    ctx.append_children(div_id, &[h1_id, btn1_id, btn2_id]);
    ctx.append_children(body_blitz_id, &[div_id]);

    // Register click handlers
    ctx.add_event_listener(btn1_id, 10, "click");
    ctx.add_event_listener(btn2_id, 11, "click");

    // Verify initial state
    assert_eq!(ctx.get_text_content(2), "High-Five counter: 0");
    assert_eq!(ctx.get_node_tag(1), "div");
    assert_eq!(ctx.get_child_count(1), 3);

    // Simulate "Up high!" click → inject event → update text
    ctx.queue_event(10, 0, String::new()); // click on handler 10

    let event = ctx.poll_event().unwrap();
    assert_eq!(event.handler_id, 10);

    // App would handle this by incrementing count and flushing...
    // Simulate the SetText mutation:
    ctx.set_text_content(count_text, "High-Five counter: 1");

    assert_eq!(ctx.get_text_content(2), "High-Five counter: 1");

    // Simulate multiple clicks
    for i in 2..=5 {
        ctx.set_text_content(count_text, &format!("High-Five counter: {i}"));
    }
    assert_eq!(ctx.get_text_content(2), "High-Five counter: 5");

    // Verify full tree serialization
    let html = ctx.serialize_subtree(1);
    assert!(
        html.contains("High-Five counter: 5"),
        "serialized tree should contain updated count: {html}"
    );
    assert!(
        html.contains("Up high!"),
        "serialized tree should contain button text: {html}"
    );
    assert!(
        html.contains("Down low!"),
        "serialized tree should contain button text: {html}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 13. Conditional rendering scenario (placeholder swap)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn conditional_rendering_placeholder_swap() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Initial: <div><p>Content</p><!----></div>
    let div_id = ctx.create_element("div");
    let p_id = ctx.create_element("p");
    let p_text = ctx.create_text_node("Content");
    let placeholder = ctx.create_placeholder();

    ctx.assign_id(1, div_id);
    ctx.assign_id(2, p_id);
    ctx.assign_id(3, placeholder);

    ctx.append_children(p_id, &[p_text]);
    ctx.append_children(div_id, &[p_id, placeholder]);
    ctx.append_children(body_blitz_id, &[div_id]);

    assert_eq!(ctx.get_child_count(1), 2);
    let html = ctx.serialize_subtree(1);
    assert!(html.contains("<!---->"), "should have placeholder: {html}");

    // Show detail: replace placeholder with <div class="detail"><p>Detail</p></div>
    let detail_div = ctx.create_element("div");
    let detail_p = ctx.create_element("p");
    let detail_text = ctx.create_text_node("Detail info");

    ctx.assign_id(4, detail_div);
    ctx.set_attribute(detail_div, "class", "detail");
    ctx.append_children(detail_p, &[detail_text]);
    ctx.append_children(detail_div, &[detail_p]);

    ctx.replace_with(placeholder, &[detail_div]);

    assert_eq!(ctx.get_child_count(1), 2);
    let html = ctx.serialize_subtree(1);
    assert!(
        !html.contains("<!---->"),
        "placeholder should be gone: {html}"
    );
    assert!(
        html.contains("Detail info"),
        "detail content should appear: {html}"
    );

    // Hide detail: replace detail div with new placeholder
    let new_placeholder = ctx.create_placeholder();
    ctx.assign_id(5, new_placeholder);
    ctx.replace_with(detail_div, &[new_placeholder]);

    assert_eq!(ctx.get_child_count(1), 2);
    let html = ctx.serialize_subtree(1);
    assert!(
        html.contains("<!---->"),
        "placeholder should be back: {html}"
    );
    assert!(
        !html.contains("Detail info"),
        "detail should be gone: {html}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 14. Todo-like list scenario
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn todo_list_add_and_remove_items() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // <ul id=1></ul>
    let ul_id = ctx.create_element("ul");
    ctx.assign_id(1, ul_id);
    ctx.append_children(body_blitz_id, &[ul_id]);

    assert_eq!(ctx.get_child_count(1), 0);

    // Add 3 items
    for i in 0..3 {
        let li = ctx.create_element("li");
        let text = ctx.create_text_node(&format!("Item {i}"));
        ctx.assign_id(10 + i, li);
        ctx.append_children(li, &[text]);
        ctx.append_children(ul_id, &[li]);
    }

    assert_eq!(ctx.get_child_count(1), 3);
    assert_eq!(ctx.get_text_content(10), "Item 0");
    assert_eq!(ctx.get_text_content(11), "Item 1");
    assert_eq!(ctx.get_text_content(12), "Item 2");

    // Remove the middle item
    let middle_blitz_id = ctx.resolve_id(11).unwrap();
    ctx.remove_node(middle_blitz_id);

    assert_eq!(ctx.get_child_count(1), 2);
    assert_eq!(ctx.get_child_mojo_id(1, 0), 10); // Item 0
    assert_eq!(ctx.get_child_mojo_id(1, 1), 12); // Item 2

    // Verify text content of remaining items
    let html = ctx.serialize_subtree(1);
    assert!(html.contains("Item 0"), "Item 0 should remain: {html}");
    assert!(!html.contains("Item 1"), "Item 1 should be gone: {html}");
    assert!(html.contains("Item 2"), "Item 2 should remain: {html}");
}

// ═══════════════════════════════════════════════════════════════════════════
// 15. Stress / edge cases
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn many_children() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let container = ctx.create_element("div");
    ctx.assign_id(1, container);
    ctx.append_children(body_blitz_id, &[container]);

    // Add 100 children
    for i in 0..100u32 {
        let p = ctx.create_element("p");
        let text = ctx.create_text_node(&format!("P{i}"));
        ctx.assign_id(100 + i, p);
        ctx.append_children(p, &[text]);
        ctx.append_children(container, &[p]);
    }

    assert_eq!(ctx.get_child_count(1), 100);
    assert_eq!(ctx.get_text_content(100), "P0");
    assert_eq!(ctx.get_text_content(199), "P99");
}

#[test]
fn deeply_nested_tree() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    // Build a 20-deep nesting: div > div > div > ... > p > "deep"
    let mut parent = body_blitz_id;
    let mut last_div_id = 0;
    for i in 0..20u32 {
        let div = ctx.create_element("div");
        ctx.assign_id(i + 1, div);
        ctx.append_children(parent, &[div]);
        parent = div;
        last_div_id = i + 1;
    }

    let p = ctx.create_element("p");
    let text = ctx.create_text_node("deep");
    ctx.assign_id(21, p);
    ctx.append_children(p, &[text]);
    ctx.append_children(parent, &[p]);

    // Verify we can still read the leaf
    assert_eq!(ctx.get_text_content(21), "deep");

    // Verify the entire tree has the text when traversed from the root
    assert_eq!(ctx.get_text_content(0), "deep");

    // Verify from the innermost div
    assert_eq!(ctx.get_text_content(last_div_id), "deep");
}

#[test]
fn rapid_text_updates() {
    let mut ctx = headless();
    let body_blitz_id = ctx.resolve_id(0).unwrap();

    let p = ctx.create_element("p");
    let text = ctx.create_text_node("");
    ctx.assign_id(1, p);
    ctx.assign_id(2, text);
    ctx.append_children(p, &[text]);
    ctx.append_children(body_blitz_id, &[p]);

    let text_blitz_id = ctx.resolve_id(2).unwrap();

    // Simulate 1000 rapid text updates (like a counter being spammed)
    for i in 0..1000 {
        ctx.set_text_content(text_blitz_id, &format!("Count: {i}"));
    }

    assert_eq!(ctx.get_text_content(1), "Count: 999");
}

#[test]
fn reassign_id_updates_mapping() {
    let mut ctx = headless();

    let div1 = ctx.create_element("div");
    let div2 = ctx.create_element("span");

    ctx.assign_id(1, div1);
    assert_eq!(ctx.get_node_tag(1), "div");

    // Reassign ID 1 to a different node
    ctx.assign_id(1, div2);
    assert_eq!(ctx.get_node_tag(1), "span");
}

// ═══════════════════════════════════════════════════════════════════════════
// 12. FFI poll_event_into (output-pointer polling)
// ═══════════════════════════════════════════════════════════════════════════
//
// These tests exercise the `mblitz_poll_event_into` C FFI function that
// the Mojo `Blitz.poll_event()` method calls. This is the code path that
// was broken (the old stub always returned invalid events), so we test it
// thoroughly here.

#[test]
fn poll_event_into_empty_queue_returns_zero() {
    let mut ctx = headless();
    let mut handler_id: u32 = 0xFF;
    let mut event_type: u8 = 0xFF;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0xFF;

    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };

    assert_eq!(valid, 0, "empty queue should return 0");
    // Output pointers should not be modified when queue is empty
    assert_eq!(handler_id, 0xFF);
    assert_eq!(event_type, 0xFF);
}

#[test]
fn poll_event_into_click_event() {
    let mut ctx = headless();
    ctx.queue_event(42, 0 /* EVT_CLICK */, String::new());

    let mut handler_id: u32 = 0;
    let mut event_type: u8 = 0xFF;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0xFF;

    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };

    assert_eq!(valid, 1, "should return 1 for available event");
    assert_eq!(handler_id, 42);
    assert_eq!(event_type, 0); // EVT_CLICK
    assert!(value_ptr.is_null(), "click events have no string value");
    assert_eq!(value_len, 0);
}

#[test]
fn poll_event_into_input_event_with_value() {
    let mut ctx = headless();
    ctx.queue_event(7, 1 /* EVT_INPUT */, "hello".to_string());

    let mut handler_id: u32 = 0;
    let mut event_type: u8 = 0;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0;

    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };

    assert_eq!(valid, 1);
    assert_eq!(handler_id, 7);
    assert_eq!(event_type, 1); // EVT_INPUT
    assert!(
        !value_ptr.is_null(),
        "input events should have a value pointer"
    );
    assert_eq!(value_len, 5);

    // Read the string value from the pointer
    let value = unsafe { std::slice::from_raw_parts(value_ptr, value_len as usize) };
    assert_eq!(std::str::from_utf8(value).unwrap(), "hello");
}

#[test]
fn poll_event_into_multiple_events_in_order() {
    let mut ctx = headless();
    ctx.queue_event(1, 0, String::new());
    ctx.queue_event(2, 1, "second".to_string());
    ctx.queue_event(3, 0, String::new());

    // Poll first event
    let mut handler_id: u32 = 0;
    let mut event_type: u8 = 0;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0;

    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 1);
    assert_eq!(handler_id, 1);

    // Poll second event (has string value)
    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 1);
    assert_eq!(handler_id, 2);
    assert_eq!(event_type, 1);
    assert_eq!(value_len, 6);
    let value = unsafe { std::slice::from_raw_parts(value_ptr, value_len as usize) };
    assert_eq!(std::str::from_utf8(value).unwrap(), "second");

    // Poll third event
    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 1);
    assert_eq!(handler_id, 3);

    // Queue should now be empty
    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 0, "queue should be empty after draining all events");
}

#[test]
fn poll_event_into_unicode_value() {
    let mut ctx = headless();
    ctx.queue_event(99, 1, "héllo 🌍".to_string());

    let mut handler_id: u32 = 0;
    let mut event_type: u8 = 0;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0;

    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };

    assert_eq!(valid, 1);
    assert_eq!(handler_id, 99);
    assert!(!value_ptr.is_null());
    assert!(value_len > 0);

    let value = unsafe { std::slice::from_raw_parts(value_ptr, value_len as usize) };
    assert_eq!(std::str::from_utf8(value).unwrap(), "héllo 🌍");
}

#[test]
fn poll_event_into_consumes_event() {
    let mut ctx = headless();
    ctx.queue_event(1, 0, String::new());

    let mut handler_id: u32 = 0;
    let mut event_type: u8 = 0;
    let mut value_ptr: *const u8 = std::ptr::null();
    let mut value_len: u32 = 0;

    // First poll should succeed
    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 1);

    // Second poll should return empty — event was consumed
    let valid = unsafe {
        mblitz_poll_event_into(
            &mut ctx,
            &mut handler_id,
            &mut event_type,
            &mut value_ptr,
            &mut value_len,
        )
    };
    assert_eq!(valid, 0, "event should be consumed after first poll");
}
