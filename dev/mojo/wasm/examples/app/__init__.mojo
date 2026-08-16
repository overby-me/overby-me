# Multi-view app example package — re-exports MultiViewApp and lifecycle functions.

from .app import (
    MultiViewApp,
    multi_view_app_init,
    multi_view_app_destroy,
    multi_view_app_rebuild,
    multi_view_app_handle_event,
    multi_view_app_flush,
    multi_view_app_navigate,
)
