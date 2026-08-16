# mojo-gui/core — Renderer-agnostic reactive GUI framework.
#
# This is the package root for the core library. It re-exports the public API
# surface so that downstream consumers (web, desktop, native renderers) can
# import from a single top-level package.
#
# Sub-packages:
#   signals/    — Reactive primitives (signals, memos, effects)
#   scope/      — Scope lifecycle and arena allocator
#   scheduler/  — Height-ordered dirty scope queue
#   arena/      — ElementId type and allocator
#   vdom/       — Virtual DOM (Template, VNode, diff) — renderer-agnostic
#   mutations/  — CreateEngine, DiffEngine (VNode → mutation buffer)
#   bridge/     — MutationWriter + binary opcode protocol
#   events/     — HandlerRegistry and action tags
#   component/  — AppShell, ComponentContext, lifecycle, KeyedList, Router
#   html/       — HTML vocabulary: tags, DSL element constructors, VNodeBuilder
#   platform/   — Platform abstraction: PlatformApp trait, launch(), feature detection
