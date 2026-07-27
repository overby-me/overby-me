//! The app-agnostic component library: small, wiki-free (no GraphQL / domain
//! knowledge) Material 3 components, each in its own module so they can be reused
//! individually and lifted into a shared crate for the atproto rewrite. Screens
//! compose these plus the form-control primitives in [`crate::components::ui`].
//!
//! Everything is re-exported at the `widgets::` root, so callers use
//! `widgets::Dialog`, `widgets::DataTable`, etc. regardless of the file layout.

mod atoms;
mod color_picker;
mod dialog;
mod feedback;
mod focus;
mod image;
mod segmented_button;
mod table;
mod tool_sheet;

pub use atoms::{Badge, Bar, Carousel, Chip, ListItem, Spinner, SupportingPaneLayout};
pub use color_picker::ColorPicker;
pub use dialog::Dialog;
pub use feedback::{EmptyState, ErrorState};
pub use image::ZoomableImage;
pub use segmented_button::SegmentedButton;
pub use table::{DataTable, PaginatedTable};
// CopyLinkAction is deliberately not re-exported: ToolSheet renders it itself as
// its first row, so no call site places (or misplaces) it.
pub use tool_sheet::{ExportAction, ToolSheet, TOOLS_DOCKED};

// Modal-accessibility helpers, reused by the layout's AppSwitcher.
pub(crate) use focus::{active_html_element, close_modal, trap_tab_focus};
