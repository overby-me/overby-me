# ContextTestApp — context (DI) surface test (Phase 31.1).
#
# A minimal test app that exercises ComponentContext.provide_context(),
# consume_context(), and the typed signal-sharing helpers.  Has a root
# scope + one child scope so that parent-chain walk-up can be verified.

from std.memory import Pointer, alloc
from component import ComponentContext
from signals.handle import SignalI32 as _SignalI32


struct ContextTestApp(Movable):
    """Minimal app for testing ComponentContext context (DI) surface.

    Creates a root scope with a count signal, a child scope, and
    provides the count signal via context so the child can consume it.
    """

    var ctx: ComponentContext
    var child_scope_id: UInt32
    var count: _SignalI32

    def __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.end_setup()
        # Create a child scope under the root
        self.child_scope_id = self.ctx.create_child_scope()

    def __init__(out self, *, deinit move: Self):
        self.ctx = move.ctx^
        self.child_scope_id = move.child_scope_id
        self.count = move.count^


def _cta_init() -> Pointer[ContextTestApp, MutUntrackedOrigin]:
    var app_ptr = alloc[ContextTestApp](1)
    app_ptr.unsafe_write(ContextTestApp())
    return app_ptr


def _cta_destroy(app_ptr: Pointer[ContextTestApp, MutUntrackedOrigin]):
    # Destroy child scope
    var scope_ids = List[UInt32]()
    scope_ids.append(app_ptr[0].child_scope_id)
    app_ptr[0].ctx.destroy_child_scopes(scope_ids)
    app_ptr[0].ctx.destroy()
    app_ptr.unsafe_deinit_pointee()
    app_ptr.free()
