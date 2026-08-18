# Phase 6: `mojo-web` — Raw Web API Bindings

> **Status:** Future
>
> **Parent document:** [PLAN.md](../../PLAN.md)

---

## Purpose

`mojo-web` is a standalone Mojo library providing typed bindings to Web APIs for any Mojo/WASM project — the equivalent of Rust's `web-sys` crate. It is **not** part of `mojo-gui` and has no dependency on it.

---

## Architecture

Since Mojo lacks a `wasm-bindgen` equivalent, `mojo-web` uses the same pattern already proven in `mojo-wasm`: WASM imports backed by a JS-side handle table.

**JS side** — a runtime that exposes Web APIs as flat WASM-importable functions:

```typescript
// Handle table: maps integer IDs to JS objects
const handles = new Map<number, any>();
let nextId = 1;

export const mojo_web = {
  document_create_element(tag_ptr: bigint, tag_len: number): number {
    const tag = readString(tag_ptr, tag_len);
    const el = document.createElement(tag);
    handles.set(nextId, el);
    return nextId++;
  },
  node_append_child(parent: number, child: number): void {
    handles.get(parent)!.appendChild(handles.get(child)!);
  },
  handle_drop(id: number): void {
    handles.delete(id);
  },
  // ... more Web API bindings
};
```

**Mojo side** — typed wrappers over the imported functions:

```mojo
struct JsHandle(Movable):
    """Opaque handle to a JS object. Dropped via handle_drop() on the JS side."""
    var id: Int32

struct Element:
    var handle: JsHandle

    fn set_attribute(self, name: String, value: String):
        _web_sys_set_attribute(self.handle.id, name, value)

struct Document:
    fn create_element(self, tag: String) -> Element:
        var id = _web_sys_create_element(tag)
        return Element(JsHandle(id))
```

---

## API Surface (MVP — Phase 6)

| Module | Web APIs | Examples |
|--------|----------|----------|
| `dom` | Document, Element, Node, Text, Event | `document.create_element()`, `el.set_attribute()` |
| `fetch` | fetch, Request, Response, Headers | `fetch(url).await_response()` |
| `timers` | setTimeout, setInterval, requestAnimationFrame | `set_timeout(callback, ms)` |
| `storage` | localStorage, sessionStorage | `local_storage.get_item(key)` |
| `console` | console.log, warn, error | `console.log(msg)` |
| `url` | URL, URLSearchParams | `URL.parse(href)` |
| `websocket` | WebSocket | `WebSocket.connect(url)` |
| `canvas` | Canvas2D, WebGL (future) | `ctx.fill_rect(x, y, w, h)` |

---

## Relationship to `mojo-gui`

`mojo-web` and `mojo-gui` are **independent** projects:

- `mojo-gui` uses the binary mutation protocol for rendering — it does NOT use `mojo-web` for DOM manipulation. This keeps the multi-renderer architecture intact.
- `mojo-gui` apps can import `mojo-web` for **non-rendering** web features: data fetching (suspense + fetch), persistent storage, WebSocket connections, animation timers, etc.
- `mojo-web` can be used by any Mojo/WASM project that has nothing to do with `mojo-gui`.

**Important for shared examples:** If an example needs a web-only feature (e.g., `fetch`), it should use `mojo-web` behind a feature gate or platform check so the example still compiles on non-web targets. For most GUI examples (counter, todo, bench), no web-specific APIs are needed — they work identically on all targets.

```text
┌────────────────────────────────────────────────┐
│  Shared Example App                             │
│                                                 │
│  GUI rendering:     Non-rendering web features: │
│  mojo-gui/core      mojo-web (optional,         │
│  (mutation protocol) gated on web target)       │
└────────────────────────────────────────────────┘
```

---

## Project Structure

```text
mojo-web/
├── src/
│   ├── handle.mojo               # JsHandle — opaque reference to JS objects
│   ├── dom.mojo                  # Document, Element, Node, Text, Event
│   ├── fetch.mojo                # fetch(), Request, Response, Headers
│   ├── timers.mojo               # setTimeout, setInterval, requestAnimationFrame
│   ├── storage.mojo              # localStorage, sessionStorage
│   ├── console.mojo              # console.log/warn/error
│   ├── url.mojo                  # URL, URLSearchParams
│   ├── websocket.mojo            # WebSocket
│   └── lib.mojo                  # Package root
├── runtime/
│   └── mojo_web.ts               # JS-side handle table + Web API bindings
├── test/
│   └── ...
├── examples/
│   └── fetch_example.mojo        # Simple fetch + DOM example
└── README.md
```

---

## Estimated Effort

| Task | Effort |
|------|--------|
| Phase 6 MVP (handle table, DOM, fetch, timers, storage) | 2–3 weeks |