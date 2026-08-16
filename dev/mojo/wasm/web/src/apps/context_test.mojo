# ContextTestApp — context (DI) surface test (Phase 31.1).
#
# A minimal test app that exercises ComponentContext.provide_context(),
# consume_context(), and the typed signal-sharing helpers.  Has a root
# scope + one child scope so that parent-chain walk-up can be verified.

from memory import UnsafePointer, alloc
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

    fn __init__(out self):
        self.ctx = ComponentContext.create()
        self.count = self.ctx.use_signal(0)
        self.ctx.end_setup()
        # Create a child scope under the root
        self.child_scope_id = self.ctx.create_child_scope()

    fn __moveinit__(out self, deinit other: Self):
        self.ctx = other.ctx^
        self.child_scope_id = other.child_scope_id
        self.count = other.count^


fn _cta_init() -> UnsafePointer[ContextTestApp, MutExternalOrigin]:
    var app_ptr = alloc[ContextTestApp](1)
    app_ptr.init_pointee_move(ContextTestApp())
    return app_ptr


fn _cta_destroy(app_ptr: UnsafePointer[ContextTestApp, MutExternalOrigin]):
    # Destroy child scope
    var scope_ids = List[UInt32]()
    scope_ids.append(app_ptr[0].child_scope_id)
    app_ptr[0].ctx.destroy_child_scopes(scope_ids)
    app_ptr[0].ctx.destroy()
    app_ptr.destroy_pointee()
    app_ptr.free()
