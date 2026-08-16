# Router — Client-side URL-based view switching for single-page apps.
#
# Maps URL paths to branch tags (UInt8) and manages the active view
# via a ConditionalSlot.  The app's flush function checks the router's
# current branch and builds the appropriate VNode for that route.
#
# Design:
#
#   var router = Router()
#   router.add_route(String("/"), 0)
#   router.add_route(String("/todo"), 1)
#   router.navigate(String("/"))       # sets current branch to 0
#   router.navigate(String("/todo"))   # sets current branch to 1
#
# Integration with ConditionalSlot:
#
#   The Router embeds a ConditionalSlot that manages the DOM lifecycle
#   of the active view.  When navigating:
#     - If the branch changes, the app rebuilds the view VNode for the
#       new branch and flushes it through the ConditionalSlot (which
#       handles create/diff/remove transitions automatically).
#     - If the branch is the same, the flush is a diff (incremental update).
#
# The Router does NOT own the view templates or VNodes — it only tracks
# which branch is active.  The app is responsible for:
#   1. Registering routes via add_route()
#   2. Checking router.current in its flush function
#   3. Building the correct VNode for the current branch
#   4. Passing the VNode to flush_conditional via the router's slot
#
# This keeps the Router generic and reusable across different app
# architectures (single-component, multi-component, etc.).

from .lifecycle import ConditionalSlot


# ══════════════════════════════════════════════════════════════════════════════
# RouteEntry — A single path → branch mapping
# ══════════════════════════════════════════════════════════════════════════════


struct RouteEntry(Copyable, Movable):
    """A route mapping from a URL path to a branch tag.

    Fields:
        path: The URL path (e.g. "/", "/todo", "/about").
        branch: The branch tag (0–255) identifying which view to render.
    """

    var path: String
    var branch: UInt8

    fn __init__(out self, path: String, branch: UInt8):
        self.path = path
        self.branch = branch

    fn __copyinit__(out self, other: Self):
        self.path = other.path
        self.branch = other.branch

    fn __moveinit__(out self, deinit other: Self):
        self.path = other.path^
        self.branch = other.branch


# ══════════════════════════════════════════════════════════════════════════════
# Router — URL path → branch tag router with ConditionalSlot
# ══════════════════════════════════════════════════════════════════════════════


struct Router(Movable):
    """Client-side router mapping URL paths to view branches.

    Tracks a list of routes (path → branch tag) and the currently active
    branch.  Embeds a ConditionalSlot for managing DOM transitions when
    the active route changes.

    The router supports up to 256 branches (UInt8 branch tags), which is
    more than sufficient for any single-page application.

    Fields:
        routes: List of RouteEntry mappings.
        current: The currently active branch tag (255 = no route matched).
        current_path: The URL path of the currently active route.
        slot: ConditionalSlot managing the DOM for the active view.
        dirty: True if navigate() changed the branch since last flush.
    """

    var routes: List[RouteEntry]
    var current: UInt8
    var current_path: String
    var slot: ConditionalSlot
    var dirty: Bool

    fn __init__(out self):
        """Create an empty router with no routes and no active branch."""
        self.routes = List[RouteEntry]()
        self.current = 255  # sentinel: no route matched
        self.current_path = String("")
        self.slot = ConditionalSlot()
        self.dirty = False

    fn __moveinit__(out self, deinit other: Self):
        self.routes = other.routes^
        self.current = other.current
        self.current_path = other.current_path^
        self.slot = other.slot.copy()
        self.dirty = other.dirty

    fn add_route(mut self, path: String, branch: UInt8):
        """Register a route mapping from a URL path to a branch tag.

        Routes are matched in registration order — first match wins.
        Register more specific paths before less specific ones if there
        is any ambiguity (though exact-match is used, so "/todo" and
        "/" are unambiguous).

        Args:
            path: The URL path to match (exact match, e.g. "/" or "/todo").
            branch: The branch tag (0–255) to activate when this path matches.
        """
        self.routes.append(RouteEntry(path, branch))

    fn navigate(mut self, path: String) -> Bool:
        """Navigate to the given URL path.

        Looks up the path in the route table (exact match, first wins).
        If found and the branch differs from the current one, updates
        the current branch and marks the router as dirty.

        If the path matches the already-active route, this is a no-op
        (returns True but does not mark dirty).

        Args:
            path: The URL path to navigate to.

        Returns:
            True if the path matched a registered route, False otherwise.
        """
        for i in range(len(self.routes)):
            if self.routes[i].path == path:
                var new_branch = self.routes[i].branch
                if new_branch != self.current:
                    self.current = new_branch
                    self.current_path = path
                    self.dirty = True
                elif self.current_path != path:
                    # Same branch but different path (shouldn't happen
                    # with well-formed routes, but handle gracefully)
                    self.current_path = path
                return True
        return False

    fn consume_dirty(mut self) -> Bool:
        """Check and clear the dirty flag.

        Returns True if navigate() changed the branch since the last
        call to consume_dirty().  Used by the app's flush function to
        determine whether to rebuild the view.

        Returns:
            True if the route changed and needs a DOM update.
        """
        if self.dirty:
            self.dirty = False
            return True
        return False

    fn route_count(self) -> Int:
        """Return the number of registered routes."""
        return len(self.routes)

    fn has_route(self, path: String) -> Bool:
        """Check whether a path has a registered route.

        Args:
            path: The URL path to check.

        Returns:
            True if the path matches a registered route.
        """
        for i in range(len(self.routes)):
            if self.routes[i].path == path:
                return True
        return False

    fn branch_for(self, path: String) -> Int:
        """Return the branch tag for a path, or -1 if not found.

        Args:
            path: The URL path to look up.

        Returns:
            The branch tag (0–255) as Int, or -1 if no route matches.
        """
        for i in range(len(self.routes)):
            if self.routes[i].path == path:
                return Int(self.routes[i].branch)
        return -1

    fn init_slot(mut self, anchor_id: UInt32):
        """Initialize the ConditionalSlot with the anchor element ID.

        Called after the parent template is mounted.  The anchor_id is
        the ElementId of the placeholder comment node in the dyn_node
        slot where the routed view will appear.

        Args:
            anchor_id: ElementId of the dyn_node placeholder.
        """
        self.slot = ConditionalSlot(anchor_id)
