"""mojo-gui/desktop — Desktop renderer for mojo-gui applications.

This package provides a native desktop GUI backend using Blitz
(Stylo + Taffy + Vello + Winit + AccessKit). The core mojo-gui framework
writes binary mutations to a heap buffer, and the desktop renderer's
mutation interpreter translates them into Blitz DOM operations via C FFI.

Architecture:

    mojo-gui/core (MutationWriter)
        │ binary opcode buffer
        ▼
    desktop/renderer.mojo (MutationInterpreter)
        │ reads opcodes, calls Blitz FFI
        ▼
    desktop/blitz.mojo (Blitz FFI wrapper)
        │ DLHandle calls
        ▼
    libmojo_blitz.so (Rust cdylib — shim/src/lib.rs)
        │ Rust API calls
        ▼
    Blitz (blitz-dom, blitz-shell, blitz-paint)
        ├── Stylo     — CSS parsing & style resolution
        ├── Taffy     — Flexbox, grid, block layout
        ├── Parley    — Text layout & shaping
        ├── Vello     — GPU-accelerated 2D rendering
        ├── Winit     — Cross-platform windowing & input
        └── AccessKit — Accessibility

Modules:
    blitz:     Mojo FFI bindings to libmojo_blitz (Blitz C shim)
    renderer:  Mutation interpreter (binary opcodes → Blitz DOM ops)
    launcher:  desktop_launch[AppType: GuiApp]() — generic entry point

Usage (via the unified launch() entry point):

    from platform import launch, AppConfig
    from counter import CounterApp

    fn main() raises:
        launch[CounterApp](AppConfig(title="Counter", width=400, height=350))

    # On native targets, launch() calls desktop_launch[CounterApp](config)
    # which creates the Blitz window, mounts the DOM, and enters the event loop.

Usage (direct, for advanced control):

    from desktop.blitz import Blitz
    from desktop.renderer import MutationInterpreter
    from desktop.launcher import desktop_launch

    fn main() raises:
        desktop_launch[MyApp](AppConfig(title="My App"))
"""
