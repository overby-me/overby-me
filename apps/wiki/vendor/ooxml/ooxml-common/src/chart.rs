//! Shared OOXML chart-XML extractors used by both the xlsx and pptx Rust
//! parsers.
//!
//! Both crates parse a chart `<c:chartSpace>` (and the modern
//! `<cx:chartSpace>` for waterfall / treemap / box-and-whisker etc.) but
//! historically did so with two near-identical bodies sitting in
//! `packages/xlsx/parser/src/lib.rs` and `packages/pptx/parser/src/lib.rs`.
//! The result was that fields added on one side stayed missing on the other
//! until somebody noticed (e.g. PowerPoint sample-2 slide-7 displaying its
//! legend on the right because the pptx adapter had a hard-coded
//! `legendPos: null` while xlsx already passed it through).
//!
//! This module hosts the helpers that don't need any crate-private state:
//! they're pure XML probes that take a roxmltree node and return the parsed
//! property. The data-structure layer (xlsx's `ChartData`, pptx's
//! `ChartElement`) intentionally stays in each crate so we don't pull
//! schema-specific types into the shared one.
//!
//! ## Namespace handling
//!
//! All helpers match elements by local name only. Real chart documents put
//! everything under either the `c:` (chart 2006) or `cx:` (chartEx 2014)
//! namespace and never mix non-chart elements at these paths, so the strict
//! `tag_name().namespace() == Some(c_ns)` check in xlsx adds nothing in
//! practice — this module drops it for symmetry with the pptx side and to
//! keep the API simple. If a future format wedges a non-chart element into
//! `<c:plotArea>` the caller can pre-filter before delegating here.
//!
//! All field references are to ECMA-376 / ISO-29500 part 1 §21.2 (DrawingML
//! Charts) unless stated otherwise.

use roxmltree::Node;
use serde::{Deserialize, Serialize};

// ============================================================================
// Shared chart data model
// ============================================================================
//
// These structs are the Rust mirror of the TypeScript `ChartModel` in
// `packages/core/src/types/chart.ts`. Both the pptx and xlsx Rust parsers build
// a `ChartModel` and emit it as a single nested `chart` object, so the TS
// renderer (`@silurus/ooxml-core`'s `renderChart`) receives a value that is
// already `ChartModel`-shaped and needs no per-field adapter.
//
// Field-for-field parity with the TS interface is the contract. Serde
// `rename_all = "camelCase"` matches the TS key names. The REQUIRED TS fields
// (no `?`) are serialized unconditionally so the wire object always carries
// them — an `Option<T>` REQUIRED field emits `null` when `None` (matching
// `T | null`), and a `bool`/`Vec` REQUIRED field emits `false`/`[]`. The
// OPTIONAL TS fields (`field?: …`) keep `skip_serializing_if` so they drop off
// the wire when unset; the renderer treats a missing key and an explicit `null`
// identically (every read is `?? default` / `!= null`), so this is
// render-equivalent to emitting `null`.
//
// All field references are ECMA-376 / ISO-29500 part 1 §21.2 (DrawingML Charts)
// as documented on the TS side; see that file for the per-field spec citations.

/// Mirror of TS `ChartModel`. Built by each parser and emitted as the single
/// `chart` object consumed by the core chart renderer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartModel {
    // ── Required (always serialized) ────────────────────────────────────────
    pub chart_type: String,
    pub title: Option<String>,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
    pub show_data_labels: bool,
    pub val_min: Option<f64>,
    pub val_max: Option<f64>,
    pub cat_axis_title: Option<String>,
    pub val_axis_title: Option<String>,
    pub cat_axis_hidden: bool,
    pub val_axis_hidden: bool,
    pub cat_axis_line_hidden: bool,
    pub val_axis_line_hidden: bool,
    pub plot_area_bg: Option<String>,
    pub chart_bg: Option<String>,
    pub show_legend: bool,
    pub legend_pos: Option<String>,
    pub cat_axis_cross_between: String,
    pub val_axis_major_tick_mark: String,
    pub cat_axis_major_tick_mark: String,
    pub title_font_size_hpt: Option<i32>,
    pub title_font_color: Option<String>,
    pub title_font_face: Option<String>,
    pub cat_axis_font_size_hpt: Option<i32>,
    pub val_axis_font_size_hpt: Option<i32>,
    pub data_label_font_size_hpt: Option<i32>,
    pub subtotal_indices: Vec<u32>,
    // ── Optional (skipped when unset) ───────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_minor_tick_mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_minor_tick_mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_manual_layout: Option<LegendManualLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_gap_width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bar_overlap: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_title_font_size_hpt: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_title_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_title_font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_title_font_size_hpt: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_title_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_title_font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_border_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart_border_width_emu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_crosses: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_crosses_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_crosses: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_crosses_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_line_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_line_width_emu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_line_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_line_width_emu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_manual_layout: Option<ChartManualLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plot_area_manual_layout: Option<ChartManualLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scatter_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar_style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_val_axis: Option<SecondaryValueAxis>,
    // ── Pie / doughnut geometry (CH8) ───────────────────────────────────────
    /// `<c:doughnutChart><c:holeSize val>` (§21.2.2.82, `ST_HoleSizePercent`
    /// §21.2.3.55) — hole diameter as 1–90% of the outer diameter. `None` when
    /// absent; the renderer defaults an absent doughnut hole to 50%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hole_size: Option<u32>,
    /// `<c:pieChart | doughnutChart><c:firstSliceAng val>` (§21.2.2.52,
    /// `ST_FirstSliceAng` §21.2.3.15) — start angle 0–360° clockwise from 12
    /// o'clock. `None` = 0 (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_slice_angle: Option<u32>,
    // ── Chart text font faces (CH10) ────────────────────────────────────────
    /// `<c:catAx><c:txPr>…<a:latin typeface>` tick-label font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_font_face: Option<String>,
    /// `<c:valAx><c:txPr>…<a:latin typeface>` tick-label font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_font_face: Option<String>,
    /// `<c:catAx><c:title>…<a:latin typeface>` axis-title font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_title_font_face: Option<String>,
    /// `<c:valAx><c:title>…<a:latin typeface>` axis-title font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_title_font_face: Option<String>,
    /// `<c:dLbls><c:txPr>…<a:latin typeface>` data-label font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_font_face: Option<String>,
    /// `<c:legend><c:txPr>…<a:latin typeface>` legend font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_font_face: Option<String>,
    /// `<c:legend><c:txPr>…<a:solidFill>` legend text color (hex, no `#`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_font_color: Option<String>,
    /// `<c:legend><c:txPr>` legend font size (OOXML hundredths of a point).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_font_size_hpt: Option<i32>,
    /// `<c:legend><c:txPr>…defRPr@b` legend bold flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend_font_bold: Option<bool>,
    /// Theme heading (majorFont) Latin face — fallback for chart title / axis
    /// titles when their `<c:txPr>` supplies no `<a:latin>`. `None` when the
    /// theme is not threaded (renderer keeps sans-serif; byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_major_font_latin: Option<String>,
    /// Theme body (minorFont) Latin face — fallback for tick labels / data
    /// labels / legend. `None` when the theme is not threaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_minor_font_latin: Option<String>,
    /// `<c:date1904>` (ECMA-376 §21.2.2.38). `true` = the chart's serial dates
    /// resolve against the 1904 date system. Omitted from JSON when false (the
    /// default 1900 system) for wire parity.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub date1904: bool,
    /// `<c:chart><c:dispBlanksAs val>` (ECMA-376 §21.2.2.42) — how blank cells
    /// are plotted on line/area charts ("gap" | "zero" | "span"). `None` when
    /// the element is absent (the renderer defaults to "gap"); only serialized
    /// when the file sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disp_blanks_as: Option<String>,
    // ── Axis scale model (CH6) ──────────────────────────────────────────────
    /// `<c:valAx><c:majorGridlines>` presence (§21.2.2.100). `Some(false)` when
    /// the value axis exists but omits the element — Office suppresses the value
    /// gridlines then. `None` when there is no value axis (or the parser path
    /// doesn't model it); the renderer keeps its historical always-on value
    /// gridlines, so a `None`/absent field is byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_major_gridlines: Option<bool>,
    /// `<c:catAx><c:majorGridlines>` presence (§21.2.2.100). `Some(true)` turns
    /// on category-axis gridlines (Office omits them by default). `None`/absent
    /// keeps the renderer's historical no-category-gridlines behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_major_gridlines: Option<bool>,
    /// `<c:valAx><c:majorGridlines><c:spPr><a:ln><a:solidFill>` resolved gridline
    /// colour (hex, no `#`) — §21.2.2.100. `None` when the value axis omits the
    /// element or gives it no explicit colour; the renderer then keeps its faint
    /// default gridline (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_gridline_color: Option<String>,
    /// `<c:valAx><c:majorGridlines><c:spPr><a:ln w>` gridline width in EMU.
    /// `None` = the renderer's default hairline (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_gridline_width_emu: Option<u32>,
    /// `<c:catAx><c:majorGridlines><c:spPr><a:ln><a:solidFill>` resolved gridline
    /// colour (hex, no `#`). Only meaningful when `cat_axis_major_gridlines` is
    /// on. `None` keeps the faint default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_gridline_color: Option<String>,
    /// `<c:catAx><c:majorGridlines><c:spPr><a:ln w>` gridline width in EMU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_gridline_width_emu: Option<u32>,
    /// `<c:valAx><c:minorGridlines>` presence (§21.2.2.109).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_minor_gridlines: Option<bool>,
    /// `<c:valAx><c:majorUnit val>` (§21.2.2.103) — explicit major gridline
    /// step, overriding the auto "nice" step. `None` = auto (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_major_unit: Option<f64>,
    /// `<c:valAx><c:minorUnit val>` (§21.2.2.112) — explicit minor gridline step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_minor_unit: Option<f64>,
    /// `<c:valAx><c:scaling><c:logBase val>` (§21.2.2.98) — logarithmic value
    /// axis base (>= 2). `None` = linear (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_log_base: Option<f64>,
    /// `<c:valAx><c:scaling><c:orientation val>` (§21.2.2.130) — `"minMax"`
    /// (normal) | `"maxMin"` (reversed). `None`/`"minMax"` = normal (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_orientation: Option<String>,
    /// `<c:catAx><c:scaling><c:orientation val>` — reverses the category axis
    /// left↔right when `"maxMin"`. `None`/`"minMax"` = normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_orientation: Option<String>,
    /// `<c:catAx><c:tickLblPos val>` (§21.2.2.207) — `"nextTo"` (default) |
    /// `"low"` | `"high"` | `"none"` (labels hidden). `None` = nextTo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_tick_label_pos: Option<String>,
    /// `<c:valAx><c:tickLblPos val>` (§21.2.2.207).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_axis_tick_label_pos: Option<String>,
    /// `<c:catAx><c:txPr><a:bodyPr rot>` (60000ths of a degree) — category
    /// tick-label rotation. `None`/0 = horizontal (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cat_axis_label_rotation: Option<i32>,
    // ── Stock chart (CH13, §21.2.2.198) ──────────────────────────────────────
    /// `<c:stockChart><c:hiLowLines>` (§21.2.2.80) presence. When `Some(true)`
    /// the stock renderer draws a vertical line spanning each category's
    /// low↔high value. Only emitted for a stock chart (`chart_type == "stock"`);
    /// `None` on every other chart type keeps the wire byte-stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_hi_low_lines: Option<bool>,
    /// `<c:hiLowLines><c:spPr><a:ln><a:solidFill>` resolved color (hex, no `#`).
    /// `None` = the renderer's default gray. Only meaningful with
    /// `stock_hi_low_lines == Some(true)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_hi_low_line_color: Option<String>,
    /// `<c:stockChart><c:upDownBars>` (§21.2.2.218) presence. Parsed so a file
    /// that carries open-close up/down bars is recognized; the stock renderer
    /// does NOT yet draw them (tracked as a follow-up). `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_up_down_bars: Option<bool>,
    // ── chartEx box-and-whisker / sunburst (CH15, MS 2014 chartex ext) ────────
    /// Structured box-and-whisker data (`chart_type == "boxWhisker"`). `None`
    /// for every other chart type — the field is populated ONLY by
    /// `parse_chartex_part` when the series `layoutId` is `boxWhisker`, so the
    /// flat `categories`/`series` model (which waterfall/treemap consume) is
    /// unchanged and the wire stays byte-stable for those.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chartex_box: Option<ChartexBoxWhisker>,
    /// Structured sunburst hierarchy (`chart_type == "sunburst"`). `None`
    /// otherwise (byte-stable for the flat-model chartEx charts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chartex_sunburst: Option<ChartexSunburst>,
    /// Theme accent palette (`accent1..6` resolved to hex, no `#`) for chartEx
    /// charts that color by branch/series index (boxWhisker series, sunburst
    /// branches). `None` when the resolver supplies no default palette (pptx);
    /// the renderer then falls back to its own `CHART_PALETTE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chartex_accents: Option<Vec<String>>,
    /// §21.2.2.227 `<c:varyColors val="1"/>` on a SINGLE-series bar/column
    /// chart: color each data point (bar) from the theme/palette sequence and
    /// list one legend entry per point (like a pie). `Some(true)` only for that
    /// non-pie, single-series case the core renderer consumes; the pie family
    /// already varies by point via `chart_type` + `data_point_colors`, so it
    /// stays `None` here (byte-stable wire for every existing chart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vary_colors: Option<bool>,
}

/// Mirror of TS `ChartSeries`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeries {
    pub name: String,
    pub color: Option<String>,
    pub values: Vec<Option<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_point_colors: Option<Vec<Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_colors: Option<Vec<Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_secondary_axis: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_marker: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub val_format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_point_overrides: Option<Vec<ChartDataPointOverride>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_label_overrides: Option<Vec<ChartDataLabelOverride>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series_data_labels: Option<ChartSeriesDataLabels>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err_bars: Option<Vec<ChartErrBars>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bubble_sizes: Option<Vec<Option<f64>>>,
    /// `<c:ser><c:smooth val>` (ECMA-376 §21.2.2.194) — line/area series flag
    /// requesting a smoothed (spline) curve. `None` (omitted) = straight
    /// polyline (the default); only serialized when the file sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<bool>,
    /// `<c:ser><c:trendline>` per-series trendlines (§21.2.2.211). `None`/empty
    /// when the series declares none (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend_lines: Option<Vec<ChartTrendline>>,
    /// `<c:ser><c:spPr><a:ln><a:noFill/>` (§21.2.2.198 CT_ShapeProperties →
    /// DrawingML §20.1.2.2.24 CT_LineProperties). `Some(true)` when the series'
    /// connecting line is explicitly turned OFF. For a scatter/line series this
    /// overrides the chart-group `<c:scatterStyle>` (§21.2.2.42) / line default:
    /// Excel/PowerPoint draw NO connecting line when the series line is
    /// `<a:noFill/>`, even if the group style is `lineMarker`. `None` (omitted)
    /// = the series carries no explicit line-off, so the group default governs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_hidden: Option<bool>,
}

/// Mirror of TS `ChartTrendline` — `<c:ser><c:trendline>` (§21.2.2.211).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartTrendline {
    /// `<c:trendlineType val>` (§21.2.2.213) — linear|exp|log|power|poly|movingAvg.
    pub trendline_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backward: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intercept: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disp_r_sqr: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disp_eq: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width_emu: Option<u32>,
}

/// Mirror of TS `ChartDataPointOverride`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartDataPointOverride {
    pub idx: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_fill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_line: Option<String>,
    /// `<c:dPt><c:explosion val>` (§21.2.2.61) — pie/doughnut slice pull-out
    /// amount. The schema type is `CT_UnsignedInt` (unbounded `xsd:unsignedInt`);
    /// the spec text itself doesn't define a 0–100 range or "percentage" unit,
    /// only "the amount the data point shall be moved from the center of the
    /// pie". Renderers interpret it as a de-facto percentage of the outer
    /// radius (0–100 typical), matching Office's Point Explosion UI slider
    /// rather than a spec-mandated bound. `None`/absent = 0 (byte-stable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explosion: Option<u32>,
}

/// Mirror of TS `ChartDataLabelOverride`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartDataLabelOverride {
    pub idx: u32,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_hpt: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_bold: Option<bool>,
    /// Per-point label callout box style (`<c:dLbl>` §21.2.2.47 `<c:spPr>`
    /// §21.2.2.197): background fill / border, mirroring the series-level
    /// defaults. Present only when the point's `<c:spPr>` overrides the shape
    /// (e.g. a differently tinted callout for one slice). See [`ChartLabelBox`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_box: Option<ChartLabelBox>,
    /// Per-point label-content flags (`<c:dLbl>` §21.2.2.47 carries the full
    /// `CT_DLbl` show-flag group: §21.2.2.189 `<c:showVal>`, §21.2.2.177
    /// `<c:showCatName>`, §21.2.2.180 `<c:showSerName>`, §21.2.2.187
    /// `<c:showPercent>`). When a `<c:dLbl>` sets these they OVERRIDE the
    /// series-level `<c:dLbls>` defaults (§21.2.2.49) for that one point — e.g.
    /// sample-14 slide-7's pie sets `showCatName=0 showPercent=1` per slice even
    /// though the series default is `showCatName=1`, so each label is percent
    /// only. `None` = the point declared no such flag, so the series default
    /// governs (byte-stable for points that carry none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_val: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_cat_name: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_ser_name: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_percent: Option<bool>,
    /// `<c:dLbl><c:delete val="1"/>` (§21.2.2.43) — this point's label is
    /// removed. Distinguishes a genuinely deleted label from a `<c:dLbl>` that
    /// merely carries style/flag overrides with no `<c:tx>` (both formerly
    /// collapsed to `text == ""`). `Some(true)` = skip the label entirely;
    /// `None`/absent = not deleted (compose from flags / text). Byte-stable for
    /// points that carry no `<c:delete>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
}

/// Callout-box style for a pie/doughnut data label — the white (or themed)
/// rounded rectangle with a thin border that Word draws around a `bestFit`
/// label placed outside its slice. Parsed from the label's `<c:spPr>`
/// (§21.2.2.197, the shape properties of a `<c:dLbl>` §21.2.2.47 /
/// `<c:dLbls>` §21.2.2.49): the direct `<a:solidFill>` is the box fill and the
/// `<a:ln>` is its border.
///
/// A `None` on `ChartSeriesDataLabels::label_box` means the file wrote no box
/// shape, so the renderer keeps the historical plain-text label (no callout).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChartLabelBox {
    /// `<c:spPr><a:solidFill>` resolved hex (no `#`). The box background;
    /// `<a:noFill>`/absent leaves this `None` (transparent box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// `<c:spPr><a:ln><a:solidFill>` resolved hex (no `#`) — border stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_color: Option<String>,
    /// `<c:spPr><a:ln w>` border width in EMU (12700 EMU = 1 pt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_width_emu: Option<u32>,
}

/// Mirror of TS `ChartSeriesDataLabels`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChartSeriesDataLabels {
    pub show_val: bool,
    pub show_cat_name: bool,
    pub show_ser_name: bool,
    pub show_percent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_hpt: Option<i32>,
    /// Series-default callout-box style (`<c:dLbls>` §21.2.2.49 `<c:spPr>`
    /// §21.2.2.197) — the box drawn around each pie/doughnut label. When present
    /// the pie renderer switches from plain outer-ring text to Word's boxed
    /// callout layout (box + optional leader line); `None` keeps the plain
    /// labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_box: Option<ChartLabelBox>,
    /// `<c:dLbls><c:showLeaderLines val>` (§21.2.2.183) — whether leader lines
    /// connect a label pulled away from its slice back to the slice. Absent =
    /// `false` (Office omits the element when leader lines are off). Only
    /// consulted by the pie/doughnut callout renderer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_leader_lines: bool,
    /// `<c:dLbls><c:leaderLines>` (§21.2.2.92) `<c:spPr><a:ln><a:solidFill>`
    /// resolved hex (no `#`) — the leader-line stroke color. `None` falls back
    /// to a neutral grey in the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_line_color: Option<String>,
    /// `<c:dLbls><c:leaderLines><c:spPr><a:ln w>` leader-line width in EMU.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_line_width_emu: Option<u32>,
}

/// Mirror of TS `ChartErrBars`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartErrBars {
    pub dir: String,
    pub bar_type: String,
    pub plus: Vec<Option<f64>>,
    pub minus: Vec<Option<f64>>,
    pub no_end_cap: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width_emu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<String>,
}

/// Mirror of TS `SecondaryValueAxis`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecondaryValueAxis {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub title: Option<String>,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size_hpt: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_width_emu: Option<u32>,
    pub line_hidden: bool,
    pub major_tick_mark: String,
    /// `<c:valAx><c:majorUnit val>` (§21.2.2.103) — explicit major-unit step on
    /// this secondary axis, overriding the auto "nice" step. `None` ⇒ auto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub major_unit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font_size_hpt: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_font_color: Option<String>,
}

/// One box-and-whisker series (chartEx `boxWhisker`, MS 2014 chartex ext).
///
/// A chartEx box-and-whisker chart carries one `<cx:series layoutId="boxWhisker">`
/// per data column, each referencing its own `<cx:data>` (via `<cx:dataId>`) of
/// RAW sample points grouped by category. Statistics (quartiles / mean /
/// whiskers / outliers) are computed by the renderer per the
/// `<cx:layoutPr><cx:statistics quartileMethod>` and `<cx:visibility>` flags;
/// the parser only groups the raw points by category and threads the flags.
/// Mirror of TS `ChartexBoxSeries`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartexBoxSeries {
    /// Series display name (`<cx:tx><cx:txData><cx:v>`), e.g. "Series1".
    pub name: String,
    /// Series fill (hex, no `#`) — the theme accent cycled by series index
    /// (`accent[(idx % 6) + 1]`). `None` when the resolver supplies no default
    /// palette (pptx); the renderer then falls back to its own palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Raw sample values grouped by category, parallel to
    /// `ChartexBoxWhisker::categories`. Outer index = category, inner = the
    /// sample points that fell in that category (source order preserved).
    pub values_by_category: Vec<Vec<f64>>,
    /// `<cx:layoutPr><cx:visibility meanMarker>` — draw the mean `×` marker.
    pub mean_marker: bool,
    /// `<cx:layoutPr><cx:visibility meanLine>` — draw a mean connector line.
    pub mean_line: bool,
    /// `<cx:layoutPr><cx:visibility outliers>` — draw outlier points.
    pub show_outliers: bool,
    /// `<cx:layoutPr><cx:visibility nonoutliers>` — draw the non-outlier
    /// (interior) points as dots in addition to the box. Flag parsed;
    /// interior-dot rendering is pending a fixture that enables it (every
    /// sample-24 series ships `nonoutliers="0"`, so there is nothing to verify
    /// the overlay against yet).
    pub show_nonoutliers: bool,
    /// `<cx:layoutPr><cx:statistics quartileMethod>` — `"exclusive"` (Excel
    /// default, median excluded when splitting halves) or `"inclusive"`.
    pub quartile_method: String,
}

/// A chartEx box-and-whisker chart: the unique categories plus one series per
/// data column. Mirror of TS `ChartexBoxWhisker`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartexBoxWhisker {
    /// Unique category labels in first-seen order (the box groups on the
    /// category axis). Each series bins its raw points into these.
    pub categories: Vec<String>,
    /// One entry per `<cx:series>`.
    pub series: Vec<ChartexBoxSeries>,
}

/// One row of a chartEx `sunburst` (MS 2014 chartex ext). A sunburst encodes
/// its hierarchy as one `<cx:strDim type="cat">` with several `<cx:lvl>`
/// (lvl[0] = deepest / Leaf, last lvl = root / Branch) and a single
/// `<cx:numDim type="size">`. Each row's `path` is the branch→…→leaf label
/// chain with empty trailing segments trimmed (a node that is itself a leaf
/// terminates early); `size` is that row's size value. Mirror of TS
/// `ChartexSunburstRow`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartexSunburstRow {
    /// Label chain root→leaf (Branch, Stem, …, Leaf), empty tail trimmed.
    pub path: Vec<String>,
    /// `<cx:numDim type="size">` value for this row (attaches to the deepest
    /// node in `path`).
    pub size: f64,
}

/// A chartEx sunburst: the flat rows the renderer folds into a ring tree, plus
/// the theme accent palette to color branches. Mirror of TS `ChartexSunburst`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartexSunburst {
    /// One row per deepest-level data point.
    pub rows: Vec<ChartexSunburstRow>,
}

/// Mirror of TS `ChartManualLayout`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChartManualLayout {
    pub x_mode: String,
    pub y_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_target: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<f64>,
}

/// Mirror of TS `LegendManualLayout`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LegendManualLayout {
    pub x_mode: String,
    pub y_mode: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Combine a chart-type family (`bar` / `line` / `area`) with its bar direction
/// and grouping into the canonical `ChartModel.chart_type` vocabulary the core
/// renderer dispatches on.
///
/// This is the Rust home of the logic the xlsx TS renderer used to run in
/// `canonicalChartType` (pptx already emitted the canonical string). `bar_dir`
/// is ECMA-376 §21.2.3.4 `ST_BarDir`: `"bar"` = horizontal, `"col"` (or any
/// other value) = vertical. `grouping` is §21.2.3.17 `ST_Grouping`. Non-bar /
/// non-line / non-area families are returned unchanged.
pub fn canonical_chart_type(chart_type: &str, bar_dir: &str, grouping: &str) -> String {
    match chart_type {
        "bar" => {
            let is_h = bar_dir == "bar";
            match (grouping, is_h) {
                ("stacked", true) => "stackedBarH",
                ("stacked", false) => "stackedBar",
                ("percentStacked", true) => "stackedBarHPct",
                ("percentStacked", false) => "stackedBarPct",
                (_, true) => "clusteredBarH",
                (_, false) => "clusteredBar",
            }
            .to_string()
        }
        "line" => match grouping {
            "stacked" => "stackedLine",
            "percentStacked" => "stackedLinePct",
            _ => "line",
        }
        .to_string(),
        "area" => match grouping {
            "stacked" => "stackedArea",
            "percentStacked" => "stackedAreaPct",
            _ => "area",
        }
        .to_string(),
        other => other.to_string(),
    }
}

/// Find a direct child of `parent` whose local name is `name`.
fn child<'a, 'i>(parent: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    parent
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// Read a no-namespace attribute `local` off `node` as an owned `String`.
///
/// Mirrors the pptx/xlsx crate-local `attr` helper exactly (matches only
/// attributes with no namespace, the shape every chart attribute uses), so the
/// chart-structure parse moved into [`parse_chart_part`] stays byte-identical
/// to the per-crate bodies it replaces.
fn attr(node: &Node, local: &str) -> Option<String> {
    node.attributes()
        .find(|a| a.name() == local && a.namespace().is_none())
        .map(|a| a.value().to_owned())
}

/// Theme-aware color resolution for chart text-color helpers.
///
/// pptx and xlsx store their theme palettes in different shapes
/// (`HashMap<String, String>` vs. `&[String]`) and apply DrawingML
/// transforms with different `tint` formulas (Word-literal vs. linear
/// sRGB lerp), so each crate keeps its own resolver. The shared chart
/// helpers take a `&dyn ColorResolver` instead of either concrete type so
/// fields like `<c:dLbls><c:txPr>...<a:solidFill>` can be extracted once.
pub trait ColorResolver {
    /// Resolve an `<a:solidFill>` node to a hex string (no leading `#`),
    /// or `None` when the contained color child can't be mapped to a
    /// concrete RGB value (for example a `<a:schemeClr val="phClr"/>`
    /// that the implementation chooses not to substitute).
    ///
    /// The node passed in is the `<a:solidFill>` element itself; the
    /// implementation reads its direct children for the actual color
    /// (`<a:srgbClr>` / `<a:schemeClr>` / `<a:sysClr>` / `<a:prstClr>`)
    /// and applies the surrounding lumMod/lumOff/tint/shade transforms.
    fn resolve_solid_fill(&self, node: Node) -> Option<String>;

    /// Resolve the first `<a:solidFill>` among `parent`'s **direct children** to
    /// a hex string (no leading `#`) using the full DrawingML color grammar,
    /// including `lumMod`/`lumOff`/`tint`/`shade` transforms.
    ///
    /// This is the resolver used for chart *shape* fills that sit one level
    /// below their container — series fills/lines (`<c:ser><c:spPr>` /
    /// `…<a:ln>`), marker fill/line (`<c:marker><c:spPr>` / `…<a:ln>`),
    /// per-point fills (`<c:dPt><c:spPr>`) and error-bar strokes
    /// (`<c:errBars><c:spPr>` / `…<a:ln>`). It is intentionally distinct from
    /// [`ColorResolver::resolve_solid_fill`] so resolvers can route every shape
    /// through the complete DrawingML color-transform grammar.
    ///
    /// The default implementation finds the direct-child `<a:solidFill>` and
    /// delegates to [`ColorResolver::resolve_solid_fill`], which is correct for
    /// resolvers whose `resolve_solid_fill` already applies the full grammar
    /// (pptx). xlsx overrides it to route through its DrawingML color path.
    fn resolve_shape_fill(&self, parent: Node) -> Option<String> {
        parent
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
            .and_then(|fill| self.resolve_solid_fill(fill))
    }

    /// Theme major (heading) Latin typeface name, or `None` when the theme
    /// declares no `fontScheme`. Used as the chart-text fallback face when a run
    /// carries no explicit `<a:latin>`. Defaults to `None` so resolvers that do
    /// not carry a theme font map need not override it.
    fn theme_major_font_latin(&self) -> Option<String> {
        None
    }

    /// Theme minor (body) Latin typeface name, or `None` when the theme declares
    /// no `fontScheme`. Companion to [`ColorResolver::theme_major_font_latin`].
    fn theme_minor_font_latin(&self) -> Option<String> {
        None
    }

    /// Default series fill for a series with no explicit `<c:spPr>` fill, keyed
    /// by its `<c:idx>` (ECMA-376 §21.2.2.84). Office cycles the theme accents:
    /// `theme.accent[(idx % 6) + 1]`. Returning the resolved accent hex here
    /// (no leading `#`) lets the renderer draw the correct default palette
    /// without needing theme access.
    ///
    /// Defaults to `None` so a resolver whose renderer already owns a default
    /// palette (pptx) leaves the series color unset and lets that palette apply.
    fn resolve_series_accent(&self, _idx: usize) -> Option<String> {
        None
    }

    /// Chart-area background to use when the `<c:chartSpace>` carries **no**
    /// `<c:spPr>` at all. Excel relies on its default opaque-white chart area in
    /// that case, so the xlsx resolver returns `Some("FFFFFF")`; PowerPoint
    /// composites the chart transparently over the slide, so pptx returns `None`.
    /// (When `<c:spPr>` *is* present the parser honours whatever it resolves to —
    /// a solid hex or `noFill` → `None` — regardless of this default.)
    fn default_chart_bg(&self) -> Option<String> {
        None
    }
}

/// `<c:legend>` presence + `<c:legendPos val>` (ECMA-376 §21.2.2.10).
///
/// `(show_legend, legend_pos)`. When the chart omits `<c:legend>` Office
/// hides the legend even if a default position would otherwise apply, so
/// `show_legend = false` is the authoritative "no legend" signal.
pub fn extract_legend(root: Node) -> (bool, Option<String>) {
    // The legend can sit anywhere inside `<c:chart>` but in practice it's a
    // direct child of `<c:chart>`. Use descendants to be tolerant of either
    // structure — there's only one `<c:legend>` element per chart.
    let legend = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "legend");
    let show = legend.is_some();
    let pos = legend.and_then(|ln| {
        child(ln, "legendPos")
            .and_then(|p| p.attribute("val"))
            .map(|s| s.to_string())
    });
    (show, pos)
}

/// `<c:barChart><c:gapWidth val>` / `<c:overlap val>` (ECMA-376 §21.2.2.13,
/// §21.2.2.25). Returns `(gap%, overlap%)`. Defaults to (None, None) when
/// the file relies on Office's defaults (gap 150, overlap 0).
pub fn extract_bar_gap_overlap(root: Node) -> (Option<i32>, Option<i32>) {
    let gap = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "gapWidth")
        .and_then(|n| n.attribute("val").and_then(|v| v.parse::<i32>().ok()));
    let ov = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "overlap")
        .and_then(|n| n.attribute("val").and_then(|v| v.parse::<i32>().ok()));
    (gap, ov)
}

/// First `<c:dLbls><c:dLblPos val>` found anywhere in the chart (chart-level
/// or per-series). ECMA-376 §21.2.2.49.
pub fn extract_data_label_position(root: Node) -> Option<String> {
    root.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
        .find_map(|dlbls| {
            child(dlbls, "dLblPos")
                .and_then(|n| n.attribute("val"))
                .map(|s| s.to_string())
        })
}

/// First non-`General` `<c:dLbls><c:numFmt formatCode>` in the chart.
/// ECMA-376 §21.2.2.37.
pub fn extract_data_label_format_code(root: Node) -> Option<String> {
    root.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
        .find_map(|dlbls| {
            child(dlbls, "numFmt")
                .and_then(|n| n.attribute("formatCode"))
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty() && s != "General")
        })
}

/// `<c:catAx|valAx><c:numFmt formatCode>` — the value-axis tick label
/// number format (ECMA-376 §21.2.2.21). Caller passes the already-located
/// `<c:catAx>` / `<c:valAx>` node.
pub fn extract_axis_format_code(axis_node: Node) -> Option<String> {
    child(axis_node, "numFmt")
        .and_then(|n| n.attribute("formatCode"))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty() && s != "General")
}

/// `<c:catAx|valAx><c:scaling>` — read explicit `<c:min val>` / `<c:max val>`.
/// Returns `(min, max)`; either can be `None` when the file leaves Excel to
/// pick the auto bound.
pub fn extract_axis_min_max(axis_node: Node) -> (Option<f64>, Option<f64>) {
    let Some(scaling) = child(axis_node, "scaling") else {
        return (None, None);
    };
    let mn = child(scaling, "min")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok());
    let mx = child(scaling, "max")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok());
    (mn, mx)
}

/// `<c:catAx|valAx><c:crosses val>` and `<c:crossesAt val>` (ECMA-376
/// §21.2.2.33/§21.2.2.34). `crosses` is `autoZero` | `min` | `max`; `crossesAt`
/// is an explicit numeric override. Returns `(crosses, crosses_at)`.
pub fn extract_axis_crosses(axis_node: Node) -> (Option<String>, Option<f64>) {
    let crosses = child(axis_node, "crosses")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string());
    let crosses_at = child(axis_node, "crossesAt")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok());
    (crosses, crosses_at)
}

/// `<c:radarChart><c:radarStyle val>` (ECMA-376 §21.2.3.10): `standard` (line
/// only), `marker` (line + markers), or `filled` (closed area). `None` when
/// the chart is not a radar chart or omits the element.
pub fn extract_radar_style(root: Node) -> Option<String> {
    root.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "radarStyle")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string())
}

/// Parse a `<c:layout><c:manualLayout>` node into a [`ChartManualLayout`]
/// (ECMA-376 §21.2.2.88). `layout_node` is the `<c:layout>` element; returns
/// `None` when it carries no `<c:manualLayout>` child. `xMode`/`yMode` default
/// to `"edge"`; `x`/`y` default to 0; `w`/`h` stay `None` when absent.
pub fn extract_manual_layout(layout_node: Node) -> Option<ChartManualLayout> {
    let manual = child(layout_node, "manualLayout")?;
    let mut x_mode = "edge".to_string();
    let mut y_mode = "edge".to_string();
    let mut layout_target: Option<String> = None;
    let mut x = 0.0_f64;
    let mut y = 0.0_f64;
    let mut w: Option<f64> = None;
    let mut h: Option<f64> = None;
    for ch in manual.children().filter(|n| n.is_element()) {
        let val_str = attr(&ch, "val");
        match ch.tag_name().name() {
            "xMode" => {
                if let Some(v) = val_str {
                    x_mode = v;
                }
            }
            "yMode" => {
                if let Some(v) = val_str {
                    y_mode = v;
                }
            }
            "layoutTarget" => {
                layout_target = val_str;
            }
            "x" => {
                if let Some(v) = val_str.and_then(|s| s.parse::<f64>().ok()) {
                    x = v;
                }
            }
            "y" => {
                if let Some(v) = val_str.and_then(|s| s.parse::<f64>().ok()) {
                    y = v;
                }
            }
            "w" => {
                w = val_str.and_then(|s| s.parse::<f64>().ok());
            }
            "h" => {
                h = val_str.and_then(|s| s.parse::<f64>().ok());
            }
            _ => {}
        }
    }
    Some(ChartManualLayout {
        x_mode,
        y_mode,
        layout_target,
        x,
        y,
        w,
        h,
    })
}

/// `<c:legend><c:layout><c:manualLayout>` (ECMA-376 §21.2.2.31) → a
/// [`LegendManualLayout`]. Unlike the plot/title layout, the legend variant has
/// no `layoutTarget` and always carries explicit `w`/`h` (defaulting to 0).
/// `legend_node` is the `<c:legend>` element. `None` when it has no manual layout.
pub fn extract_legend_manual_layout(legend_node: Node) -> Option<LegendManualLayout> {
    let layout = child(legend_node, "layout")?;
    let manual = child(layout, "manualLayout")?;
    let val = |tag: &str| {
        child(manual, tag).and_then(|n| n.attribute("val").and_then(|v| v.parse::<f64>().ok()))
    };
    let mode = |tag: &str| {
        child(manual, tag)
            .and_then(|n| n.attribute("val").map(|v| v.to_string()))
            .unwrap_or_else(|| "edge".to_string())
    };
    Some(LegendManualLayout {
        x_mode: mode("xMode"),
        y_mode: mode("yMode"),
        x: val("x").unwrap_or(0.0),
        y: val("y").unwrap_or(0.0),
        w: val("w").unwrap_or(0.0),
        h: val("h").unwrap_or(0.0),
    })
}

/// `<c:catAx|valAx><c:delete val="1"/>` — true when the axis (labels, ticks
/// and line) should be hidden. ECMA-376 §21.2.2.40. `<c:delete>` is a
/// `CT_Boolean` (dml-chart.xsd `val` default `true`), so a bare `<c:delete/>`
/// means the axis IS deleted; an absent element leaves the axis shown.
pub fn axis_is_deleted(axis_node: Node) -> bool {
    bool_child(axis_node, "delete").unwrap_or(false)
}

/// `<c:catAx|valAx><c:majorTickMark val>` / `<c:minorTickMark val>`. Values
/// are the ECMA-376 §21.2.3.48 ST_TickMark enum: `none` | `out` | `in` |
/// `cross`. Returns the raw string (None when the element is absent).
pub fn extract_axis_tick_mark(axis_node: Node, name: &str) -> Option<String> {
    child(axis_node, name)
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string())
}

/// Like [`extract_axis_tick_mark`] but applies the schema default `"out"` when
/// the element is absent (CT_TickMark `val` defaults to `out` — ECMA-376
/// §21.2.3.48 ST_TickMark). Keeps pptx/xlsx in agreement: the xlsx renderer
/// already defaults to `"out"`; the legacy pptx `"cross"` default was a bug
/// (it drew crossing ticks on charts that omit `<c:majorTickMark>`).
pub fn extract_axis_tick_mark_or_default(axis_node: Node, name: &str) -> String {
    extract_axis_tick_mark(axis_node, name).unwrap_or_else(|| "out".to_string())
}

/// First `<a:defRPr@sz>` or `<a:rPr@sz>` found inside the axis's `<c:txPr>`.
/// Sizes are OOXML hundredths of a point (e.g. 1200 = 12 pt).
pub fn extract_axis_tick_label_size(axis_node: Node) -> Option<i32> {
    let txpr = child(axis_node, "txPr")?;
    txpr.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
    })
}

/// First `<a:defRPr@b>` / `<a:rPr@b>` bold flag inside the axis's `<c:txPr>`
/// — the tick-label bold flag (ECMA-376 §21.2.2.17). `None` when unspecified.
pub fn extract_axis_tick_label_bold(axis_node: Node) -> Option<bool> {
    let txpr = child(axis_node, "txPr")?;
    txpr.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("b")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    })
}

/// Plain text of `node`'s direct-child `<c:title>` (ECMA-376 §21.2.2.6
/// `CT_Title`). Works for the `<c:chart>` element (chart title) or a
/// `<c:catAx>` / `<c:valAx>` (axis title). Walks `<a:t>` (rich text runs) and
/// `<c:v>` (string-ref cache) descendants and concatenates their text.
/// Returns `None` when there is no `<c:title>` child or it carries no text.
pub fn extract_chart_title_text(node: Node) -> Option<String> {
    let title = child(node, "title")?;
    let mut text = String::new();
    for d in title.descendants().filter(|n| n.is_element()) {
        match d.tag_name().name() {
            "t" | "v" => {
                if let Some(t) = d.text() {
                    text.push_str(t);
                }
            }
            _ => {}
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// First `<a:defRPr@sz>` / `<a:rPr@sz>` (hundredths of a point) inside `node`'s
/// direct-child `<c:title>`. `None` when absent.
pub fn extract_chart_title_size(node: Node) -> Option<i32> {
    let title = child(node, "title")?;
    title.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
    })
}

/// chartEx (`<cx:chartSpace>`) title font size in hundredths of a point.
///
/// Unlike the legacy chart, whose `<c:title>` is a direct child of the chart
/// node, a chartEx title lives at `<cx:chart><cx:title>` (a grandchild of the
/// part root), so this walks all descendants to find the first `<cx:title>` and
/// reads its first `<a:defRPr@sz>` / `<a:rPr@sz>`. `None` when the title carries
/// no explicit size — which is the common case (see
/// [`extract_chartex_style_title_size`]).
fn extract_chartex_title_size(root: Node) -> Option<i32> {
    let title = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "title")?;
    title.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
    })
}

/// Relationship-type suffix that a chart part's `.rels` uses to point at its
/// chartStyle sidecar (`styleN.xml`). Matched by `ends_with` so both the
/// Transitional and Strict namespace prefixes resolve. Shared by the pptx /
/// xlsx / docx callers so they resolve the same relationship the same way.
pub const CHART_STYLE_REL_TYPE_SUFFIX: &str = "office/2011/relationships/chartStyle";

/// Title font size (hundredths of a point) declared by the chart's associated
/// chartStyle part (`<cs:chartStyle><cs:title><cs:defRPr@sz>`).
///
/// A chartEx part almost never inlines the title size on its own `<cx:title>`;
/// instead the size lives in the sibling `styleN.xml` reached via the chart
/// part's `.../2011/relationships/chartStyle` relationship. Word's default
/// modern chart style writes `<cs:title><cs:defRPr sz="1400">` (14pt), so
/// without reading it a chartEx title would fall back to an area-proportional
/// guess that is visibly too large. `None` when `style_xml` is absent, malformed,
/// or declares no `<cs:title>` size.
pub fn extract_chartex_style_title_size(style_xml: &str) -> Option<i32> {
    let doc = roxmltree::Document::parse(style_xml).ok()?;
    let title = doc
        .root_element()
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "title")?;
    // `<cs:title>`'s size sits on its direct-child `<cs:defRPr@sz>`; scan
    // descendants so a nested `<a:defRPr>`/`<a:rPr>` (if any) is also honored.
    title.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
    })
}

/// First `<a:defRPr@b>` / `<a:rPr@b>` bold flag inside `node`'s direct-child
/// `<c:title>`. `None` when not specified (renderer treats as not bold).
pub fn extract_chart_title_bold(node: Node) -> Option<bool> {
    let title = child(node, "title")?;
    title.descendants().find_map(|n| {
        if !n.is_element() {
            return None;
        }
        let tag = n.tag_name().name();
        if tag != "defRPr" && tag != "rPr" {
            return None;
        }
        n.attribute("b")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    })
}

/// First `<a:solidFill>/<a:srgbClr@val>` (hex without `#`) inside `node`'s
/// direct-child `<c:title>`. Only an `<a:srgbClr>` that is a direct child of a
/// `<a:solidFill>` is honored — this skips gradient stops and other non-fill
/// color nodes. `<a:schemeClr>` is left unresolved here (the theme palette is
/// not wired through to chart title/border parsing yet — a known limitation
/// shared by both parsers). `None` = renderer default.
pub fn extract_chart_title_srgb(node: Node) -> Option<String> {
    let title = child(node, "title")?;
    title.descendants().find_map(|n| {
        if !n.is_element() || n.tag_name().name() != "srgbClr" {
            return None;
        }
        // Skip srgbClr nodes that aren't inside a solidFill (e.g. a gradient stop).
        let parent_is_solid = n
            .parent()
            .map(|p| p.tag_name().name() == "solidFill")
            .unwrap_or(false);
        if !parent_is_solid {
            return None;
        }
        n.attribute("val").map(|s| s.to_string())
    })
}

/// Theme-aware chart-title text color from `node`'s direct-child `<c:title>`,
/// resolved to a hex string (no leading `#`) via the caller's `ColorResolver`.
///
/// Unlike [`extract_chart_title_srgb`] (srgb-only, a historical limitation),
/// this resolves BOTH `<a:srgbClr>` and `<a:schemeClr>` (e.g. `tx2` → the
/// theme's dark-2 slot) plus the surrounding lumMod/lumOff/tint/shade
/// transforms, because chart parts now thread a `&dyn ColorResolver` through
/// `parse_chart_part`. Works for the `<c:chart>` element (chart title) or a
/// `<c:catAx>` / `<c:valAx>` (axis title) since both scope to the node's
/// direct-child `<c:title>`.
///
/// The search is restricted to a `<a:solidFill>` that is a run-property fill
/// (its ancestor chain includes `<a:defRPr>` or `<a:rPr>`), so a title-frame
/// `<c:spPr><a:solidFill>` background fill can never shadow the text color.
/// `None` when there is no `<c:title>`, no run-property solid fill, or the
/// resolver cannot map the contained color (renderer default applies).
pub fn extract_chart_title_color(node: Node, resolver: &dyn ColorResolver) -> Option<String> {
    let title = child(node, "title")?;
    title.descendants().find_map(|n| {
        if !n.is_element() || n.tag_name().name() != "solidFill" {
            return None;
        }
        // Only honor a solidFill that is a text run-property fill — its ancestor
        // chain must pass through a `<a:defRPr>` / `<a:rPr>`. This excludes a
        // `<c:title><c:spPr><a:solidFill>` frame fill.
        let is_run_prop = n
            .ancestors()
            .any(|a| matches!(a.tag_name().name(), "defRPr" | "rPr"));
        if !is_run_prop {
            return None;
        }
        resolver.resolve_solid_fill(n)
    })
}

/// Axis title text + run props from a `<c:catAx>` / `<c:valAx>` node. Reuses
/// the chart-title helpers (which scope to the node's direct-child `<c:title>`);
/// run props are resolved only when title text is present, so an axis with no
/// title yields all `None`. Returns `(text, size_hpt, bold, srgb_color)`.
///
/// NOTE: the color here is srgb-only (via [`extract_chart_title_srgb`]). Prefer
/// [`extract_axis_title_with_props_resolved`] when a `ColorResolver` is in hand
/// so a `<a:schemeClr>` axis-title color resolves too; this srgb-only variant is
/// kept for callers without a resolver.
pub fn extract_axis_title_with_props(
    axis_node: Node,
) -> (Option<String>, Option<i32>, Option<bool>, Option<String>) {
    match extract_chart_title_text(axis_node) {
        None => (None, None, None, None),
        Some(text) => (
            Some(text),
            extract_chart_title_size(axis_node),
            extract_chart_title_bold(axis_node),
            extract_chart_title_srgb(axis_node),
        ),
    }
}

/// Like [`extract_axis_title_with_props`] but resolves the axis-title color via
/// the caller's `ColorResolver`, so a `<a:schemeClr>` (theme) axis-title color
/// resolves in addition to a literal `<a:srgbClr>`. All other fields
/// (text/size/bold) are identical. Returns `(text, size_hpt, bold, color_hex)`.
pub fn extract_axis_title_with_props_resolved(
    axis_node: Node,
    resolver: &dyn ColorResolver,
) -> (Option<String>, Option<i32>, Option<bool>, Option<String>) {
    match extract_chart_title_text(axis_node) {
        None => (None, None, None, None),
        Some(text) => (
            Some(text),
            extract_chart_title_size(axis_node),
            extract_chart_title_bold(axis_node),
            extract_chart_title_color(axis_node, resolver),
        ),
    }
}

// ============================================================================
// Chart text font faces (CH10) — `<c:txPr>` / `<c:title>` → `<a:latin@typeface>`
// ============================================================================

/// First `<a:latin typeface>` (DrawingML §20.1.4.2.24) descendant of `container`.
/// Empty typefaces are dropped; a theme reference like `+mn-lt` / `+mj-lt` is
/// returned verbatim so the caller can resolve it against the font scheme.
fn first_latin_typeface(container: Node) -> Option<String> {
    container.descendants().find_map(|n| {
        if !n.is_element() || n.tag_name().name() != "latin" {
            return None;
        }
        n.attribute("typeface")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    })
}

/// `<c:catAx|valAx><c:txPr>…<a:latin typeface>` — the axis tick-label font face.
/// Scoped to the axis's `<c:txPr>` so an axis *title* face (under `<c:title>`)
/// is not misread as the tick face. `None` when absent (renderer falls back to
/// the theme body font, then sans-serif).
pub fn extract_axis_tick_label_face(axis_node: Node) -> Option<String> {
    first_latin_typeface(child(axis_node, "txPr")?)
}

/// `<c:catAx|valAx><c:title>…<a:latin typeface>` — the axis-title font face.
/// Scoped to the axis's direct-child `<c:title>`. `None` when absent.
pub fn extract_axis_title_face(axis_node: Node) -> Option<String> {
    first_latin_typeface(child(axis_node, "title")?)
}

/// First `<c:dLbls><c:txPr>…<a:latin typeface>` in the chart — the data-label
/// font face. Scoped to a `<c:txPr>` inside a `<c:dLbls>` so a series-value
/// run's face isn't picked up. `None` when absent.
pub fn extract_data_label_face(root: Node) -> Option<String> {
    root.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
        .find_map(|dlbls| first_latin_typeface(child(dlbls, "txPr")?))
}

/// `<c:legend><c:txPr>` text properties (CH10). Returns
/// `(face, size_hpt, bold)` — the legend `<a:latin typeface>`, first
/// `<a:defRPr|rPr@sz>` (hundredths of a point) and `@b` bold flag. Color is
/// resolved separately via [`extract_legend_font_color`] (needs the theme
/// resolver). All `None` when the legend has no `<c:txPr>`.
pub fn extract_legend_text_props(root: Node) -> (Option<String>, Option<i32>, Option<bool>) {
    let Some(legend) = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "legend")
    else {
        return (None, None, None);
    };
    let Some(txpr) = child(legend, "txPr") else {
        return (None, None, None);
    };
    let face = first_latin_typeface(txpr);
    let size = txpr.descendants().find_map(|n| {
        let tag = n.tag_name().name();
        if n.is_element() && (tag == "defRPr" || tag == "rPr") {
            n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
        } else {
            None
        }
    });
    let bold = txpr.descendants().find_map(|n| {
        let tag = n.tag_name().name();
        if n.is_element() && (tag == "defRPr" || tag == "rPr") {
            n.attribute("b")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        } else {
            None
        }
    });
    (face, size, bold)
}

/// `<c:legend><c:txPr>…<a:solidFill>` legend text color, resolved to a hex
/// string (no `#`) via the caller's `ColorResolver`. Scoped to the legend's
/// `<c:txPr>` so a legend-frame `<c:spPr>` fill doesn't leak. `None` when absent.
pub fn extract_legend_font_color(root: Node, resolver: &dyn ColorResolver) -> Option<String> {
    let legend = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "legend")?;
    let txpr = child(legend, "txPr")?;
    txpr.descendants().find_map(|n| {
        if n.is_element() && n.tag_name().name() == "solidFill" {
            resolver.resolve_solid_fill(n)
        } else {
            None
        }
    })
}

// ============================================================================
// Pie / doughnut geometry (CH8)
// ============================================================================

/// `<c:doughnutChart><c:holeSize val>` (§21.2.2.82) — hole diameter percentage
/// (1–90). Clamped to the ECMA range. `None` when absent. `root` is the chart
/// space (or `<c:chart>`); the search is scoped to a `<c:doughnutChart>` so a
/// hole size only ever comes from a doughnut plot.
pub fn extract_hole_size(root: Node) -> Option<u32> {
    let doughnut = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "doughnutChart")?;
    child(doughnut, "holeSize")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.trim_end_matches('%').parse::<u32>().ok())
        .map(|v| v.clamp(1, 90))
}

/// `<c:pieChart|doughnutChart><c:firstSliceAng val>` (§21.2.2.52) — start angle
/// in degrees (0–360, clockwise from 12 o'clock). Clamped to the ECMA range.
/// `None` when absent (renderer defaults to 0).
pub fn extract_first_slice_angle(root: Node) -> Option<u32> {
    root.descendants()
        .find(|n| {
            n.is_element()
                && (n.tag_name().name() == "pieChart" || n.tag_name().name() == "doughnutChart")
        })
        .and_then(|pie| child(pie, "firstSliceAng"))
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v.min(360))
}

/// `<c:dPt><c:explosion val>` (§21.2.2.61) — pie/doughnut slice pull-out
/// amount, parsed as the unbounded `xsd:unsignedInt` the schema (`CT_UnsignedInt`)
/// actually specifies (no 0–100 clamp here; see `ChartDataPointOverride::explosion`
/// for how renderers interpret the value). Caller passes a `<c:dPt>` node.
/// `None` when absent.
pub fn extract_dpt_explosion(dpt_node: Node) -> Option<u32> {
    child(dpt_node, "explosion")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<u32>().ok())
}

/// Explicit chart-frame border from `<c:chartSpace><c:spPr><a:ln>` (ECMA-376
/// §21.2.2.5 / DrawingML §20.1.2.2.24). `chart_space_root` is the
/// `<c:chartSpace>` element. Returns `(srgb_color, width_emu)` under the locked
/// policy shared by both parsers: a border is drawn ONLY when the XML explicitly
/// declares a paintable line.
///
///  - no `<a:ln>` (or no `<c:spPr>`) → `(None, None)` — no default border;
///  - `<a:ln><a:noFill/>` → border explicitly off → color `None` (width still
///    reported when `@w` is present);
///  - `<a:ln><a:solidFill><a:srgbClr@val>` → `(Some(hex), width)`.
///
/// `@w` (EMU) is captured as `u32` regardless of the fill. `<a:schemeClr>` is
/// intentionally left unresolved here (theme not wired through to chart border
/// parsing yet).
/// `<c:date1904>` (ECMA-376 §21.2.2.38) as a direct child of `<c:chartSpace>`.
/// The element is a `CT_Boolean`: `val` defaults to `true` when the element is
/// present but the attribute is omitted, so `<c:date1904/>` alone means
/// date1904=true. `val="0"` / `"false"` disable it. Absent element ⇒ false (the
/// default 1900 date system, §18.17.4.1).
pub fn extract_chart_date1904(chart_space_root: Node) -> bool {
    match child(chart_space_root, "date1904") {
        Some(n) => match n.attribute("val") {
            None => true, // element present, val implied true
            Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        },
        None => false,
    }
}

/// `<c:ser><c:smooth val>` (ECMA-376 §21.2.2.194) — line/area series smoothing
/// flag. `ser_node` is the `<c:ser>` element. Returns `Some(true/false)` when
/// the element is present (CT_Boolean: `val` implied true when omitted),
/// `None` when the series has no `<c:smooth>` (straight-polyline default). Shared
/// so the pptx and xlsx parsers honor the flag identically.
pub fn extract_series_smooth(ser_node: Node) -> Option<bool> {
    child(ser_node, "smooth").map(|n| match n.attribute("val") {
        None => true, // element present, val implied true
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
    })
}

/// Parse `bool_val`: a `CT_Boolean` child's `val` where an absent attribute
/// implies true (the OOXML default when the element is present).
fn bool_child(parent: Node, name: &str) -> Option<bool> {
    child(parent, name).map(|n| match n.attribute("val") {
        None => true,
        Some(v) => v == "1" || v.eq_ignore_ascii_case("true"),
    })
}

/// `<c:ser><c:trendline>` (ECMA-376 §21.2.2.211, `CT_Trendline`) — every
/// trendline declared on `ser_node` (0..N). Each carries a required
/// `<c:trendlineType>` plus optional order/period/forward/backward/intercept,
/// the `<c:dispRSqr>` / `<c:dispEq>` label flags, and an `<c:spPr><a:ln>` line
/// style (color resolved via `resolver`, width in EMU). Returns `None` when the
/// series declares no trendline (byte-stable); otherwise the parsed vec. Shared
/// so pptx and xlsx honor trendlines identically.
pub fn extract_series_trendlines(
    ser_node: Node,
    resolver: &dyn ColorResolver,
) -> Option<Vec<ChartTrendline>> {
    let mut out = Vec::new();
    for tl in ser_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "trendline")
    {
        // trendlineType is required per the schema; skip a malformed trendline
        // that somehow lacks it rather than emitting an empty type.
        let Some(trendline_type) = child(tl, "trendlineType").and_then(|n| n.attribute("val"))
        else {
            continue;
        };
        let u32_val = |name: &str| -> Option<u32> {
            child(tl, name)
                .and_then(|n| n.attribute("val"))
                .and_then(|v| v.parse::<u32>().ok())
        };
        let f64_val = |name: &str| -> Option<f64> {
            child(tl, name)
                .and_then(|n| n.attribute("val"))
                .and_then(|v| v.parse::<f64>().ok())
        };
        // `<c:spPr><a:ln>` line style: solidFill color + width.
        let (line_color, line_width_emu) = match child(tl, "spPr").and_then(|sp| child(sp, "ln")) {
            None => (None, None),
            Some(ln) => {
                let color = child(ln, "solidFill").and_then(|sf| resolver.resolve_solid_fill(sf));
                let width = ln.attribute("w").and_then(|v| v.parse::<u32>().ok());
                (color, width)
            }
        };
        out.push(ChartTrendline {
            trendline_type: trendline_type.to_string(),
            order: u32_val("order"),
            period: u32_val("period"),
            forward: f64_val("forward"),
            backward: f64_val("backward"),
            intercept: f64_val("intercept"),
            disp_r_sqr: bool_child(tl, "dispRSqr"),
            disp_eq: bool_child(tl, "dispEq"),
            line_color,
            line_width_emu,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `<c:chart><c:dispBlanksAs val>` (ECMA-376 §21.2.2.42, `ST_DispBlanksAs`
/// §21.2.3.10) — how blank cells are plotted ("gap" | "zero" | "span").
/// `root` may be the `<c:chartSpace>` or `<c:chart>` node; the single
/// `<c:dispBlanksAs>` is found by descendant walk either way. Returns `None`
/// when the element is absent (the renderer defaults to "gap"). Per the XSD the
/// `@val` default is "zero" (applies only when `<c:dispBlanksAs/>` is present
/// but the attribute is omitted). Shared so pptx and xlsx behave identically.
pub fn extract_disp_blanks_as(root: Node) -> Option<String> {
    root.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "dispBlanksAs")
        .map(|n| n.attribute("val").unwrap_or("zero").to_string())
}

pub fn extract_chart_space_border(chart_space_root: Node) -> (Option<String>, Option<u32>) {
    let Some(ln) = child(chart_space_root, "spPr").and_then(|sp| child(sp, "ln")) else {
        return (None, None);
    };
    let width = ln.attribute("w").and_then(|v| v.parse::<u32>().ok());
    // An explicit `<a:noFill/>` turns the border off → no color.
    if child(ln, "noFill").is_some() {
        return (None, width);
    }
    // Only an srgbClr inside a direct `<a:solidFill>` is honored.
    let color = child(ln, "solidFill")
        .and_then(|sf| child(sf, "srgbClr"))
        .and_then(|srgb| srgb.attribute("val"))
        .map(|s| s.to_string());
    (color, width)
}

/// First `<c:dLbls><c:txPr>` font size (hpt). Mirrors the per-series + chart
/// fallback chain: walk every `<c:dLbls>` in document order, returning the
/// first inner `<a:defRPr@sz>` / `<a:rPr@sz>` we find.
pub fn extract_data_label_font_size(root: Node) -> Option<i32> {
    root.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
        .find_map(|dl| {
            child(dl, "txPr").and_then(|tx| {
                tx.descendants().find_map(|n| {
                    if !n.is_element() {
                        return None;
                    }
                    let tag = n.tag_name().name();
                    if tag != "defRPr" && tag != "rPr" {
                        return None;
                    }
                    n.attribute("sz").and_then(|v| v.parse::<i32>().ok())
                })
            })
        })
}

/// First `<c:dLbls><c:txPr>...<a:solidFill>` resolved to a hex color.
///
/// Walks each `<c:dLbls>` (chart-level + per-series) in document order,
/// drills into its `<c:txPr>` and looks for the first descendant
/// `<a:solidFill>` whose color the resolver can map. Stops on the first
/// successful resolution — this matches the chart-then-series fallback
/// pattern Office writers actually emit (e.g. a top-level `<c:dLbls>`
/// declaring the label color globally and the `<c:ser><c:dLbls>` blocks
/// inheriting it).
///
/// Note we deliberately scope the search to inside `<c:txPr>` so a
/// sibling `<c:dLbls><c:spPr><a:solidFill>` (the label *background*
/// fill, distinct from the text color) can't shadow the answer.
pub fn extract_data_label_font_color(root: Node, resolver: &dyn ColorResolver) -> Option<String> {
    for dlbls in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
    {
        let Some(txpr) = child(dlbls, "txPr") else {
            continue;
        };
        for desc in txpr.descendants().filter(|n| n.is_element()) {
            if desc.tag_name().name() != "solidFill" {
                continue;
            }
            if let Some(c) = resolver.resolve_solid_fill(desc) {
                return Some(c);
            }
        }
    }
    None
}

/// `<c:catAx|valAx><c:txPr>` tick-label text color, resolved to a hex string
/// (no leading `#`). Walks the axis's `<c:txPr>` for the first descendant
/// `<a:solidFill>` the resolver can map — this is the `<a:defRPr><a:solidFill>`
/// that ECMA-376 §21.2.2.* / §21.1.2.2.* uses to color the axis tick labels
/// (e.g. PowerPoint's "category labels in gray"). Scoped to `<c:txPr>` so the
/// sibling `<c:spPr>` axis-line fill can't shadow the answer.
pub fn extract_axis_tick_label_color(
    axis_node: Node,
    resolver: &dyn ColorResolver,
) -> Option<String> {
    let txpr = child(axis_node, "txPr")?;
    for desc in txpr.descendants().filter(|n| n.is_element()) {
        if desc.tag_name().name() != "solidFill" {
            continue;
        }
        if let Some(c) = resolver.resolve_solid_fill(desc) {
            return Some(c);
        }
    }
    None
}

/// `<c:catAx|valAx><c:spPr><a:ln>` axis-line style (ECMA-376 §21.2.2.* line
/// properties via DrawingML §20.1.2.2.24). Returns `(color, width_emu, no_fill)`:
///
///  - `color`: resolved hex (no `#`) when the line carries a `<a:solidFill>`.
///  - `width_emu`: the `<a:ln w>` width in EMU when present.
///  - `no_fill`: true when the line is explicitly `<a:noFill>` — the axis still
///    shows its labels/ticks but the rule itself is suppressed.
///
/// When the axis has no `<c:spPr><a:ln>` at all the tuple is
/// `(None, None, false)` and the caller falls back to its default rule.
pub fn extract_axis_line_style(
    axis_node: Node,
    resolver: &dyn ColorResolver,
) -> (Option<String>, Option<u32>, bool) {
    extract_sp_pr_ln_style(axis_node, resolver)
}

/// `<…><c:spPr><a:ln>` line style for any node that carries a `<c:spPr>` shape
/// property (an axis, a `<c:majorGridlines>` element, etc.). Returns
/// `(color, width_emu, no_fill)` with the same contract as
/// [`extract_axis_line_style`]:
///
///  - `color`: resolved hex (no `#`) when the line carries a `<a:solidFill>`.
///  - `width_emu`: the `<a:ln w>` width in EMU when present.
///  - `no_fill`: true when the line is explicitly `<a:noFill>`.
///
/// `(None, None, false)` when the node has no `<c:spPr><a:ln>`.
fn extract_sp_pr_ln_style(
    node: Node,
    resolver: &dyn ColorResolver,
) -> (Option<String>, Option<u32>, bool) {
    let Some(sp_pr) = child(node, "spPr") else {
        return (None, None, false);
    };
    let Some(ln) = child(sp_pr, "ln") else {
        return (None, None, false);
    };
    let width = ln.attribute("w").and_then(|v| v.parse::<u32>().ok());
    let no_fill = child(ln, "noFill").is_some();
    // The rule is a SHAPE stroke, so resolve it through `resolve_shape_fill`
    // (full DrawingML grammar incl. lumMod/lumOff tints). xlsx keeps its lighter
    // transform-free `resolve_solid_fill` for series/legend/title fills, so a
    // scheme-color line (e.g. a `bg1 lumMod 65%` light-gray rule, or an
    // `accent3` gridline) must go through the shape path to render at the right
    // strength rather than its untransformed base color.
    let color = resolver.resolve_shape_fill(ln);
    (color, width, no_fill)
}

// ============================================================================
// Axis scale model (CH6) — gridlines / units / logBase / orientation / labels
// ============================================================================
//
// All helpers take the already-located `<c:catAx>` / `<c:valAx>` node (per
// EG_AxShared, ECMA-376 §21.2.2). `<c:majorGridlines>` / `<c:minorGridlines>`
// are direct children of the axis; `<c:logBase>` / `<c:orientation>` live under
// `<c:scaling>`; `<c:majorUnit>` / `<c:minorUnit>` are direct children of a
// `<c:valAx>` (after `<c:crossBetween>`).

/// `<c:catAx|valAx><c:majorGridlines>` presence (ECMA-376 §21.2.2.100,
/// `CT_ChartLines`). The element carries only an optional `<c:spPr>` line
/// style; its mere PRESENCE requests gridlines. Returns `true` when the axis
/// declares `<c:majorGridlines>`. Office writes it on the value axis by default
/// and omits it on the category axis, so this maps directly to "draw them".
pub fn axis_has_major_gridlines(axis_node: Node) -> bool {
    child(axis_node, "majorGridlines").is_some()
}

/// `<c:catAx|valAx><c:majorGridlines><c:spPr><a:ln>` gridline style (ECMA-376
/// §21.2.2.100, `CT_ChartLines` → DrawingML §20.1.2.2.24). The `<c:spPr>` on the
/// gridlines element styles the gridline stroke exactly like `<c:spPr>` on an
/// axis styles the axis rule, so this reuses the same `<a:ln>` resolver. Returns
/// `(color, width_emu)`: the resolved hex (no `#`) when the line carries a
/// `<a:solidFill>` (e.g. `accent3`), and the `<a:ln w>` width in EMU when
/// present. `(None, None)` when the axis omits `<c:majorGridlines>` or the
/// element carries no `<c:spPr><a:ln>` — the renderer then keeps its faint
/// default gridline. `<a:noFill>` is not exposed here: gridline PRESENCE is
/// already modeled by [`axis_has_major_gridlines`], so a no-fill gridline only
/// means "no explicit colour", handled by the `None` colour fallback.
pub fn extract_gridline_style(
    axis_node: Node,
    resolver: &dyn ColorResolver,
) -> (Option<String>, Option<u32>) {
    let Some(gridlines) = child(axis_node, "majorGridlines") else {
        return (None, None);
    };
    let (color, width, _no_fill) = extract_sp_pr_ln_style(gridlines, resolver);
    (color, width)
}

/// `<c:catAx|valAx><c:minorGridlines>` presence (ECMA-376 §21.2.2.109). Same
/// presence-only semantics as [`axis_has_major_gridlines`]. Minor gridlines
/// require a minor unit to place them; the renderer only draws them when both a
/// `<c:minorGridlines>` element and a resolvable minor step exist.
pub fn axis_has_minor_gridlines(axis_node: Node) -> bool {
    child(axis_node, "minorGridlines").is_some()
}

/// `<c:valAx><c:majorUnit val>` (ECMA-376 §21.2.2.103, `ST_AxisUnit`
/// §21.2.3.1) — an explicit distance between major ticks/gridlines. Must be a
/// positive floating-point number; non-positive values are rejected so they
/// can't wedge the renderer into an infinite gridline loop. `None` when absent
/// (the renderer keeps its Excel-style auto "nice" step).
pub fn extract_axis_major_unit(axis_node: Node) -> Option<f64> {
    child(axis_node, "majorUnit")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// `<c:valAx><c:minorUnit val>` (ECMA-376 §21.2.2.112) — explicit distance
/// between minor ticks/gridlines. Positive floating-point; `None` when absent.
pub fn extract_axis_minor_unit(axis_node: Node) -> Option<f64> {
    child(axis_node, "minorUnit")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
}

/// `<c:catAx|valAx><c:scaling><c:logBase val>` (ECMA-376 §21.2.2.98,
/// `ST_LogBase` §21.2.3.25) — the base of a logarithmic value axis. Per the
/// spec the base shall be `>= 2`; smaller/invalid values are rejected. `None`
/// when the axis is linear (the common case).
pub fn extract_axis_log_base(axis_node: Node) -> Option<f64> {
    let scaling = child(axis_node, "scaling")?;
    child(scaling, "logBase")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 2.0)
}

/// `<c:catAx|valAx><c:scaling><c:orientation val>` (ECMA-376 §21.2.2.130,
/// `ST_Orientation` §21.2.3.30) — axis direction. Returns the raw enum string
/// `"minMax"` (normal, the default) or `"maxMin"` (reversed). `None` when the
/// element is absent (the renderer treats absent and `"minMax"` identically, so
/// omitting it is byte-stable).
pub fn extract_axis_orientation(axis_node: Node) -> Option<String> {
    let scaling = child(axis_node, "scaling")?;
    child(scaling, "orientation")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string())
}

/// `<c:catAx|valAx><c:tickLblPos val>` (ECMA-376 §21.2.2.207, `ST_TickLblPos`
/// §21.2.3.47) — where the tick labels sit: `"high"` | `"low"` | `"nextTo"`
/// (default) | `"none"` (labels not drawn). Returns the raw enum string; `None`
/// when absent (renderer treats absent as `"nextTo"`, byte-stable).
pub fn extract_axis_tick_label_pos(axis_node: Node) -> Option<String> {
    child(axis_node, "tickLblPos")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string())
}

/// `<c:catAx|valAx><c:txPr><a:bodyPr rot>` (DrawingML `ST_Angle`, 60000ths of a
/// degree — §20.1.10.3) — tick-label rotation. Scoped to the axis's `<c:txPr>`
/// body properties so a title's rotation isn't misread. Returns the raw
/// 60000ths-degree integer; `None` when absent or 0 is not written (renderer
/// treats absent as 0, byte-stable). A value like `-2700000` = -45°.
pub fn extract_axis_tick_label_rotation(axis_node: Node) -> Option<i32> {
    let txpr = child(axis_node, "txPr")?;
    let body_pr = child(txpr, "bodyPr")?;
    body_pr.attribute("rot").and_then(|v| v.parse::<i32>().ok())
}

/// chartEx (`<cx:chartSpace>`) axis visibility. ChartEx encodes the
/// scale type via a `<cx:catScaling>` / `<cx:valScaling>` child rather
/// than separate `<c:catAx>` / `<c:valAx>` elements, so callers can't just
/// reuse `axis_is_deleted` — this helper walks `<cx:axis hidden="1">` and
/// pairs each one with its scaling kind.
///
/// Returns `(cat_hidden, val_hidden)`. Defaults to `(false, false)` when no
/// `<cx:axis>` declares `hidden`.
pub fn extract_chartex_axis_hidden(root: Node) -> (bool, bool) {
    let mut cat_hidden = false;
    let mut val_hidden = false;
    for ax in root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "axis")
    {
        let hidden = ax.attribute("hidden").map(|v| v == "1").unwrap_or(false);
        if !hidden {
            continue;
        }
        let is_val = ax
            .children()
            .any(|c| c.is_element() && c.tag_name().name() == "valScaling");
        let is_cat = ax
            .children()
            .any(|c| c.is_element() && c.tag_name().name() == "catScaling");
        if is_val {
            val_hidden = true;
        }
        if is_cat {
            cat_hidden = true;
        }
    }
    (cat_hidden, val_hidden)
}

/// Parse a modern chartEx part (`<cx:chartSpace>`, MS 2014 chartex namespace)
/// into the shared [`ChartModel`] — waterfall / treemap / etc.
///
/// This is the chartEx counterpart to [`parse_chart_part`]: the caller passes
/// the `<cx:chartSpace>` root and a [`ColorResolver`], and receives a bare
/// [`ChartModel`] (no graphic-frame geometry — the caller wraps it in its own
/// container). It was lifted verbatim from the pptx `parse_chartex` body; the
/// only behavioural change is that colour resolution routes through
/// `resolver.resolve_solid_fill` (instead of pptx's local `parse_color_node`)
/// and the theme fallback faces come from `resolver.theme_major_font_latin` /
/// `theme_minor_font_latin` (instead of a direct `theme.get("+mj-lt")` read).
/// A pptx `PptxColorResolver` returns identical values for both, so the emitted
/// JSON is byte-identical to the previous per-crate parse.
///
/// The chart type is the series `layoutId` string as-is (`"waterfall"`,
/// `"treemap"`, `"sunburst"`, `"boxWhisker"`, `"funnel"`, `"histogram"`, …);
/// this function does not gate on which layouts the renderer supports, so
/// adding a new chartEx layout is a renderer concern, not a parse concern.
///
/// Returns `None` when the part has no `<cx:series>` (not a chartEx chart).
pub fn parse_chartex_part(
    chartspace_root: Node,
    resolver: &dyn ColorResolver,
    style_xml: Option<&str>,
) -> Option<ChartModel> {
    let root = chartspace_root;

    // Chart type from series layoutId attribute.
    let series_node = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "series")?;
    let layout_id = attr(&series_node, "layoutId").unwrap_or_default();
    let chart_type = layout_id; // "waterfall", "treemap", etc.

    // ── chartEx title (MS 2014 chartex ext) ──────────────────────────────────
    // `<cx:chart><cx:title><cx:tx><cx:rich>` mirrors the DrawingML rich-text of a
    // legacy `<c:title>`; `flatten_rich_text` walks its `<a:p>/<a:r>/<a:t>`
    // identically. Only populated for the layouts CH15 renders (boxWhisker /
    // sunburst) so waterfall/treemap wire output stays byte-stable. `None`
    // otherwise.
    let renders_chartex = chart_type == "boxWhisker" || chart_type == "sunburst";
    let chartex_title = if renders_chartex {
        root.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "title")
            .and_then(|t| {
                t.descendants()
                    .find(|n| n.is_element() && n.tag_name().name() == "rich")
            })
            .map(|rich| flatten_rich_text(rich, None))
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    // ── chartEx title font size (MS 2014 chartex ext) ────────────────────────
    // Precedence: an explicit `sz` on the chartEx part's own `<cx:title>` rich
    // text wins; otherwise fall back to the associated chartStyle part's
    // `<cs:title><cs:defRPr@sz>` (Word's default modern style = 1400 = 14pt).
    // Without the style part a chartEx title would be sized by the renderer's
    // area-proportional guess (≈21pt on sample-24), 1.5× too large. Only
    // populated for the rendered layouts so waterfall/treemap stay byte-stable.
    let chartex_title_font_size_hpt = if renders_chartex {
        extract_chartex_title_size(root)
            .or_else(|| style_xml.and_then(extract_chartex_style_title_size))
    } else {
        None
    };

    // ── chartEx theme accent palette ─────────────────────────────────────────
    // boxWhisker series and sunburst branches color by index off the theme
    // accents (`accent[(idx % 6) + 1]`, the same cycle Office draws). Resolve
    // accent1..6 once here; `None` when the resolver owns no default palette
    // (pptx), letting the renderer fall back to its own `CHART_PALETTE`.
    let chartex_accents: Option<Vec<String>> =
        if chart_type == "boxWhisker" || chart_type == "sunburst" {
            let accents: Vec<String> = (0..6)
                .filter_map(|i| resolver.resolve_series_accent(i))
                .collect();
            if accents.len() == 6 {
                Some(accents)
            } else {
                None
            }
        } else {
            None
        };

    // ── chartEx box-and-whisker structured parse ─────────────────────────────
    let chartex_box = if chart_type == "boxWhisker" {
        parse_chartex_boxwhisker(root, resolver)
    } else {
        None
    };

    // ── chartEx sunburst structured parse ────────────────────────────────────
    let chartex_sunburst = if chart_type == "sunburst" {
        parse_chartex_sunburst(root)
    } else {
        None
    };

    // Categories from chartData > data > strDim[@type="cat"] > lvl > pt
    let categories: Vec<String> = root
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "strDim"
                && attr(n, "type").as_deref() == Some("cat")
        })
        .and_then(|dim| {
            dim.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "lvl")
        })
        .map(|lvl| {
            lvl.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "pt")
                .filter_map(|pt| pt.text().map(|t| t.replace('\n', " ")))
                .collect()
        })
        .unwrap_or_default();

    let pt_count = categories.len().max(1);

    // Values from chartData > data > numDim[@type="val"] > lvl > pt
    let raw_values: Vec<Option<f64>> = root
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "numDim"
                && attr(n, "type").as_deref() == Some("val")
        })
        .and_then(|dim| {
            dim.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "lvl")
        })
        .map(|lvl| {
            let mut vals: Vec<Option<f64>> = vec![None; pt_count];
            for (i, pt) in lvl
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "pt")
                .enumerate()
            {
                if i < vals.len() {
                    vals[i] = pt.text().and_then(|t| t.parse().ok());
                }
            }
            vals
        })
        .unwrap_or_else(|| vec![None; pt_count]);

    // Subtotal indices (idx=0 is always implicit; add from cx:subtotals)
    let mut subtotal_indices: Vec<u32> = vec![0];
    if let Some(subtotals_node) = series_node
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "subtotals")
    {
        for idx_node in subtotals_node
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "idx")
        {
            if let Some(v) = attr(&idx_node, "val").and_then(|v| v.parse::<u32>().ok()) {
                if v != 0 {
                    subtotal_indices.push(v);
                }
            }
        }
    }

    // Series color (first dataPt or series spPr)
    let color = series_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "spPr")
        .and_then(|sp| {
            sp.children()
                .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
        })
        .and_then(|fill| resolver.resolve_solid_fill(fill));

    // Per-idx data-label colors. ChartEx writes `<cx:dataLabels>` with
    // `<cx:dataLabel idx="N">` overrides; each carries its own `<cx:txPr>`
    // whose first `<a:solidFill>` is the label colour for that bar. Sample-2
    // waterfall uses this to paint negative-bar labels in accent1 (red) while
    // positive-bar labels stay tx1 (black).
    let mut data_label_colors_vec: Vec<Option<String>> = vec![None; raw_values.len().max(1)];
    let mut has_per_label_color = false;
    for dl in series_node
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dataLabel")
    {
        let Some(idx) = attr(&dl, "idx").and_then(|v| v.parse::<usize>().ok()) else {
            continue;
        };
        if idx >= data_label_colors_vec.len() {
            continue;
        }
        // First `<a:solidFill>` inside the per-idx <cx:txPr>.
        let txpr = match dl
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "txPr")
        {
            Some(n) => n,
            None => continue,
        };
        for desc in txpr.descendants().filter(|n| n.is_element()) {
            if desc.tag_name().name() != "solidFill" {
                continue;
            }
            if let Some(c) = resolver.resolve_solid_fill(desc) {
                data_label_colors_vec[idx] = Some(c);
                has_per_label_color = true;
                break;
            }
        }
    }

    let series = vec![ChartSeries {
        name: String::new(),
        values: raw_values,
        color,
        data_point_colors: None,
        data_label_colors: if has_per_label_color {
            Some(data_label_colors_vec)
        } else {
            None
        },
        categories: None,
        bubble_sizes: None,
        val_format_code: None,
        label_color: None,
        series_type: None,
        use_secondary_axis: None,
        show_marker: None,
        marker_symbol: None,
        marker_size: None,
        marker_fill: None,
        marker_line: None,
        data_point_overrides: None,
        data_label_overrides: None,
        series_data_labels: None,
        err_bars: None,
        // chartEx (waterfall) has no `<c:smooth>` concept.
        smooth: None,
        // chartEx series carry no classic `<c:trendline>`.
        trend_lines: None,
        // chartEx has no scatter connecting line to suppress.
        line_hidden: None,
    }];

    // ChartEx axis visibility — shared helper that pairs each `<cx:axis hidden>`
    // with its `<cx:catScaling>` / `<cx:valScaling>` child to disambiguate cat
    // vs. val (chartEx doesn't declare axis kind via the `id` attribute).
    let (cat_axis_hidden, val_axis_hidden) = extract_chartex_axis_hidden(root);

    // `<cx:catScaling gapWidth>` (chartEx) — same semantics as legacy
    // `<c:gapWidth>` but stored as a *fraction* (e.g. 0.8 ≡ 80%) instead of
    // an integer percentage. Convert to the legacy percentage form so the
    // shared renderer's `barW = catGap / (1 + gapWidth/100)` formula works
    // uniformly across chart types. Default 1.5 (= legacy 150%) per PowerPoint
    // when the attribute is omitted.
    let bar_gap_width = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "catScaling")
        .and_then(|n| attr(&n, "gapWidth"))
        .and_then(|v| v.parse::<f64>().ok())
        .map(|frac| (frac * 100.0).round() as i32);

    Some(ChartModel {
        chart_type,
        title: chartex_title,
        categories,
        series,
        // chartEx layouts color by branch/series index, not §21.2.2.227
        // varyColors (a `<c:>` chart-group element that chartEx has no analog
        // of), so the flag never applies here.
        vary_colors: None,
        val_max: None,
        val_min: None,
        subtotal_indices,
        show_data_labels: false,
        cat_axis_hidden,
        val_axis_hidden,
        plot_area_bg: None,
        chart_bg: None,
        show_legend: false,
        cat_axis_cross_between: "between".to_string(),
        val_axis_major_tick_mark: "cross".to_string(),
        cat_axis_major_tick_mark: "cross".to_string(),
        title_font_size_hpt: chartex_title_font_size_hpt,
        title_font_color: None,
        title_font_face: None,
        cat_axis_font_size_hpt: None,
        val_axis_font_size_hpt: None,
        cat_axis_font_color: None,
        val_axis_font_color: None,
        cat_axis_line_color: None,
        cat_axis_line_width_emu: None,
        cat_axis_line_hidden: false,
        val_axis_line_color: None,
        val_axis_line_width_emu: None,
        val_axis_line_hidden: false,
        data_label_font_size_hpt: None,
        legend_pos: None,
        bar_gap_width,
        bar_overlap: None,
        data_label_position: None,
        data_label_font_color: None,
        data_label_format_code: None,
        val_axis_format_code: None,
        plot_area_manual_layout: None,
        scatter_style: None,
        // chartEx (waterfall/treemap/etc.) has its own axis model and is not
        // wired for axis titles or an explicit chartSpace border yet.
        cat_axis_title: None,
        val_axis_title: None,
        cat_axis_title_font_size_hpt: None,
        cat_axis_title_font_bold: None,
        cat_axis_title_font_color: None,
        val_axis_title_font_size_hpt: None,
        val_axis_title_font_bold: None,
        val_axis_title_font_color: None,
        title_font_bold: None,
        cat_axis_font_bold: None,
        val_axis_font_bold: None,
        chart_border_color: None,
        chart_border_width_emu: None,
        secondary_val_axis: None,
        // chartEx charts (waterfall/treemap/etc.) are not pie/doughnut and
        // don't carry `<c:txPr>` axis/legend faces; only the theme fallback
        // fonts are threaded so their data labels can pick up the body font.
        hole_size: None,
        first_slice_angle: None,
        cat_axis_font_face: None,
        val_axis_font_face: None,
        cat_axis_title_font_face: None,
        val_axis_title_font_face: None,
        data_label_font_face: None,
        legend_font_face: None,
        legend_font_color: None,
        legend_font_size_hpt: None,
        legend_font_bold: None,
        theme_major_font_latin: resolver.theme_major_font_latin(),
        theme_minor_font_latin: resolver.theme_minor_font_latin(),
        val_axis_minor_tick_mark: None,
        cat_axis_minor_tick_mark: None,
        legend_manual_layout: None,
        title_manual_layout: None,
        cat_axis_crosses: None,
        cat_axis_crosses_at: None,
        val_axis_crosses: None,
        val_axis_crosses_at: None,
        cat_axis_format_code: None,
        cat_axis_min: None,
        cat_axis_max: None,
        radar_style: None,
        // chartEx (cx: namespace) has its own date-axis model; the legacy
        // `<c:date1904>` element does not apply here, so keep the 1900
        // default until/unless a chartEx date system is wired.
        date1904: false,
        // chartEx waterfall has no line/area blanks to display.
        disp_blanks_as: None,
        // chartEx (cx:) has its own axis model (`<cx:axis>`); the classic
        // `<c:catAx>`/`<c:valAx>` scale properties don't apply, so leave the
        // CH6 fields unset — the renderer keeps its defaults (byte-stable).
        val_axis_major_gridlines: None,
        cat_axis_major_gridlines: None,
        val_axis_gridline_color: None,
        val_axis_gridline_width_emu: None,
        cat_axis_gridline_color: None,
        cat_axis_gridline_width_emu: None,
        val_axis_minor_gridlines: None,
        val_axis_major_unit: None,
        val_axis_minor_unit: None,
        val_axis_log_base: None,
        val_axis_orientation: None,
        cat_axis_orientation: None,
        cat_axis_tick_label_pos: None,
        val_axis_tick_label_pos: None,
        cat_axis_label_rotation: None,
        stock_hi_low_lines: None,
        stock_hi_low_line_color: None,
        stock_up_down_bars: None,
        chartex_box,
        chartex_sunburst,
        chartex_accents,
    })
}

/// Parse the structured box-and-whisker data of a chartEx `boxWhisker`.
///
/// A box-and-whisker chart has one `<cx:series layoutId="boxWhisker">` per data
/// column; each series' `<cx:dataId val="N">` selects a `<cx:data id="N">`
/// carrying RAW sample points (a `<cx:strDim type="cat">` of per-point category
/// labels and a `<cx:numDim type="val">` of the sample values). This groups
/// each series' points by the unique categories (taken in first-seen order from
/// the first series' data) and threads the `<cx:layoutPr>` visibility /
/// statistics flags. Quartiles / mean / whiskers / outliers are the renderer's
/// job. Returns `None` when there is no plottable series.
fn parse_chartex_boxwhisker(root: Node, resolver: &dyn ColorResolver) -> Option<ChartexBoxWhisker> {
    // Build id -> <cx:data> lookup.
    let data_by_id: std::collections::HashMap<String, Node> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "data")
        .filter_map(|d| attr(&d, "id").map(|id| (id, d)))
        .collect();

    // Series nodes, in document order (one column each).
    let series_nodes: Vec<Node> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "series")
        .collect();
    if series_nodes.is_empty() {
        return None;
    }

    // Per-series raw (category-label, value) points, resolving each series' own
    // <cx:dataId> -> <cx:data>.
    let per_series_points: Vec<Vec<(String, f64)>> = series_nodes
        .iter()
        .map(|s| {
            let data_id = s
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "dataId")
                .and_then(|n| attr(&n, "val"));
            let data = data_id.as_ref().and_then(|id| data_by_id.get(id).copied());
            match data {
                Some(d) => chartex_data_cat_val_points(d),
                None => Vec::new(),
            }
        })
        .collect();

    // Unique categories in first-seen order across all series (first series'
    // order dominates; later series only contribute unseen labels).
    let mut categories: Vec<String> = Vec::new();
    for pts in &per_series_points {
        for (cat, _) in pts {
            if !categories.iter().any(|c| c == cat) {
                categories.push(cat.clone());
            }
        }
    }
    if categories.is_empty() {
        return None;
    }
    let cat_index: std::collections::HashMap<&str, usize> = categories
        .iter()
        .enumerate()
        .map(|(i, c)| (c.as_str(), i))
        .collect();

    let series: Vec<ChartexBoxSeries> = series_nodes
        .iter()
        .enumerate()
        .map(|(si, s)| {
            let name = s
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "txData")
                .and_then(|tx| tx.children().find(|n| n.tag_name().name() == "v"))
                .and_then(|v| v.text().map(|t| t.to_string()))
                .unwrap_or_default();

            // Bin this series' raw points into the shared category order.
            let mut values_by_category: Vec<Vec<f64>> = vec![Vec::new(); categories.len()];
            for (cat, v) in &per_series_points[si] {
                if let Some(&ci) = cat_index.get(cat.as_str()) {
                    values_by_category[ci].push(*v);
                }
            }

            // `<cx:layoutPr><cx:visibility …>` flags; Office defaults when omitted:
            // meanMarker on, meanLine off, outliers on, nonoutliers off.
            let vis = s
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "visibility");
            let bool_attr = |name: &str, dflt: bool| {
                vis.and_then(|v| attr(&v, name))
                    .map(|s| s == "1" || s == "true")
                    .unwrap_or(dflt)
            };
            let quartile_method = s
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "statistics")
                .and_then(|st| attr(&st, "quartileMethod"))
                .unwrap_or_else(|| "exclusive".to_string());

            ChartexBoxSeries {
                name,
                color: resolver.resolve_series_accent(si),
                values_by_category,
                mean_marker: bool_attr("meanMarker", true),
                mean_line: bool_attr("meanLine", false),
                show_outliers: bool_attr("outliers", true),
                show_nonoutliers: bool_attr("nonoutliers", false),
                quartile_method,
            }
        })
        .collect();

    Some(ChartexBoxWhisker { categories, series })
}

/// Collect a chartEx `<cx:data>`'s (category-label, value) sample points: the
/// `<cx:strDim type="cat">` first `<cx:lvl>` supplies per-point labels, the
/// `<cx:numDim type="val">` first `<cx:lvl>` the values. Points align by their
/// `idx` attribute; a point with no numeric value is dropped.
fn chartex_data_cat_val_points(data: Node) -> Vec<(String, f64)> {
    let cat_lvl = data
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "strDim"
                && attr(n, "type").as_deref() == Some("cat")
        })
        .and_then(|dim| {
            dim.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "lvl")
        });
    let val_lvl = data
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "numDim"
                && matches!(attr(n, "type").as_deref(), Some("val") | Some("size"))
        })
        .and_then(|dim| {
            dim.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "lvl")
        });
    let (Some(cat_lvl), Some(val_lvl)) = (cat_lvl, val_lvl) else {
        return Vec::new();
    };

    // Index the category labels by their `idx`.
    let cats: std::collections::HashMap<usize, String> = cat_lvl
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "pt")
        .filter_map(|pt| {
            let idx = attr(&pt, "idx").and_then(|v| v.parse::<usize>().ok())?;
            let label = pt.text().unwrap_or("").to_string();
            Some((idx, label))
        })
        .collect();

    val_lvl
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "pt")
        .filter_map(|pt| {
            let idx = attr(&pt, "idx").and_then(|v| v.parse::<usize>().ok())?;
            let v = pt.text().and_then(|t| t.parse::<f64>().ok())?;
            let cat = cats.get(&idx).cloned().unwrap_or_default();
            Some((cat, v))
        })
        .collect()
}

/// Parse the structured hierarchy of a chartEx `sunburst`.
///
/// A sunburst's single `<cx:data>` carries a `<cx:strDim type="cat">` with
/// several `<cx:lvl>` (lvl[0] = deepest / Leaf, subsequent lvls step toward the
/// root, last lvl = Branch) and one `<cx:numDim type="size">`. Each data-point
/// `idx` yields a root→leaf `path` (Branch, …, Leaf) with empty trailing
/// segments trimmed — a node that is itself a leaf terminates before the
/// deepest level — and the `size` value at that `idx`. Returns `None` when
/// there is no size dimension or no rows.
fn parse_chartex_sunburst(root: Node) -> Option<ChartexSunburst> {
    let cat_dim = root.descendants().find(|n| {
        n.is_element()
            && n.tag_name().name() == "strDim"
            && attr(n, "type").as_deref() == Some("cat")
    })?;
    // Levels in document order: lvl[0] = Leaf (deepest), last = Branch (root).
    let levels: Vec<Node> = cat_dim
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "lvl")
        .collect();
    if levels.is_empty() {
        return None;
    }

    // size dimension (chartEx sunburst uses type="size").
    let size_lvl = root
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "numDim"
                && matches!(attr(n, "type").as_deref(), Some("size") | Some("val"))
        })
        .and_then(|dim| {
            dim.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "lvl")
        })?;
    let sizes: std::collections::HashMap<usize, f64> = size_lvl
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "pt")
        .filter_map(|pt| {
            let idx = attr(&pt, "idx").and_then(|v| v.parse::<usize>().ok())?;
            let v = pt.text().and_then(|t| t.parse::<f64>().ok())?;
            Some((idx, v))
        })
        .collect();

    // Per-level idx -> label maps.
    let level_maps: Vec<std::collections::HashMap<usize, String>> = levels
        .iter()
        .map(|lvl| {
            lvl.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "pt")
                .filter_map(|pt| {
                    let idx = attr(&pt, "idx").and_then(|v| v.parse::<usize>().ok())?;
                    let label = pt.text().unwrap_or("").to_string();
                    Some((idx, label))
                })
                .collect()
        })
        .collect();

    // Row count = the max ptCount / max idx across levels + sizes.
    let n = sizes
        .keys()
        .chain(level_maps.iter().flat_map(|m| m.keys()))
        .copied()
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);

    let mut rows: Vec<ChartexSunburstRow> = Vec::new();
    for idx in 0..n {
        let size = *sizes.get(&idx).unwrap_or(&0.0);
        // Build path root→leaf: iterate levels from LAST (Branch/root) to FIRST
        // (Leaf/deepest). Trailing empty leaf cells are trimmed so a node that is
        // itself a leaf terminates early.
        let mut path: Vec<String> = Vec::new();
        for lvl in level_maps.iter().rev() {
            let label = lvl.get(&idx).cloned().unwrap_or_default();
            if label.is_empty() {
                // Empty deeper cell terminates the path (no further descent).
                break;
            }
            path.push(label);
        }
        if path.is_empty() {
            continue;
        }
        rows.push(ChartexSunburstRow { path, size });
    }
    if rows.is_empty() {
        return None;
    }
    Some(ChartexSunburst { rows })
}

// ============================================================================
// Series-detail extractors (markers, per-point overrides, data labels, error
// bars) — moved verbatim from the xlsx crate so `parse_chart_part` populates
// the rich per-series fields for both pptx and xlsx.
// ============================================================================

/// Parse `<c:marker>` into `(symbol, size, fill, line)` — colors are hex without
/// `#`. ECMA-376 §21.2.2.32 / §21.2.2.34. Fill and line come from `<c:spPr>`
/// nested inside the marker, resolved via the full DrawingML color grammar
/// ([`ColorResolver::resolve_shape_fill`]). `size` is the point value parsed as
/// an integer (matching Excel's `<c:size val>` unsignedByte) then widened to
/// `f64` for the shared model.
pub fn parse_marker_block(
    marker_node: Option<Node>,
    resolver: &dyn ColorResolver,
) -> (Option<String>, Option<f64>, Option<String>, Option<String>) {
    let Some(mk) = marker_node else {
        return (None, None, None, None);
    };
    let symbol = child(mk, "symbol")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string());
    let size = child(mk, "size")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<u32>().ok())
        .map(|v| v as f64);
    let sp_pr = child(mk, "spPr");
    let fill = sp_pr.and_then(|p| resolver.resolve_shape_fill(p));
    let line = sp_pr
        .and_then(|p| child(p, "ln"))
        .and_then(|ln| resolver.resolve_shape_fill(ln));
    (symbol, size, fill, line)
}

/// Walk every `<c:dPt>` direct child of the series and collect per-point
/// overrides. Multiple `<c:dPt>` per series is normal; each targets one
/// `<c:idx>` (ECMA-376 §21.2.2.39). Fill from `<c:spPr>`, marker from a nested
/// `<c:marker>`, and `<c:explosion>` (pie/doughnut pull-out) are captured.
pub fn parse_data_point_overrides(
    ser_node: Node,
    resolver: &dyn ColorResolver,
) -> Vec<ChartDataPointOverride> {
    let mut result = Vec::new();
    for dpt in ser_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "dPt")
    {
        let idx = child(dpt, "idx")
            .and_then(|n| n.attribute("val"))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let color = child(dpt, "spPr").and_then(|p| resolver.resolve_shape_fill(p));
        let mk = child(dpt, "marker");
        let (marker_symbol, marker_size, marker_fill, marker_line) =
            parse_marker_block(mk, resolver);
        let explosion = extract_dpt_explosion(dpt);
        result.push(ChartDataPointOverride {
            idx,
            color,
            marker_symbol,
            marker_size,
            marker_fill,
            marker_line,
            explosion,
        });
    }
    result
}

/// Resolve `<c:ser><c:extLst><c:ext><c15:datalabelsRange>` cache: index → label
/// text. Used to substitute `<a:fld type="CELLRANGE">` placeholders. Missing
/// entries stay absent from the map.
pub fn collect_dlbl_range_cache(ser_node: Node) -> std::collections::HashMap<u32, String> {
    let mut map: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let Some(ext_lst) = child(ser_node, "extLst") else {
        return map;
    };
    for ext in ext_lst
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "ext")
    {
        for range in ext
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "datalabelsRange")
        {
            for cache in range
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "dlblRangeCache")
            {
                for pt in cache
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "pt")
                {
                    let Some(idx) = pt.attribute("idx").and_then(|v| v.parse::<u32>().ok()) else {
                        continue;
                    };
                    let v = child(pt, "v")
                        .and_then(|n| n.text())
                        .unwrap_or("")
                        .to_string();
                    map.insert(idx, v);
                }
            }
        }
    }
    map
}

/// Walk a `<c:tx><c:rich>` (or any DrawingML rich-text root) and reduce it to
/// plain text. `<a:fld type="CELLRANGE">` placeholders are substituted from
/// `cellrange_cache`. Other field types and runs are concatenated; newlines
/// come from paragraph breaks.
pub fn flatten_rich_text(rich_root: Node, cellrange_cache: Option<&str>) -> String {
    let mut out = String::new();
    let mut first_para = true;
    for p in rich_root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "p")
    {
        if !first_para {
            out.push('\n');
        }
        first_para = false;
        for c in p.children().filter(|n| n.is_element()) {
            match c.tag_name().name() {
                "r" => {
                    if let Some(t) = c.children().find(|n| n.tag_name().name() == "t") {
                        if let Some(s) = t.text() {
                            out.push_str(s);
                        }
                    }
                }
                "fld" => {
                    let typ = c.attribute("type").unwrap_or("");
                    if typ == "CELLRANGE" {
                        if let Some(s) = cellrange_cache {
                            out.push_str(s);
                        }
                    } else if let Some(t) = c.children().find(|n| n.tag_name().name() == "t") {
                        if let Some(s) = t.text() {
                            out.push_str(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Parse a data-label `<c:spPr>` (§21.2.2.197) into a callout [`ChartLabelBox`]
/// (fill + border). Returns `None` when the shape node is absent OR carries
/// neither a resolvable fill nor a border — i.e. nothing that would draw a box.
/// The direct-child `<a:solidFill>` is the box fill; the `<a:ln>` solidFill and
/// its `w` attribute are the border. Colors resolve through
/// [`ColorResolver::resolve_shape_fill`] so a `<a:sysClr>`/`<a:schemeClr>` picks
/// up its transforms (Office writes the default white box as
/// `<a:sysClr val="window">`).
fn parse_label_box(sp_pr: Option<Node>, resolver: &dyn ColorResolver) -> Option<ChartLabelBox> {
    let sp = sp_pr?;
    let fill = resolver.resolve_shape_fill(sp);
    let (border_color, border_width_emu) = match child(sp, "ln") {
        None => (None, None),
        Some(ln) => {
            let color = resolver.resolve_shape_fill(ln);
            let width = ln.attribute("w").and_then(|v| v.parse::<u32>().ok());
            (color, width)
        }
    };
    if fill.is_none() && border_color.is_none() && border_width_emu.is_none() {
        return None;
    }
    Some(ChartLabelBox {
        fill,
        border_color,
        border_width_emu,
    })
}

/// Parse `<c:dLbls><c:leaderLines>` into `(show_leader_lines, color, width_emu)`.
/// `show` comes from the sibling `<c:showLeaderLines val>` (§21.2.2.183); the
/// stroke style comes from `<c:leaderLines>` (§21.2.2.92) `<c:spPr><a:ln>`.
fn parse_leader_lines(
    d_lbls: Node,
    resolver: &dyn ColorResolver,
) -> (bool, Option<String>, Option<u32>) {
    // §21.2.2.183 `<c:showLeaderLines>` — CT_Boolean, so a bare element ⇒ true;
    // absent ⇒ false (no leader lines by default).
    let show = bool_child(d_lbls, "showLeaderLines").unwrap_or(false);
    let (color, width) = match child(d_lbls, "leaderLines")
        .and_then(|ll| child(ll, "spPr"))
        .and_then(|sp| child(sp, "ln"))
    {
        None => (None, None),
        Some(ln) => (
            resolver.resolve_shape_fill(ln),
            ln.attribute("w").and_then(|v| v.parse::<u32>().ok()),
        ),
    };
    (show, color, width)
}

/// Parse a series-level `<c:dLbls>` into `(series_defaults, per_idx_overrides)`.
/// ECMA-376 §21.2.2.47. Colors resolve through [`ColorResolver::resolve_shape_fill`]
/// so a scheme-color label text picks up its lumMod/lumOff transforms.
pub fn parse_series_data_labels(
    ser_node: Node,
    resolver: &dyn ColorResolver,
    cellrange_cache: &std::collections::HashMap<u32, String>,
) -> (Option<ChartSeriesDataLabels>, Vec<ChartDataLabelOverride>) {
    let Some(d_lbls) = child(ser_node, "dLbls") else {
        return (None, Vec::new());
    };

    // CT_Boolean show-flag: element present ⇒ true unless `val` explicitly
    // disables it (§21.2.2, dml-chart.xsd `val` default `true`); element absent
    // ⇒ false (the flag defaults off when the deck names no show-flag element).
    let bool_attr = |n: Node, name: &str| bool_child(n, name).unwrap_or(false);

    let position = child(d_lbls, "dLblPos")
        .and_then(|n| n.attribute("val"))
        .map(|s| s.to_string());
    let format_code = child(d_lbls, "numFmt")
        .and_then(|n| n.attribute("formatCode"))
        .map(|s| s.to_string());
    // defRPr fill / bold / size come from the dLbls-level `<c:txPr>`.
    let txpr = child(d_lbls, "txPr");
    let font_color = txpr.and_then(|tx| {
        tx.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "defRPr")
            .and_then(|def| resolver.resolve_shape_fill(def))
    });
    let font_bold_default = txpr.and_then(|tx| {
        tx.descendants()
            .find(|n| {
                n.is_element() && (n.tag_name().name() == "defRPr" || n.tag_name().name() == "rPr")
            })
            .and_then(|n| n.attribute("b"))
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    });
    let font_size_default = txpr.and_then(|tx| {
        tx.descendants()
            .find(|n| {
                n.is_element() && (n.tag_name().name() == "defRPr" || n.tag_name().name() == "rPr")
            })
            .and_then(|n| n.attribute("sz"))
            .and_then(|v| v.parse::<i32>().ok())
    });

    // §21.2.2.197 series-level callout-box shape (`<c:dLbls><c:spPr>`) and
    // §21.2.2.183/§21.2.2.92 leader-line style. `<c:spPr>` may appear both as a
    // direct child of `<c:dLbls>` (the series default) and inside each
    // `<c:dLbl>` (per-point) — pick the direct child here.
    let label_box = parse_label_box(
        d_lbls
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "spPr"),
        resolver,
    );
    let (show_leader_lines, leader_line_color, leader_line_width_emu) =
        parse_leader_lines(d_lbls, resolver);

    let series_defaults = ChartSeriesDataLabels {
        show_val: bool_attr(d_lbls, "showVal"),
        show_cat_name: bool_attr(d_lbls, "showCatName"),
        show_ser_name: bool_attr(d_lbls, "showSerName"),
        show_percent: bool_attr(d_lbls, "showPercent"),
        position: position.clone(),
        font_color: font_color.clone(),
        format_code,
        font_bold: font_bold_default,
        font_size_hpt: font_size_default,
        label_box,
        show_leader_lines,
        leader_line_color,
        leader_line_width_emu,
    };

    let mut overrides = Vec::new();
    for dl in d_lbls
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbl")
    {
        let idx = child(dl, "idx")
            .and_then(|n| n.attribute("val"))
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        // §21.2.2.43 per-point `<c:delete>` — CT_Boolean, so a bare
        // `<c:delete/>` removes this point's label (val default true).
        let deleted = bool_child(dl, "delete").unwrap_or(false);
        let pos = child(dl, "dLblPos")
            .and_then(|n| n.attribute("val"))
            .map(|s| s.to_string());
        let cache_for_idx = cellrange_cache.get(&idx).map(|s| s.as_str());
        let text = if deleted {
            String::new()
        } else {
            match child(dl, "tx") {
                Some(tx_node) => flatten_rich_text(tx_node, cache_for_idx),
                None => cache_for_idx.unwrap_or("").to_string(),
            }
        };
        let font_color = child(dl, "txPr").and_then(|tx| {
            tx.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "defRPr")
                .and_then(|def| resolver.resolve_shape_fill(def))
        });
        let font_size_hpt = dl
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "defRPr")
            .and_then(|def| def.attribute("sz"))
            .and_then(|v| v.parse::<i32>().ok());
        let font_bold = dl
            .descendants()
            .find(|n| {
                n.is_element() && (n.tag_name().name() == "defRPr" || n.tag_name().name() == "rPr")
            })
            .and_then(|def| def.attribute("b"))
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        // Per-point callout box (`<c:dLbl>` §21.2.2.47 `<c:spPr>` §21.2.2.197):
        // direct child spPr overrides the series-default box for this one point.
        let label_box = parse_label_box(
            dl.children()
                .find(|n| n.is_element() && n.tag_name().name() == "spPr"),
            resolver,
        );
        // Per-point show-flags (§21.2.2.47 CT_DLbl carries the same show-flag
        // group as CT_DLbls). Read as `Option` so an absent flag falls through
        // to the series default; a present flag overrides it for this point.
        // CT_Boolean: a present-but-`val`-omitted flag is `Some(true)`.
        let opt_bool_flag = |name: &str| -> Option<bool> { bool_child(dl, name) };
        overrides.push(ChartDataLabelOverride {
            idx,
            text,
            position: pos,
            font_color,
            font_size_hpt,
            font_bold,
            label_box,
            show_val: opt_bool_flag("showVal"),
            show_cat_name: opt_bool_flag("showCatName"),
            show_ser_name: opt_bool_flag("showSerName"),
            show_percent: opt_bool_flag("showPercent"),
            // §21.2.2.43 `<c:delete>` — record genuine deletes distinctly from a
            // style-only `<c:dLbl>` so the renderer never mistakes an empty tx
            // (compose-from-flags) for a removed label.
            deleted: if deleted { Some(true) } else { None },
        });
    }

    let any_default = series_defaults.show_val
        || series_defaults.show_cat_name
        || series_defaults.show_ser_name
        || series_defaults.show_percent
        || series_defaults.position.is_some()
        || series_defaults.font_color.is_some()
        || series_defaults.format_code.is_some()
        || series_defaults.font_bold.is_some()
        || series_defaults.font_size_hpt.is_some()
        || series_defaults.label_box.is_some()
        || series_defaults.show_leader_lines
        || series_defaults.leader_line_color.is_some()
        || series_defaults.leader_line_width_emu.is_some();
    let series_out = if any_default {
        Some(series_defaults)
    } else {
        None
    };
    (series_out, overrides)
}

/// Read a `<c:numRef><c:numCache>` or `<c:numLit>` block under `parent` and
/// return per-point values keyed by `<c:pt idx>`. Length is at least
/// `expected_len` (padded with `None`).
pub fn extract_num_block(parent: Node, expected_len: usize) -> Vec<Option<f64>> {
    let cache = parent.descendants().find(|n| {
        n.is_element() && (n.tag_name().name() == "numCache" || n.tag_name().name() == "numLit")
    });
    let Some(cache) = cache else {
        return Vec::new();
    };
    let pt_count: usize = child(cache, "ptCount")
        .and_then(|n| n.attribute("val"))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(expected_len);
    let len = pt_count.max(expected_len);
    let mut values: Vec<Option<f64>> = vec![None; len];
    for pt in cache
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "pt")
    {
        let Some(idx) = pt.attribute("idx").and_then(|v| v.parse::<usize>().ok()) else {
            continue;
        };
        let v = child(pt, "v")
            .and_then(|n| n.text())
            .and_then(|s| s.trim().parse::<f64>().ok());
        if idx < values.len() {
            values[idx] = v;
        }
    }
    values
}

/// Parse all `<c:errBars>` direct children of a series and resolve per-point
/// plus / minus deltas to absolute numbers. Each errBars block fixes a
/// direction (x|y); a series can have at most one of each direction.
/// ECMA-376 §21.2.2.20.
pub fn parse_error_bars(
    ser_node: Node,
    series_values: &[Option<f64>],
    resolver: &dyn ColorResolver,
) -> Vec<ChartErrBars> {
    let mut result = Vec::new();
    for eb in ser_node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "errBars")
    {
        let dir = child(eb, "errDir")
            .and_then(|n| n.attribute("val"))
            .unwrap_or("y")
            .to_string();
        let bar_type = child(eb, "errBarType")
            .and_then(|n| n.attribute("val"))
            .unwrap_or("both")
            .to_string();
        let val_type = child(eb, "errValType")
            .and_then(|n| n.attribute("val"))
            .unwrap_or("fixedVal")
            .to_string();
        // §21.2.2.117 `<c:noEndCap>` — CT_Boolean, so a bare element ⇒ true (no
        // I-beam end caps); absent ⇒ false (draw end caps by default).
        let no_end_cap = bool_child(eb, "noEndCap").unwrap_or(false);

        let n_points = series_values.len();
        let mut plus: Vec<Option<f64>> = vec![None; n_points];
        let mut minus: Vec<Option<f64>> = vec![None; n_points];

        match val_type.as_str() {
            "cust" => {
                for (slot, target) in [("plus", &mut plus), ("minus", &mut minus)] {
                    let Some(side) = child(eb, slot) else {
                        continue;
                    };
                    let vals = extract_num_block(side, n_points);
                    if !vals.is_empty() {
                        let len = vals.len().min(target.len());
                        target[..len].copy_from_slice(&vals[..len]);
                    }
                }
            }
            "fixedVal" => {
                let v = child(eb, "val")
                    .and_then(|n| n.attribute("val"))
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                for i in 0..n_points {
                    plus[i] = Some(v);
                    minus[i] = Some(v);
                }
            }
            "percentage" => {
                let pct = child(eb, "val")
                    .and_then(|n| n.attribute("val"))
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);
                for (i, v) in series_values.iter().enumerate() {
                    if let Some(val) = v {
                        let d = val.abs() * pct / 100.0;
                        plus[i] = Some(d);
                        minus[i] = Some(d);
                    }
                }
            }
            "stdErr" | "stdDev" => {
                let nums: Vec<f64> = series_values.iter().filter_map(|v| *v).collect();
                if !nums.is_empty() {
                    let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                    let var =
                        nums.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                    let std = var.sqrt();
                    let mult = child(eb, "val")
                        .and_then(|n| n.attribute("val"))
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(1.0);
                    let sample = if val_type == "stdErr" {
                        std / (nums.len() as f64).sqrt()
                    } else {
                        std
                    };
                    let delta = sample * mult;
                    for i in 0..n_points {
                        plus[i] = Some(delta);
                        minus[i] = Some(delta);
                    }
                }
            }
            _ => {}
        }

        let sp_pr = child(eb, "spPr");
        let color = sp_pr.and_then(|p| match child(p, "ln") {
            Some(l) => resolver.resolve_shape_fill(l),
            None => resolver.resolve_shape_fill(p),
        });
        let line_width_emu = sp_pr
            .and_then(|p| child(p, "ln"))
            .and_then(|ln| ln.attribute("w"))
            .and_then(|v| v.parse::<u32>().ok());
        let dash = sp_pr
            .and_then(|p| child(p, "ln"))
            .and_then(|ln| child(ln, "prstDash"))
            .and_then(|n| n.attribute("val"))
            .map(|s| s.to_string());

        result.push(ChartErrBars {
            dir,
            bar_type,
            plus,
            minus,
            no_end_cap,
            color,
            line_width_emu,
            dash,
        });
    }
    result
}

/// Positional string-cache collector for `<c:cat>` / `<c:xVal>`. Reads
/// `<c:ptCount>` to size the result, then places each `<c:pt idx>` string at its
/// index (multi-level caches use the innermost `<c:lvl>`). Unlike a naive
/// document-order collector this preserves gaps (sparse caches) so a category
/// list that starts at `idx=1`, or a value series with a hole, keeps its true
/// length and alignment (ECMA-376 §21.2.2.20/.75/.181).
pub fn collect_str_cache_positional(ser_node: Node, child_tag: &str) -> Vec<String> {
    let Some(container) = ser_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == child_tag)
    else {
        return Vec::new();
    };

    // Multi-level categories: use only the first (innermost) lvl.
    if let Some(multi_cache) = container
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "multiLvlStrCache")
    {
        let pt_count: usize = child(multi_cache, "ptCount")
            .and_then(|n| n.attribute("val"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if let Some(first_lvl) = child(multi_cache, "lvl") {
            let mut pts: Vec<(usize, String)> = Vec::new();
            for pt in first_lvl
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "pt")
            {
                let idx: usize = pt
                    .attribute("idx")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let val = child(pt, "v")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string();
                pts.push((idx, val));
            }
            let len = pt_count.max(pts.iter().map(|(i, _)| i + 1).max().unwrap_or(0));
            let mut result = vec![String::new(); len];
            for (idx, val) in pts {
                if idx < result.len() {
                    result[idx] = val;
                }
            }
            return result;
        }
    }

    // Standard strRef/strCache or numRef/numCache.
    let mut pt_count: usize = 0;
    let mut pts: Vec<(usize, String)> = Vec::new();
    for desc in container.descendants() {
        match desc.tag_name().name() {
            "ptCount" => {
                pt_count = desc
                    .attribute("val")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
            "pt" => {
                let idx: usize = desc
                    .attribute("idx")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let val = child(desc, "v")
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .to_string();
                pts.push((idx, val));
            }
            _ => {}
        }
    }
    if pt_count == 0 {
        pt_count = pts.len();
    }
    let mut result = vec![String::new(); pt_count];
    for (idx, val) in pts {
        if idx < result.len() {
            result[idx] = val;
        }
    }
    result
}

/// Positional numeric-cache collector for `<c:val>` / `<c:yVal>`. Reads
/// `<c:ptCount>` to size the result, then places each `<c:pt idx>` value at its
/// index (padding gaps with `None`). Sparse-safe companion to
/// [`collect_str_cache_positional`].
pub fn collect_num_cache_positional(ser_node: Node, child_tag: &str) -> Vec<Option<f64>> {
    let Some(container) = ser_node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == child_tag)
    else {
        return Vec::new();
    };

    let mut pt_count: usize = 0;
    let mut pts: Vec<(usize, f64)> = Vec::new();
    for desc in container.descendants() {
        match desc.tag_name().name() {
            "ptCount" => {
                pt_count = desc
                    .attribute("val")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
            "pt" => {
                let idx: usize = desc
                    .attribute("idx")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                if let Some(v) = child(desc, "v")
                    .and_then(|n| n.text())
                    .and_then(|t| t.parse::<f64>().ok())
                {
                    pts.push((idx, v));
                }
            }
            _ => {}
        }
    }
    if pt_count == 0 {
        pt_count = pts.len();
    }
    let mut result: Vec<Option<f64>> = vec![None; pt_count];
    for (idx, val) in pts {
        if idx < result.len() {
            result[idx] = Some(val);
        }
    }
    result
}

/// Package-specific resolver for legacy chart formulas whose authored cache or
/// literal is absent. DrawingML owns the series-field walk; a host package such
/// as XLSX supplies only the external data lookup. DOCX and PPTX use the
/// no-op resolver through [`parse_chart_part`].
pub trait ChartReferenceResolver {
    /// `None` means the host could not resolve the formula. `Some` with an
    /// empty or all-gap vector is a successfully resolved empty range.
    fn resolve_strings(&mut self, formula: &str) -> Option<Vec<String>>;
    fn resolve_numbers(&mut self, formula: &str) -> Option<Vec<Option<f64>>>;
}

struct EmptyChartReferenceResolver;

impl ChartReferenceResolver for EmptyChartReferenceResolver {
    fn resolve_strings(&mut self, _formula: &str) -> Option<Vec<String>> {
        None
    }

    fn resolve_numbers(&mut self, _formula: &str) -> Option<Vec<Option<f64>>> {
        None
    }
}

fn has_authored_reference_data(container: Node<'_, '_>) -> bool {
    container.descendants().any(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "strCache" | "numCache" | "multiLvlStrCache" | "strLit" | "numLit"
            )
    })
}

fn reference_formula(container: Node<'_, '_>) -> Option<String> {
    container
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "f")
        .and_then(|node| node.text())
        .map(str::trim)
        .filter(|formula| !formula.is_empty())
        .map(str::to_owned)
}

fn collect_string_source(
    ser_node: Node<'_, '_>,
    child_tag: &str,
    references: &mut dyn ChartReferenceResolver,
) -> Option<Vec<String>> {
    let container = ser_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_tag)?;
    if has_authored_reference_data(container) {
        return Some(collect_str_cache_positional(ser_node, child_tag));
    }
    reference_formula(container).and_then(|formula| references.resolve_strings(&formula))
}

fn collect_number_source(
    ser_node: Node<'_, '_>,
    child_tag: &str,
    references: &mut dyn ChartReferenceResolver,
) -> Option<Vec<Option<f64>>> {
    let container = ser_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_tag)?;
    if has_authored_reference_data(container) {
        return Some(collect_num_cache_positional(ser_node, child_tag));
    }
    reference_formula(container).and_then(|formula| references.resolve_numbers(&formula))
}

/// Formula identity for a source that genuinely needs the host resolver.
/// Authored caches/literals deliberately return `None`: even when their `<f>`
/// text matches another series, their authored point data remains authoritative.
fn external_reference_formula(ser_node: Node<'_, '_>, child_tag: &str) -> Option<String> {
    let container = ser_node
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == child_tag)?;
    (!has_authored_reference_data(container))
        .then(|| reference_formula(container))
        .flatten()
}

/// Parse the shared body of a legacy DrawingML chart (`c:` namespace) into a
/// [`ChartModel`]. `chart_root` is the `<c:chartSpace>` root element; the
/// crate-specific `<a:solidFill>` resolution (pptx theme map vs. xlsx theme
/// slice) arrives as `color_resolver`, so this one function owns the entire
/// chart-structure parse (series, categories, axes, legend, titles, dLbls,
/// borders, plus every shared `extract_*` probe) that the per-format adapters
/// delegate to. The graphic-frame geometry (`x`/`y`/`w`/`h`) stays in each
/// crate's wrapper.
///
/// The core structure (series/axis/legend/title walk, the overall control
/// flow) was moved from the pptx `parse_legacy_chart` body, with only the
/// mechanical edits listed below: `parse_color_node(fill, theme)` became
/// `color_resolver.resolve_solid_fill(fill)`, the `ooxml_common::chart::`
/// self-prefix was dropped, the local `PptxColorResolver` was replaced by the
/// passed `color_resolver`, and the `ChartElement` frame wrapper was replaced
/// by a bare `ChartModel` return. The richer series/axis extractors this
/// function calls (markers, per-point overrides, data labels, error bars,
/// positional num/str caches, radar style, axis crosses, manual layout, etc.)
/// were moved here from the xlsx parser, which had the more complete
/// implementation of each.
pub fn parse_chart_part(
    chart_root: Node,
    color_resolver: &dyn ColorResolver,
) -> Option<ChartModel> {
    let mut references = EmptyChartReferenceResolver;
    parse_chart_part_with_references(chart_root, color_resolver, &mut references)
}

/// Parse a legacy chart with an optional package-supplied formula resolver.
/// Authored caches and literals always win; the resolver is called only for a
/// formula-only reference. This keeps series identity and field dispatch in the
/// shared DrawingML parser while leaving workbook lookup to the host package.
pub fn parse_chart_part_with_references(
    chart_root: Node,
    color_resolver: &dyn ColorResolver,
    references: &mut dyn ChartReferenceResolver,
) -> Option<ChartModel> {
    let root = chart_root;

    // Determine chart type by finding the first recognized chart element
    let find_chart = |name: &str| {
        root.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == name)
    };

    // ECMA-376 3D chart types (§21.2.2.15 bar3DChart, §21.2.2.96 line3DChart,
    // §21.2.2.4 area3DChart, §21.2.2.140 pie3DChart) are FLATTENED to their 2D
    // equivalents: the child data structure (`<c:ser>`/`<c:cat>`/`<c:val>`/
    // grouping/`<c:dLbls>`) is identical to the 2D form, so a 3D chart is drawn
    // as the corresponding 2D chart. The 3D-only elements (`<c:view3D>`
    // §21.2.2.228, the 3D chart-space surfaces `<c:floor>` §21.2.2.69 /
    // `<c:sideWall>` §21.2.2.191 / `<c:backWall>` §21.2.2.11 (all `CT_Surface`),
    // `<a:scene3d>`/`<a:sp3d>` shape 3D and `<c:gapDepth>` §21.2.2.74) are
    // ignored. This 2D-flattening is the established strategy of web chart
    // engines (Google Slides, Keynote) and was approved in the CH13 plan; a
    // faithful isometric 3D projection is out of scope.
    // `surfaceChart`/`surface3DChart` are NOT flattened (they have no 2D
    // analogue) and stay "unknown".
    let read_grouping = |group: &Node, default: &str| -> String {
        group
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "grouping")
            .and_then(|n| attr(&n, "val"))
            .unwrap_or_else(|| default.into())
    };
    let chart_type = if let Some(bc) = find_chart("barChart").or_else(|| find_chart("bar3DChart")) {
        // §21.2.2.17 barDir + §21.2.2.77 grouping (Bar Grouping). bar3DChart shares both
        // (its extra `<c:gapDepth>` is ignored). `clustered` is the 2D default;
        // `standard` (the bar3DChart default) folds to clustered as well since
        // `canonical_chart_type` treats any non-stacked grouping as clustered.
        let grouping = read_grouping(&bc, "clustered");
        let bar_dir = bc
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "barDir")
            .and_then(|n| attr(&n, "val"))
            .unwrap_or_else(|| "col".into());
        canonical_chart_type("bar", &bar_dir, &grouping)
    } else if let Some(lc) = find_chart("lineChart").or_else(|| find_chart("line3DChart")) {
        let grouping = read_grouping(&lc, "standard");
        canonical_chart_type("line", "col", &grouping)
    } else if find_chart("pieChart").is_some() || find_chart("pie3DChart").is_some() {
        "pie".to_string()
    } else if find_chart("ofPieChart").is_some() {
        // §21.2.2.126 ofPieChart (pie-of-pie / bar-of-pie). DECISION: draw the
        // whole series as ONE plain pie (main-pie-only fallback) rather than
        // splitting the tail data points into the secondary pie/bar. The
        // split is governed by `<c:splitType>` (§21.2.2.196: `auto` / `cust` /
        // `percent` / `pos` / `val`), `<c:splitPos>` (§21.2.2.195) and
        // `<c:custSplit>`, plus `<c:secondPieSize>` and `<c:serLines>` for the
        // connector geometry — all of which need a validated fixture to lay out
        // correctly. Without one, splitting risks assigning the wrong points to
        // the secondary plot; a single combined pie is a lossless, always-correct
        // representation of the same data (every point is shown as a slice). The
        // secondary-plot elements are ignored (not errors). A `bar` `ofPieType`
        // is likewise flattened to a pie — the bar-of-pie's detail column is the
        // same subset-of-points concern. `<c:varyColors>` still cycles the accent
        // palette across the slices (handled by the shared pie color path below).
        "pie".to_string()
    } else if find_chart("doughnutChart").is_some() {
        "doughnut".to_string()
    } else if let Some(ac) = find_chart("areaChart").or_else(|| find_chart("area3DChart")) {
        let grouping = read_grouping(&ac, "standard");
        canonical_chart_type("area", "col", &grouping)
    } else if find_chart("scatterChart").is_some() {
        "scatter".to_string()
    } else if find_chart("bubbleChart").is_some() {
        "bubble".to_string()
    } else if find_chart("radarChart").is_some() {
        "radar".to_string()
    } else if find_chart("stockChart").is_some() {
        // §21.2.2.198 stockChart — high/low/close[/open] series drawn as
        // per-category hi-lo lines + close ticks by the core stock renderer.
        "stock".to_string()
    } else {
        "unknown".to_string()
    };

    // §21.2.2.198 stockChart decoration: `<c:hiLowLines>` (§21.2.2.80) and
    // `<c:upDownBars>` (§21.2.2.218). Both are direct children of `<c:stockChart>`.
    // The hi-lo line spans each category's low↔high; its `<c:spPr><a:ln>` fill is
    // resolved so the renderer strokes it in the file's color (else a gray
    // default). up/down bars are recognized but not yet drawn (follow-up). Every
    // field stays `None` for non-stock charts (byte-stable wire).
    let (stock_hi_low_lines, stock_hi_low_line_color, stock_up_down_bars) = if chart_type == "stock"
    {
        let stock = find_chart("stockChart");
        let hi_low = stock.and_then(|s| child(s, "hiLowLines"));
        let hi_low_color = hi_low
            .and_then(|hl| child(hl, "spPr"))
            .and_then(|sp| child(sp, "ln"))
            .and_then(|ln| {
                ln.children()
                    .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
            })
            .and_then(|fill| color_resolver.resolve_solid_fill(fill));
        let up_down = stock.and_then(|s| child(s, "upDownBars")).is_some();
        (
            Some(hi_low.is_some()),
            hi_low_color,
            if up_down { Some(true) } else { None },
        )
    } else {
        (None, None, None)
    };

    // Title text. The CHART title is the direct-child `<c:title>` of `<c:chart>`
    // (ECMA-376 §21.2.2.6) — NOT any `<c:title>` descendant. A `descendants()`
    // search would pick up the first AXIS title (which lives inside `<c:plotArea>`
    // → `<c:valAx>`/`<c:catAx>`) on a chart that has axis titles but no chart
    // title, wrongly promoting it to the chart title. Scope strictly to the
    // `<c:chart>` element's own `<c:title>` child.
    let chart_node = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "chart");
    let title_node_opt = chart_node.and_then(|c| child(c, "title"));
    let mut title = title_node_opt.and_then(|title_node| {
        let texts: Vec<String> = title_node
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "t")
            .filter_map(|n| n.text().map(|t| t.to_string()))
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join(""))
        }
    });
    // Title font size in hundredths of a point — taken from the first
    // defRPr@sz or rPr@sz we find inside the title. ECMA-376 uses hpt for size.
    let title_font_size_hpt = title_node_opt.and_then(|t| {
        t.descendants().find_map(|n| {
            if !n.is_element() {
                return None;
            }
            let tag = n.tag_name().name();
            if tag != "defRPr" && tag != "rPr" {
                return None;
            }
            attr(&n, "sz").and_then(|v| v.parse::<i32>().ok())
        })
    });
    // Title font color — resolved via the `ColorResolver` so a `<a:schemeClr>`
    // (e.g. `tx2` → the theme dark-2 slot) resolves in addition to a literal
    // `<a:srgbClr>`. `extract_chart_title_color` scopes to the direct-child
    // `<c:title>` of the node it's given, so pass `title_node_opt`'s parent (the
    // element that holds `<c:title>`). Previously hardcoded `None` (the srgb was
    // never threaded into the wire model); resolving it fixes titles that use a
    // theme scheme color, which Office decks commonly do.
    let title_font_color = title_node_opt
        .and_then(|t| t.parent())
        .and_then(|parent| extract_chart_title_color(parent, color_resolver));

    // val axis max / min and visibility — shared helpers in ooxml-common
    // so xlsx & pptx stay in sync (`<c:scaling><c:min|max val>` §21.2.2.160
    // scaling and `<c:delete val>` ECMA-376 §21.2.2.40 delete).
    // Combo charts (bar + line) declare TWO `<c:valAx>`: a PRIMARY (axPos="l",
    // `<c:crosses val="autoZero">`) and a SECONDARY (axPos="r",
    // `<c:crosses val="max">`). Collect both so series can be mapped to the
    // right scale and the right-hand axis drawn. The primary axis keeps driving
    // every existing axis read below; only the secondary is new.
    let val_ax_nodes: Vec<roxmltree::Node> = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "valAx")
        .collect();
    let ax_pos = |n: &roxmltree::Node| -> Option<String> {
        n.children()
            .find(|c| c.is_element() && c.tag_name().name() == "axPos")
            .and_then(|c| attr(&c, "val"))
    };
    let ax_id_of = |n: &roxmltree::Node| -> Option<String> {
        n.children()
            .find(|c| c.is_element() && c.tag_name().name() == "axId")
            .and_then(|c| attr(&c, "val"))
    };
    // The category axis is normally `<c:catAx>` or, for a date/time-series X
    // axis, `<c:dateAx>` (§21.2.2.39) — same child grammar, so every cat-axis
    // read below treats them identically. A SCATTER / BUBBLE chart has NO catAx:
    // it declares two `<c:valAx>` and the *horizontal* one (`axPos` b/t) plays
    // the category-axis role, while the *vertical* one (`axPos` l/r) is the
    // value axis. Detect this and route the horizontal valAx into `cat_ax` so
    // its tick-label / line / format / crossing properties land in the cat-axis
    // fields, exactly as Excel presents them.
    let real_cat_ax = root
        .descendants()
        .find(|n| n.is_element() && matches!(n.tag_name().name(), "catAx" | "dateAx"));
    let is_scatter_axes = real_cat_ax.is_none() && val_ax_nodes.len() >= 2;
    let scatter_x_val_ax = if is_scatter_axes {
        val_ax_nodes
            .iter()
            .find(|n| matches!(ax_pos(n).as_deref(), Some("b") | Some("t")))
            .copied()
    } else {
        None
    };
    let cat_ax = real_cat_ax.or(scatter_x_val_ax);

    // Primary value axis. Normally the first value axis that isn't on the right.
    // For scatter it's the VERTICAL (l/r) axis — never the horizontal one, which
    // is the category axis above. Secondary (combo charts) = a right-edge valAx.
    let val_ax = if is_scatter_axes {
        val_ax_nodes
            .iter()
            .find(|n| matches!(ax_pos(n).as_deref(), Some("l") | Some("r")))
            .or_else(|| val_ax_nodes.first())
            .copied()
    } else {
        val_ax_nodes
            .iter()
            .find(|n| ax_pos(n).as_deref() != Some("r"))
            .or_else(|| val_ax_nodes.first())
            .copied()
    };
    let secondary_val_ax = if !is_scatter_axes && val_ax_nodes.len() >= 2 {
        val_ax_nodes
            .iter()
            .find(|n| ax_pos(n).as_deref() == Some("r"))
            .copied()
    } else {
        None
    };
    let secondary_ax_id = secondary_val_ax.as_ref().and_then(ax_id_of);
    let (val_min, val_max) = val_ax.map(extract_axis_min_max).unwrap_or((None, None));
    let val_axis_hidden = val_ax.map(axis_is_deleted).unwrap_or(false);
    let cat_axis_hidden = cat_ax.map(axis_is_deleted).unwrap_or(false);

    // Series
    let plot_area = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "plotArea")?;

    // Plot area background: <c:plotArea><c:spPr><a:solidFill>
    let plot_area_bg = plot_area
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "spPr")
        .and_then(|sp| {
            sp.children()
                .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
        })
        .and_then(|fill| color_resolver.resolve_solid_fill(fill));

    let ser_nodes: Vec<_> = plot_area
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ser")
        .collect();

    if ser_nodes.is_empty() {
        return None;
    }

    // Chart-level category labels from the first series, using the POSITIONAL
    // collector so a sparse cache (labels that start at `idx=1`, or a hole in
    // the middle) keeps its true length and per-index alignment. The old
    // document-order collector collapsed such caches, truncating every series
    // and mis-registering data (issue: cat-less line 11→1, idx=1 radar 11→10).
    // Scatter/bubble carry their X labels in `<c:xVal>` (there is no `<c:cat>`),
    // so read that instead — the shared category list mirrors the first series'
    // X data, matching how Excel drives the horizontal-axis labels.
    let chart_uses_xval = chart_type == "scatter" || chart_type == "bubble";
    let category_tag = if chart_uses_xval { "xVal" } else { "cat" };
    let shared_category_formula = external_reference_formula(ser_nodes[0], category_tag);
    let categories: Vec<String> =
        collect_string_source(ser_nodes[0], category_tag, references).unwrap_or_default();

    let is_scatter_like = chart_type == "scatter" || chart_type == "bubble";

    // Map a chart-group element name to the per-series `seriesType` string the
    // renderer dispatches on (mixed bar+line charts key line vs. non-line off
    // this field). Mirrors the xlsx `type_map`; `bubbleChart` folds to
    // `scatter` like everything else.
    // 3D groups fold to the same series type as their 2D equivalent (they are
    // flattened above); `stockChart`/`ofPieChart` have no combo-mixing role so
    // map to a plain type too.
    let group_series_type = |group_name: &str| -> Option<String> {
        match group_name {
            "barChart" | "bar3DChart" => Some("bar"),
            "lineChart" | "line3DChart" => Some("line"),
            "areaChart" | "area3DChart" => Some("area"),
            "pieChart" | "pie3DChart" | "ofPieChart" => Some("pie"),
            "doughnutChart" => Some("doughnut"),
            "radarChart" => Some("radar"),
            "scatterChart" | "bubbleChart" => Some("scatter"),
            "stockChart" => Some("stock"),
            _ => None,
        }
        .map(|s| s.to_string())
    };

    // Total series in the plot. §21.2.2.227 varyColors on a NON-pie chart
    // varies each data point by color only for a single-series plot (Office
    // keeps per-series colors when several series share the axes); captured
    // here so the per-series closure can gate the accent fill on it.
    let series_count = ser_nodes.len();

    let series: Vec<ChartSeries> = ser_nodes
        .iter()
        .enumerate()
        .map(|(series_position, ser)| {
            // Each `<c:ser>` is a direct child of its chart-group element
            // (`<c:barChart>`/`<c:lineChart>`/…). `series_type` carries that
            // group's type so the renderer can draw line-group series as a line
            // over the columns in a combo chart (ECMA-376 §21.2.2.97); we also
            // flag series whose group references the secondary value axis so they
            // plot against the right-hand scale.
            let group = ser.parent();
            let series_type = group
                .map(|p| p.tag_name().name())
                .and_then(group_series_type);
            let use_secondary_axis = match (group, secondary_ax_id.as_deref()) {
                (Some(g), Some(sec)) => g
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "axId")
                    .any(|c| attr(&c, "val").as_deref() == Some(sec)),
                _ => false,
            };

            // Series name from <c:tx>  (can be strRef/strCache, strLit, or a bare <c:v>)
            let name = collect_string_source(*ser, "tx", references)
                .and_then(|values| values.into_iter().next())
                .or_else(|| {
                    ser.children()
                        .find(|n| n.is_element() && n.tag_name().name() == "tx")
                        .and_then(|tx| {
                            tx.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "v")
                                .and_then(|v| v.text().map(|t| t.to_string()))
                        })
                })
                .unwrap_or_default();

            // `<c:idx val>` (ECMA-376 §21.2.2.84) — the canonical series index
            // Office uses for default-palette color selection. `<c:order>` is a
            // separate display-order field and must NOT drive coloring.
            let series_idx: usize = child(*ser, "idx")
                .and_then(|n| n.attribute("val"))
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);

            // Per-series category labels. Scatter/bubble put numeric X data in
            // `<c:xVal>` (ECMA-376 §21.2.2.43); every other type reads the
            // series' own `<c:cat>`. The first series is already represented by
            // chart-level `categories`; repeated live formulas also use that
            // canonical vector instead of resolving and retaining duplicate
            // copies. `ChartSeries.categories = None` explicitly means to fall
            // back to `ChartModel.categories` (the shared TS contract). Authored
            // caches/literals and genuinely distinct formulas remain per-series.
            let series_categories: Option<Vec<String>> = {
                let own_formula = external_reference_formula(*ser, category_tag);
                let shares_chart_categories = series_position == 0
                    || (own_formula.is_some()
                        && own_formula.as_deref() == shared_category_formula.as_deref());
                if shares_chart_categories {
                    None
                } else {
                    let has_own_source = ser
                        .children()
                        .any(|node| node.is_element() && node.tag_name().name() == category_tag);
                    match collect_string_source(*ser, category_tag, references) {
                        Some(own) if is_scatter_like || !own.is_empty() => Some(own),
                        // A distinct scatter/bubble X source that cannot be
                        // resolved must not inherit the first series' X values.
                        // `Some([])` explicitly suppresses that fallback.
                        None if is_scatter_like && has_own_source => Some(Vec::new()),
                        _ => None,
                    }
                }
            };

            // Y values (scatter/bubble → `<c:yVal>`, else `<c:val>`), collected
            // POSITIONALLY. The series' own cache `<c:ptCount>` sizes the vector
            // and each `<c:pt idx>` lands at its index — the length no longer
            // rides on the category count, so a value series with more points
            // than there are cat labels (cat-less line, sparse radar) keeps all
            // of its data.
            let val_tag = if is_scatter_like { "yVal" } else { "val" };
            let values: Vec<Option<f64>> =
                collect_number_source(*ser, val_tag, references).unwrap_or_default();
            let series_pt_count = values.len().max(1);
            // Value-cache node for the series-value number format (`<c:formatCode>`).
            let val_cache = ser
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == val_tag)
                .and_then(|v| {
                    v.descendants().find(|n| {
                        n.is_element()
                            && (n.tag_name().name() == "numCache"
                                || n.tag_name().name() == "numLit")
                    })
                });

            // Bubble per-point sizes (ECMA-376 §21.2.2.4 `<c:bubbleSize>`).
            // Only meaningful for bubble charts; scatter / others ignore.
            let bubble_sizes: Option<Vec<Option<f64>>> = if chart_type == "bubble" {
                collect_number_source(*ser, "bubbleSize", references).map(|mut sizes| {
                    sizes.resize(sizes.len().max(series_pt_count), None);
                    sizes
                })
            } else {
                None
            };

            // Series color from spPr > solidFill (bar/area/pie) or spPr > ln >
            // solidFill (line/scatter/radar carry their color on the stroke).
            // When neither is present, fall back to the theme accent for this
            // series index (`theme.accent[(idx % 6) + 1]`) via the resolver, so
            // the default Office palette renders without theme access. Resolvers
            // whose renderer owns its own palette (pptx) return `None` here and
            // keep `color` unset.
            let color = ser
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "spPr")
                .and_then(|sp| {
                    if sp
                        .children()
                        .any(|n| n.is_element() && n.tag_name().name() == "solidFill")
                    {
                        color_resolver.resolve_shape_fill(sp)
                    } else {
                        sp.children()
                            .find(|n| n.is_element() && n.tag_name().name() == "ln")
                            .and_then(|ln| color_resolver.resolve_shape_fill(ln))
                    }
                })
                .or_else(|| color_resolver.resolve_series_accent(series_idx));

            // §21.2.2.198 series-level `<c:spPr><a:ln>`: an explicit `<a:noFill/>`
            // turns the connecting line OFF, overriding the chart-group
            // `<c:scatterStyle>` (§21.2.2.42) / line-group default. Excel draws a
            // markers-only scatter when the series line is `<a:noFill/>` even
            // though `<c:scatterStyle val="lineMarker">` sets the group default to
            // connect points. Only the noFill flag matters here (color/width ride
            // on `color` above), so discard the other two tuple fields.
            let (_, _, line_no_fill) = extract_sp_pr_ln_style(*ser, color_resolver);

            // Per-data-point colors from <c:dPt> (§21.2.2.52; important for
            // pie charts). The point index is the CHILD element `<c:idx val>`
            // (ECMA-376 §21.2.2.84, CT_UnsignedInt), not an attribute on
            // `<c:dPt>` — the old `attr(dpt, "idx")` always returned None, so
            // every slice fell
            // back to the series colour. The fill is `<c:spPr><a:solidFill>`;
            // restrict to spPr's direct child so a border `<a:ln><a:solidFill>`
            // can't be mistaken for the slice fill.
            let data_point_colors: Vec<Option<String>> = (0..series_pt_count)
                .map(|i| {
                    ser.children()
                        .filter(|n| n.is_element() && n.tag_name().name() == "dPt")
                        .find(|dpt| {
                            dpt.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "idx")
                                .and_then(|n| attr(&n, "val"))
                                .and_then(|v| v.parse::<usize>().ok())
                                == Some(i)
                        })
                        .and_then(|dpt| {
                            dpt.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "spPr")
                        })
                        .and_then(|sp| {
                            sp.children()
                                .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
                        })
                        .and_then(|fill| color_resolver.resolve_solid_fill(fill))
                })
                .collect();

            // §21.2.2.227 `<c:varyColors>`: a pie/doughnut varies each DATA POINT
            // by the theme accent palette (`accent[(i % 6) + 1]`), rather than
            // giving the whole series one fill. It defaults to ON for the pie
            // family, so an absent element still cycles the accents. When on, fill
            // every slice that lacks an explicit `<c:dPt>` fill from the resolver's
            // accent for that point index — this is the same palette Office draws,
            // so a docx/xlsx pie matches Word/Excel instead of falling back to the
            // renderer's built-in default colors. Resolvers that own their own
            // palette (pptx `resolve_series_accent` → None) contribute nothing here
            // and stay byte-stable. Non-pie families are unaffected: only the pie
            // renderer consumes `data_point_colors`, and a multi-series pie (rare)
            // still varies by point within series[0].
            let is_pie_family = chart_type == "pie" || chart_type == "doughnut";
            let is_bar_family = chart_type.contains("Bar");
            // §21.2.2.227 `<c:varyColors>` — CT_Boolean (bare element ⇒ true).
            // "Vary colors by point" is the effective default for the pie family
            // AND for a SINGLE-series bar/column chart (Word/Excel/PowerPoint
            // draw a lone series' bars in the rotating theme palette and keep an
            // explicit `<c:varyColors val="0"/>` when the user forces one color
            // — verified against the sample-17/18 decks, whose single-series
            // columns render four accent-colored bars with the element ABSENT).
            // A multi-series plot keeps per-series colors, so it never varies by
            // point even with `val="1"`. When ON, each data point takes the
            // accent for its POINT index (i); `dPt` fills already sit in
            // `data_point_colors` and are never overwritten (explicit per-point
            // color keeps priority, §21.2.2.52).
            let qualifies_for_vary = is_pie_family || (is_bar_family && series_count == 1);
            let vary = group
                .and_then(|g| bool_child(g, "varyColors"))
                .unwrap_or(true);
            let vary_by_point = qualifies_for_vary && vary;
            let mut data_point_colors = data_point_colors;
            if vary_by_point {
                for (i, slot) in data_point_colors.iter_mut().enumerate() {
                    if slot.is_none() {
                        *slot = color_resolver.resolve_series_accent(i);
                    }
                }
            }
            let has_dpt_colors = data_point_colors.iter().any(|c| c.is_some());

            // Per-point `<c:dPt>` overrides (§21.2.2.39): marker (symbol/size/
            // fill/line) and `<c:explosion>` (pie/doughnut pull-out). Plain
            // per-point FILL flows through `data_point_colors` above (the pie
            // model the pptx path established), so we only emit an override when
            // it carries a marker or explosion — a color-only dPt yields no
            // override and stays clean on the wire. This makes the shared parser
            // populate xlsx's marker overrides (e.g. sample-26 scatter) without
            // double-representing pie slice fills.
            let data_point_overrides: Vec<ChartDataPointOverride> =
                parse_data_point_overrides(*ser, color_resolver)
                    .into_iter()
                    .filter(|o| {
                        o.marker_symbol.is_some()
                            || o.marker_size.is_some()
                            || o.marker_fill.is_some()
                            || o.marker_line.is_some()
                            || o.explosion.is_some()
                    })
                    .collect();

            // Series value number format from `<c:val>…<c:numCache><c:formatCode>`.
            // Used for data labels when `<c:dLbls>` carries no explicit `<c:numFmt>`
            // (ECMA-376 §21.2.2.121). "General" means "no format" → drop it so the
            // renderer's default integer/decimal formatter takes over.
            let val_format_code = val_cache
                .and_then(|cache| {
                    cache
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "formatCode")
                        .and_then(|fc| fc.text().map(|t| t.to_string()))
                })
                .filter(|s| !s.is_empty() && s != "General");

            // Series-level data-label text colour from `<c:dLbls><c:txPr>…solidFill`.
            // Scoped to this `<c:ser>` (not chart-root) so stacked-bar segments keep
            // their independent label colours (white on dark fill, black on light).
            let label_color = ser
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "dLbls")
                .and_then(|dlbls| {
                    dlbls
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "txPr")
                })
                .and_then(|txpr| {
                    txpr.descendants()
                        .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
                })
                .and_then(|fill| color_resolver.resolve_solid_fill(fill));

            // Marker styling (ECMA-376 §21.2.2.32/§21.2.2.34). A per-series
            // `<c:marker>` gives the symbol/size/fill/line; when the symbol is
            // absent the chart-type-level `<c:lineChart><c:marker val>` default
            // (§21.2.2.33) governs visibility. Scatter defaults to visible
            // markers even without an explicit flag.
            // §21.2.2.33 chart-type-level `<c:lineChart><c:marker>` — CT_Boolean,
            // so a bare `<c:marker/>` enables markers (val default true); absent
            // ⇒ false (line series draw no markers unless opted in).
            let chart_marker_default = group.and_then(|g| bool_child(g, "marker")).unwrap_or(false);
            let marker_node = child(*ser, "marker");
            let (marker_symbol, marker_size, marker_fill, marker_line) =
                parse_marker_block(marker_node, color_resolver);
            let show_marker = match (&marker_symbol, is_scatter_like) {
                (Some(sym), _) => sym != "none",
                (None, true) => true,
                _ => chart_marker_default,
            };

            // Series-level `<c:dLbls>` defaults + per-idx custom labels, and
            // error bars (§21.2.2.20, resolved to absolute plus/minus arrays).
            let dlbl_range_cache = collect_dlbl_range_cache(*ser);
            let (series_data_labels, data_label_overrides) =
                parse_series_data_labels(*ser, color_resolver, &dlbl_range_cache);
            let err_bars = parse_error_bars(*ser, &values, color_resolver);

            ChartSeries {
                name,
                values,
                color,
                data_point_colors: if has_dpt_colors {
                    Some(data_point_colors)
                } else {
                    None
                },
                // Legacy `<c:chart>` per-point label colors are extracted via
                // `<c:dLbls><c:dLbl idx>` — not yet wired here; chartEx is the only
                // path that needs it for sample-2's waterfall.
                data_label_colors: None,
                categories: series_categories,
                bubble_sizes,
                val_format_code,
                label_color,
                series_type,
                // Shared `ChartSeries.use_secondary_axis` is `Option<bool>`; the
                // legacy default (false) is expressed as `None` so it drops off
                // the wire exactly as the old `skip_serializing_if = "Not::not"`
                // did.
                use_secondary_axis: if use_secondary_axis { Some(true) } else { None },
                // Marker styling / per-series data labels / error bars, now
                // populated by the shared extractors so both pptx and xlsx get
                // markers, dLbls and errBars from the one parse path.
                show_marker: Some(show_marker),
                marker_symbol,
                marker_size,
                marker_fill,
                marker_line,
                data_point_overrides: if data_point_overrides.is_empty() {
                    None
                } else {
                    Some(data_point_overrides)
                },
                data_label_overrides: if data_label_overrides.is_empty() {
                    None
                } else {
                    Some(data_label_overrides)
                },
                series_data_labels,
                err_bars: if err_bars.is_empty() {
                    None
                } else {
                    Some(err_bars)
                },
                // `<c:ser><c:smooth>` (§21.2.2.194) — line/area spline flag.
                // Shared with the xlsx parser via ooxml-common so both honor the
                // CT_Boolean implied-true semantics.
                smooth: extract_series_smooth(*ser),
                // `<c:ser><c:trendline>` (§21.2.2.211) — regression lines. Shared
                // extractor; the line color resolves through the pptx theme.
                trend_lines: extract_series_trendlines(*ser, color_resolver),
                // §21.2.2.198 `<c:spPr><a:ln><a:noFill/>` — series connecting line
                // explicitly off. Only serialized when set, so byte-stable for
                // series that carry no line-off (the common case).
                line_hidden: if line_no_fill { Some(true) } else { None },
            }
        })
        .collect();

    // Auto-title (ECMA-376 §21.2.2.7 `<c:autoTitleDeleted>`). When the chart has
    // no explicit title text but auto-titling is enabled, Word synthesizes a
    // title and shows it in the chart frame. §21.2.2.7 says the element only
    // governs WHETHER an auto title may be shown ("val=0/false ⇒ the chart title
    // SHALL be shown" when otherwise absent; "val=1/true ⇒ it SHALL NOT be
    // shown"); the spec leaves the auto title's TEXT implementation-defined.
    // Word's observed rule — the ground truth here is sample-25.docx / .pdf,
    // whose `<c:title>` carries a `<c:txPr>` (fonts, `cap="all"`) but NO `<c:tx>`
    // text, `<c:autoTitleDeleted val="0"/>`, and exactly one series named
    // "Production in 2017" — is:
    //   * exactly ONE series  → the auto title is that single series' name
    //   * two or more series   → NO auto title (a lone series name would be
    //                            misleading, so Word shows none)
    // We adopt only the single-series case; multi-series charts stay untitled,
    // matching Word. The title's `<a:defRPr cap="all">` would uppercase the
    // rendered glyphs ("PRODUCTION IN 2017"); chart-title `cap` is a display
    // transform we do not yet apply, so the model carries the series name
    // VERBATIM ("Production in 2017"). Making the title APPEAR is the goal; the
    // caps transform is a separate, tracked rendering-layer limitation.
    if title.is_none() {
        // §21.2.2.7 `<c:autoTitleDeleted>` — CT_Boolean, so a bare element ⇒ true
        // (the auto title is deleted, suppressing the single-series fallback
        // title); absent ⇒ false (the auto title may still be shown).
        let auto_title_deleted = chart_node
            .and_then(|c| bool_child(c, "autoTitleDeleted"))
            .unwrap_or(false);
        if !auto_title_deleted && series.len() == 1 {
            let ser_name = series[0].name.trim();
            if !ser_name.is_empty() {
                title = Some(ser_name.to_string());
            }
        }
    }

    // Data labels are on when `<c:dLbls>` enables `<c:showVal>` OR
    // `<c:showPercent>` (ECMA-376 §21.2.2.189 / §21.2.2.187) — at chart level
    // or in any series. Pie/doughnut decks commonly use showPercent only (e.g.
    // sample-14 slide-7's "54%/27%/…" slice labels); the renderer draws the
    // slice percentage for pie/doughnut and the raw value for bar/line.
    let show_data_labels = root
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "dLbls")
        .any(|d_lbls| {
            // `<c:showVal>` / `<c:showPercent>` are CT_Boolean: a present element
            // is ON unless `val` explicitly disables it (bare element ⇒ true), so
            // a bare `<c:showVal/>` enables data labels while `val="0"` does not.
            ["showVal", "showPercent"]
                .iter()
                .any(|name| bool_child(d_lbls, name).unwrap_or(false))
        });

    // Outer chartSpace spPr: we want the child of chartSpace (not plotArea).
    // When the `<c:spPr>` is PRESENT we honor whatever it resolves to (a
    // `<a:solidFill>` hex or, for `<a:noFill>` / an spPr with no fill child,
    // `None`). When it is ABSENT the file relies on the host default chart area
    // — Excel's opaque white vs. PowerPoint's transparent composite — supplied
    // by the resolver via `default_chart_bg`.
    let chart_sp_pr = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "spPr");
    let chart_bg = match chart_sp_pr {
        Some(sp) => sp
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "solidFill")
            .and_then(|fill| color_resolver.resolve_solid_fill(fill)),
        None => color_resolver.default_chart_bg(),
    };

    // <c:legend> + <c:legendPos val> — shared helper.
    let (show_legend, legend_pos) = extract_legend(root);

    // ECMA-376 §21.2.2.35: `<c:crossBetween>` lives on the VALUE axis (not cat),
    // and describes whether value gridlines land between or on category ticks.
    // Default is "between" (categories inset by half a step each side).
    let cat_axis_cross_between = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "valAx")
        .and_then(|ax| {
            ax.children()
                .find(|n| n.is_element() && n.tag_name().name() == "crossBetween")
        })
        .and_then(|n| attr(&n, "val"))
        .unwrap_or_else(|| "between".to_string());

    // Major tick marks (ECMA-376 §21.2.2.49 ST_TickMark, default "cross").
    // Schema default is `out` (ST_TickMark §21.2.3.48), shared with xlsx via
    // the ooxml-common helper so the two parsers don't diverge on the default.
    let read_major_tick_mark = |ax: Option<roxmltree::Node>| -> String {
        ax.map(|n| extract_axis_tick_mark_or_default(n, "majorTickMark"))
            .unwrap_or_else(|| "out".to_string())
    };
    let val_axis_major_tick_mark = read_major_tick_mark(val_ax);
    let cat_axis_major_tick_mark = read_major_tick_mark(cat_ax);

    // Axis tick-label font size from `<c:txPr>` (in OOXML hundredths of a point).
    let cat_axis_font_size_hpt = cat_ax.and_then(extract_axis_tick_label_size);
    let val_axis_font_size_hpt = val_ax.and_then(extract_axis_tick_label_size);

    // Data-label font size — first `<c:dLbls><c:txPr>` defRPr/rPr@sz we find.
    let data_label_font_size_hpt = extract_data_label_font_size(root);

    // Bar gap / overlap, dLblPos and numFmt — all shared helpers so any new
    // chart property added to the xlsx side stays applied to pptx without
    // a manual port (the slide-7 / sample-2 issue this PR avoids).
    let (bar_gap_width, bar_overlap) = extract_bar_gap_overlap(root);
    let data_label_position = extract_data_label_position(root);
    let data_label_format_code = extract_data_label_format_code(root);

    // Data-label font color uses the shared helper too — pptx supplies a
    // ColorResolver wrapper around `parse_color_node` so the
    // ECMA-376 §21.2.2.16 dLbls > txPr > solidFill walk lives in one place.
    let data_label_font_color = extract_data_label_font_color(root, color_resolver);

    // Axis tick-label text color + axis-line style (color / width / noFill).
    // ECMA-376 §21.2.2.* — `<c:catAx|valAx><c:txPr>…<a:solidFill>` colors the
    // tick labels and `<c:spPr><a:ln>` styles the axis rule. Shared helpers so
    // the gray "2025年3月期" category labels and the light-gray category-axis
    // line in sample-2 slide-16's horizontal bar chart resolve the same way.
    let cat_axis_font_color = cat_ax.and_then(|n| extract_axis_tick_label_color(n, color_resolver));
    let val_axis_font_color = val_ax.and_then(|n| extract_axis_tick_label_color(n, color_resolver));
    let (cat_axis_line_color, cat_axis_line_width_emu, cat_axis_line_hidden) = cat_ax
        .map(|n| extract_axis_line_style(n, color_resolver))
        .unwrap_or((None, None, false));
    let (val_axis_line_color, val_axis_line_width_emu, val_axis_line_hidden) = val_ax
        .map(|n| extract_axis_line_style(n, color_resolver))
        .unwrap_or((None, None, false));

    // `<c:valAx><c:numFmt formatCode>` — value-axis tick label number format.
    let val_axis_format_code = val_ax.and_then(extract_axis_format_code);
    // `<c:catAx|dateAx><c:numFmt formatCode>` — category-axis number format. For
    // a `<c:dateAx>` this is the date serial format code (e.g. "m/d/yyyy") the TS
    // side needs to format category labels. Reaches parity with the xlsx parser,
    // which already wires this field (pptx previously hardcoded it to None).
    let cat_axis_format_code = cat_ax.and_then(extract_axis_format_code);

    // Secondary value axis (combo charts) — parse the right-hand `<c:valAx>`
    // into a self-contained spec using the same shared helpers as the primary
    // axis. None for the common single value-axis case.
    let secondary_val_axis = secondary_val_ax.map(|ax| {
        let (min, max) = extract_axis_min_max(ax);
        let (t, title_size, title_bold, title_color) =
            extract_axis_title_with_props_resolved(ax, color_resolver);
        let (line_color, line_width_emu, line_hidden) = extract_axis_line_style(ax, color_resolver);
        SecondaryValueAxis {
            min,
            max,
            title: t,
            hidden: axis_is_deleted(ax),
            format_code: extract_axis_format_code(ax),
            font_color: extract_axis_tick_label_color(ax, color_resolver),
            font_size_hpt: extract_axis_tick_label_size(ax),
            line_color,
            line_width_emu,
            line_hidden,
            major_tick_mark: extract_axis_tick_mark_or_default(ax, "majorTickMark"),
            major_unit: extract_axis_major_unit(ax),
            title_font_size_hpt: title_size,
            title_font_bold: title_bold,
            title_font_color: title_color,
        }
    });

    // `<c:plotArea><c:layout><c:manualLayout>` — explicit plot-area rectangle
    // (fractions of chart space). ECMA-376 §21.2.2.32. Sample-2 slide-16 uses
    // this to keep its horizontal bar chart from spilling into the side
    // annotation column. We parse the same shape xlsx already exposes
    // (xMode, yMode, layoutTarget, x, y, w?, h?).
    let plot_area_manual_layout = plot_area
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "layout")
        .and_then(|layout| {
            layout
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "manualLayout")
        })
        .map(|ml| {
            let mut x_mode = "edge".to_string();
            let mut y_mode = "edge".to_string();
            let mut layout_target: Option<String> = None;
            let mut x = 0.0_f64;
            let mut y = 0.0_f64;
            let mut w: Option<f64> = None;
            let mut h: Option<f64> = None;
            for ch in ml.children().filter(|n| n.is_element()) {
                let val_str = attr(&ch, "val");
                match ch.tag_name().name() {
                    "xMode" => {
                        if let Some(v) = val_str {
                            x_mode = v;
                        }
                    }
                    "yMode" => {
                        if let Some(v) = val_str {
                            y_mode = v;
                        }
                    }
                    "layoutTarget" => {
                        layout_target = val_str;
                    }
                    "x" => {
                        if let Some(v) = val_str.and_then(|s| s.parse::<f64>().ok()) {
                            x = v;
                        }
                    }
                    "y" => {
                        if let Some(v) = val_str.and_then(|s| s.parse::<f64>().ok()) {
                            y = v;
                        }
                    }
                    "w" => {
                        w = val_str.and_then(|s| s.parse::<f64>().ok());
                    }
                    "h" => {
                        h = val_str.and_then(|s| s.parse::<f64>().ok());
                    }
                    _ => {}
                }
            }
            ChartManualLayout {
                x_mode,
                y_mode,
                layout_target,
                x,
                y,
                w,
                h,
            }
        });

    // `<c:scatterChart><c:scatterStyle val>` — ECMA-376 §21.2.2.42. Lives
    // directly under scatterChart, so a plot_area descendant walk is enough.
    let scatter_style = if chart_type == "scatter" {
        plot_area
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "scatterStyle")
            .and_then(|n| attr(&n, "val"))
    } else {
        None
    };

    // Axis titles + run props (ECMA-376 §21.2.2.6 `CT_Title`). Iterate every
    // `<c:catAx>`/`<c:valAx>` so the scatter case — two `<c:valAx>`, no
    // `<c:catAx>` — resolves correctly: a `<c:valAx>` whose `<c:axPos val>` is
    // `b`/`t` is the horizontal (X) axis → cat-axis title; `l`/`r` is the
    // vertical (Y) axis → val-axis title. A real `<c:catAx>` always feeds the
    // cat-axis title. First title wins for each axis (matches the xlsx parser).
    let mut cat_axis_title: Option<String> = None;
    let mut cat_axis_title_size: Option<i32> = None;
    let mut cat_axis_title_bold: Option<bool> = None;
    let mut cat_axis_title_color: Option<String> = None;
    let mut cat_axis_title_face: Option<String> = None;
    let mut val_axis_title: Option<String> = None;
    let mut val_axis_title_size: Option<i32> = None;
    let mut val_axis_title_bold: Option<bool> = None;
    let mut val_axis_title_color: Option<String> = None;
    let mut val_axis_title_face: Option<String> = None;
    for ax in plot_area
        .children()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "catAx" | "dateAx" | "valAx"))
    {
        let is_cat = if matches!(ax.tag_name().name(), "catAx" | "dateAx") {
            true
        } else {
            // valAx: disambiguate by axPos (b/t → X/cat, l/r → Y/val).
            let ax_pos = ax
                .children()
                .find(|n| n.is_element() && n.tag_name().name() == "axPos")
                .and_then(|n| attr(&n, "val"))
                .unwrap_or_default();
            matches!(ax_pos.as_str(), "b" | "t")
        };
        if is_cat {
            if cat_axis_title.is_none() {
                let (t, sz, b, col) = extract_axis_title_with_props_resolved(ax, color_resolver);
                if t.is_some() {
                    cat_axis_title = t;
                    cat_axis_title_size = sz;
                    cat_axis_title_bold = b;
                    cat_axis_title_color = col;
                    cat_axis_title_face = extract_axis_title_face(ax);
                }
            }
        } else if val_axis_title.is_none() {
            let (t, sz, b, col) = extract_axis_title_with_props_resolved(ax, color_resolver);
            if t.is_some() {
                val_axis_title = t;
                val_axis_title_size = sz;
                val_axis_title_bold = b;
                val_axis_title_color = col;
                val_axis_title_face = extract_axis_title_face(ax);
            }
        }
    }

    // Axis tick-label bold flags (`<c:txPr>…defRPr@b`) and the chart-title bold
    // flag (`<c:title>…defRPr@b`). These were never serialized before; wiring
    // them through reaches parity with the xlsx parser so the renderer's
    // ST_Style bold handling applies uniformly. All three come from the shared
    // ooxml-common helpers so the two parsers stay in lockstep. The chart-title
    // bold helper expects the `<c:title>`'s parent, so pass `title_node_opt`'s
    // parent (the element that holds it as a direct child).
    let cat_axis_font_bold = cat_ax.and_then(extract_axis_tick_label_bold);
    let val_axis_font_bold = val_ax.and_then(extract_axis_tick_label_bold);
    let title_font_bold = title_node_opt
        .and_then(|t| t.parent())
        .and_then(extract_chart_title_bold);

    // Explicit chartSpace border from `<c:chartSpace><c:spPr><a:ln>` (ECMA-376
    // §21.2.2.5 / DrawingML §20.1.2.2.24). Shared with the xlsx parser via
    // `extract_chart_space_border` so the locked policy
    // (border only on an explicit paintable line; `<a:noFill/>` → color None;
    // srgb inside solidFill → hex; `@w` captured as u32; schemeClr unresolved)
    // stays in lockstep. `root` is the `<c:chartSpace>` element here.
    let (chart_border_color, chart_border_width_emu) = extract_chart_space_border(root);

    // `<c:date1904>` (ECMA-376 §21.2.2.38) — direct child of `<c:chartSpace>`
    // (`root`). Shared with the xlsx parser via ooxml-common so both honor the
    // CT_Boolean implied-true semantics.
    let date1904 = extract_chart_date1904(root);

    // `<c:chart><c:dispBlanksAs>` (ECMA-376 §21.2.2.42) — null-cell plotting for
    // line/area. Shared with the xlsx parser via ooxml-common.
    let disp_blanks_as = extract_disp_blanks_as(root);

    // ── Chart text font faces (CH10) ────────────────────────────────────────
    // Tick-label faces (`<c:catAx|valAx><c:txPr>…<a:latin>`), data-label face
    // (`<c:dLbls><c:txPr>…<a:latin>`) and legend text props, all via the shared
    // ooxml-common extractors so pptx/xlsx stay in lockstep. Absent faces stay
    // None; the renderer falls back to the theme body/heading font.
    let cat_axis_font_face = cat_ax.and_then(extract_axis_tick_label_face);
    let val_axis_font_face = val_ax.and_then(extract_axis_tick_label_face);
    let data_label_font_face = extract_data_label_face(root);
    let (legend_font_face, legend_font_size_hpt, legend_font_bold) =
        extract_legend_text_props(root);
    let legend_font_color = { extract_legend_font_color(root, color_resolver) };
    // Theme fallback fonts: the resolver supplies the theme's major/minor Latin
    // faces (pptx keys them `+mj-lt` / `+mn-lt` in its color+font map). None
    // when the theme lacks a fontScheme. The renderer uses these when a chart
    // text run carries no explicit face.
    let theme_major_font_latin = color_resolver.theme_major_font_latin();
    let theme_minor_font_latin = color_resolver.theme_minor_font_latin();

    // ── Pie / doughnut geometry (CH8) ───────────────────────────────────────
    // holeSize (doughnut) / firstSliceAng (pie + doughnut), shared extractors.
    let hole_size = extract_hole_size(root);
    let first_slice_angle = extract_first_slice_angle(root);

    // ── Axis scale model (CH6) ──────────────────────────────────────────────
    // Gridline presence, manual major/minor units, log scale and orientation —
    // all via the shared ooxml-common extractors on the primary val/cat axes.
    // `<c:majorGridlines>` presence: Office writes it on the value axis by
    // default (renderer keeps its historical always-on when the field is None),
    // so we only emit `Some(false)` when a value axis EXISTS without the element.
    let val_axis_major_gridlines = val_ax.map(|ax| axis_has_major_gridlines(ax));
    let cat_axis_major_gridlines = cat_ax.map(|ax| axis_has_major_gridlines(ax));
    // `<c:majorGridlines><c:spPr><a:ln>` colour/width — the explicit gridline
    // style (e.g. sample-1 slide 5's `accent3` 0.25 pt value-axis gridlines).
    // `(None, None)` when absent, so the renderer keeps its faint default.
    let (val_axis_gridline_color, val_axis_gridline_width_emu) = val_ax
        .map(|ax| extract_gridline_style(ax, color_resolver))
        .unwrap_or((None, None));
    let (cat_axis_gridline_color, cat_axis_gridline_width_emu) = cat_ax
        .map(|ax| extract_gridline_style(ax, color_resolver))
        .unwrap_or((None, None));
    let val_axis_minor_gridlines = val_ax.map(|ax| axis_has_minor_gridlines(ax));
    let val_axis_major_unit = val_ax.and_then(extract_axis_major_unit);
    let val_axis_minor_unit = val_ax.and_then(extract_axis_minor_unit);
    let val_axis_log_base = val_ax.and_then(extract_axis_log_base);
    let val_axis_orientation = val_ax.and_then(extract_axis_orientation);
    let cat_axis_orientation = cat_ax.and_then(extract_axis_orientation);
    let cat_axis_tick_label_pos = cat_ax.and_then(extract_axis_tick_label_pos);
    let val_axis_tick_label_pos = val_ax.and_then(extract_axis_tick_label_pos);
    let cat_axis_label_rotation = cat_ax.and_then(extract_axis_tick_label_rotation);

    // Chart title font face (`<c:title>…<a:latin>`) — parity with xlsx, which
    // already extracts it. `extract_axis_title_face` scopes to a node's
    // direct-child `<c:title>`, so pass the title's parent (`<c:chart>`).
    let title_font_face = title_node_opt
        .and_then(|t| t.parent())
        .and_then(extract_axis_title_face);

    // Minor tick marks (ECMA-376 §21.2.2.115) — raw ST_TickMark string, `None`
    // when the axis omits `<c:minorTickMark>` (renderer default applies).
    let cat_axis_minor_tick_mark = cat_ax.and_then(|n| extract_axis_tick_mark(n, "minorTickMark"));
    let val_axis_minor_tick_mark = val_ax.and_then(|n| extract_axis_tick_mark(n, "minorTickMark"));

    // Axis crossing (`<c:crosses>` / `<c:crossesAt>`, ECMA-376 §21.2.2.33/.34).
    let (cat_axis_crosses, cat_axis_crosses_at) =
        cat_ax.map(extract_axis_crosses).unwrap_or((None, None));
    let (val_axis_crosses, val_axis_crosses_at) =
        val_ax.map(extract_axis_crosses).unwrap_or((None, None));

    // Category-axis explicit scaling bounds (`<c:scaling><c:min|max>`).
    let (cat_axis_min, cat_axis_max) = cat_ax.map(extract_axis_min_max).unwrap_or((None, None));

    // `<c:radarChart><c:radarStyle>` (ECMA-376 §21.2.3.10).
    let radar_style = extract_radar_style(root);

    // Legend `<c:layout><c:manualLayout>` (ECMA-376 §21.2.2.31).
    let legend_manual_layout = root
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "legend")
        .and_then(extract_legend_manual_layout);

    // Chart-title `<c:title><c:layout><c:manualLayout>` (ECMA-376 §21.2.2.88).
    let title_manual_layout = title_node_opt
        .and_then(|t| child(t, "layout"))
        .and_then(extract_manual_layout);

    // §21.2.2.227 varyColors chart-level flag. Emitted (`Some(true)`) for a
    // SINGLE-series bar/column chart that varies by point — the case where the
    // core renderer must color each bar per point and list one legend entry per
    // point. "Vary by point" is the default for a lone bar series, so an ABSENT
    // `<c:varyColors>` resolves to `true` here; an explicit `val="0"` (the way
    // Office records "force one color") leaves it `None`. The pie family already
    // varies by point via `chart_type` + `data_point_colors`, so it stays `None`
    // to keep the wire byte-identical for every existing pie/doughnut chart.
    let vary_colors = {
        let is_pie_family = chart_type == "pie" || chart_type == "doughnut";
        let is_bar_family = chart_type.contains("Bar");
        if !is_pie_family && is_bar_family && series_count == 1 {
            let vary = ser_nodes[0]
                .parent()
                .and_then(|g| bool_child(g, "varyColors"))
                .unwrap_or(true);
            if vary {
                Some(true)
            } else {
                None
            }
        } else {
            None
        }
    };

    Some(ChartModel {
        chart_type,
        title,
        categories,
        series,
        vary_colors,
        val_max,
        val_min,
        subtotal_indices: vec![],
        show_data_labels,
        cat_axis_hidden,
        val_axis_hidden,
        plot_area_bg,
        chart_bg,
        show_legend,
        cat_axis_cross_between,
        val_axis_major_tick_mark,
        cat_axis_major_tick_mark,
        title_font_size_hpt,
        title_font_color,
        title_font_face,
        cat_axis_font_size_hpt,
        val_axis_font_size_hpt,
        cat_axis_font_color,
        val_axis_font_color,
        cat_axis_line_color,
        cat_axis_line_width_emu,
        cat_axis_line_hidden,
        val_axis_line_color,
        val_axis_line_width_emu,
        val_axis_line_hidden,
        data_label_font_size_hpt,
        legend_pos,
        bar_gap_width,
        bar_overlap,
        data_label_position,
        data_label_font_color,
        data_label_format_code,
        val_axis_format_code,
        plot_area_manual_layout,
        scatter_style,
        cat_axis_title,
        val_axis_title,
        // TS `ChartElement` renamed the axis-title run-prop fields to the
        // core `ChartModel` names (`…TitleFontSizeHpt/Bold/Color`); the
        // parser locals keep the shorter legacy names.
        cat_axis_title_font_size_hpt: cat_axis_title_size,
        cat_axis_title_font_bold: cat_axis_title_bold,
        cat_axis_title_font_color: cat_axis_title_color,
        val_axis_title_font_size_hpt: val_axis_title_size,
        val_axis_title_font_bold: val_axis_title_bold,
        val_axis_title_font_color: val_axis_title_color,
        title_font_bold,
        cat_axis_font_bold,
        val_axis_font_bold,
        chart_border_color,
        chart_border_width_emu,
        secondary_val_axis,
        // Pie/doughnut geometry (CH8) + chart text font faces (CH10).
        hole_size,
        first_slice_angle,
        cat_axis_font_face,
        val_axis_font_face,
        cat_axis_title_font_face: cat_axis_title_face,
        val_axis_title_font_face: val_axis_title_face,
        data_label_font_face,
        legend_font_face,
        legend_font_color,
        legend_font_size_hpt,
        legend_font_bold,
        theme_major_font_latin,
        theme_minor_font_latin,
        // ChartModel fields the legacy pptx `<c:chart>` path leaves unset
        // (they were never in the pptx `ChartElement` copy, so they defaulted
        // to `undefined` on the TS side and stay absent on the wire).
        val_axis_minor_tick_mark,
        cat_axis_minor_tick_mark,
        legend_manual_layout,
        title_manual_layout,
        cat_axis_crosses,
        cat_axis_crosses_at,
        val_axis_crosses,
        val_axis_crosses_at,
        cat_axis_format_code,
        cat_axis_min,
        cat_axis_max,
        radar_style,
        date1904,
        disp_blanks_as,
        // ── Axis scale model (CH6) ──────────────────────────────────────
        val_axis_major_gridlines,
        cat_axis_major_gridlines,
        val_axis_gridline_color,
        val_axis_gridline_width_emu,
        cat_axis_gridline_color,
        cat_axis_gridline_width_emu,
        val_axis_minor_gridlines,
        val_axis_major_unit,
        val_axis_minor_unit,
        val_axis_log_base,
        val_axis_orientation,
        cat_axis_orientation,
        cat_axis_tick_label_pos,
        val_axis_tick_label_pos,
        cat_axis_label_rotation,
        stock_hi_low_lines,
        stock_hi_low_line_color,
        stock_up_down_bars,
        // Legacy `c:` charts never carry the chartEx boxWhisker/sunburst model.
        chartex_box: None,
        chartex_sunburst: None,
        chartex_accents: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use roxmltree::Document;

    fn root_of(xml: &str) -> Document<'_> {
        Document::parse(xml).expect("parse fixture")
    }

    #[test]
    fn canonical_chart_type_bar_matrix() {
        // Mirrors the TS `canonicalChartType` bar branch (ST_BarDir "bar" =
        // horizontal). Every (grouping, dir) pair must map to the same string
        // the renderer dispatches on.
        assert_eq!(
            canonical_chart_type("bar", "col", "clustered"),
            "clusteredBar"
        );
        assert_eq!(
            canonical_chart_type("bar", "bar", "clustered"),
            "clusteredBarH"
        );
        assert_eq!(canonical_chart_type("bar", "col", "stacked"), "stackedBar");
        assert_eq!(canonical_chart_type("bar", "bar", "stacked"), "stackedBarH");
        assert_eq!(
            canonical_chart_type("bar", "col", "percentStacked"),
            "stackedBarPct"
        );
        assert_eq!(
            canonical_chart_type("bar", "bar", "percentStacked"),
            "stackedBarHPct"
        );
        // Unknown grouping → clustered fallback (matches the TS default arm).
        assert_eq!(
            canonical_chart_type("bar", "col", "standard"),
            "clusteredBar"
        );
    }

    #[test]
    fn canonical_chart_type_line_area_and_passthrough() {
        assert_eq!(canonical_chart_type("line", "col", "standard"), "line");
        assert_eq!(
            canonical_chart_type("line", "col", "stacked"),
            "stackedLine"
        );
        assert_eq!(
            canonical_chart_type("line", "col", "percentStacked"),
            "stackedLinePct"
        );
        assert_eq!(canonical_chart_type("area", "col", "standard"), "area");
        assert_eq!(
            canonical_chart_type("area", "col", "stacked"),
            "stackedArea"
        );
        assert_eq!(
            canonical_chart_type("area", "col", "percentStacked"),
            "stackedAreaPct"
        );
        // Families the renderer already names canonically pass through verbatim.
        for t in ["pie", "doughnut", "scatter", "bubble", "radar", "waterfall"] {
            assert_eq!(canonical_chart_type(t, "col", "clustered"), t);
        }
    }

    /// The wire contract: a `ChartModel` must serialize with the same camelCase
    /// keys the TS `ChartModel` declares, REQUIRED fields present even when
    /// `None`/`false`/empty, OPTIONAL fields dropped when unset. This is the
    /// Rust-side oracle that pins the emitted JSON shape.
    #[test]
    fn chart_model_serializes_canonical_shape() {
        let m = ChartModel {
            chart_type: "clusteredBar".to_string(),
            title: None,
            categories: vec!["A".to_string(), "B".to_string()],
            series: vec![ChartSeries {
                name: "S1".to_string(),
                color: Some("FF0000".to_string()),
                values: vec![Some(1.0), None, Some(3.0)],
                data_point_colors: None,
                data_label_colors: None,
                label_color: None,
                series_type: None,
                use_secondary_axis: None,
                categories: None,
                show_marker: None,
                val_format_code: None,
                marker_symbol: None,
                marker_size: None,
                marker_fill: None,
                marker_line: None,
                data_point_overrides: None,
                data_label_overrides: None,
                series_data_labels: None,
                err_bars: None,
                bubble_sizes: None,
                smooth: None,
                trend_lines: None,
                line_hidden: None,
            }],
            vary_colors: None,
            show_data_labels: false,
            val_min: None,
            val_max: None,
            cat_axis_title: None,
            val_axis_title: None,
            cat_axis_hidden: false,
            val_axis_hidden: false,
            cat_axis_line_hidden: false,
            val_axis_line_hidden: false,
            plot_area_bg: None,
            chart_bg: Some("FFFFFF".to_string()),
            show_legend: false,
            legend_pos: None,
            cat_axis_cross_between: "between".to_string(),
            val_axis_major_tick_mark: "out".to_string(),
            cat_axis_major_tick_mark: "out".to_string(),
            title_font_size_hpt: None,
            title_font_color: None,
            title_font_face: None,
            cat_axis_font_size_hpt: None,
            val_axis_font_size_hpt: None,
            data_label_font_size_hpt: None,
            subtotal_indices: vec![],
            val_axis_minor_tick_mark: None,
            cat_axis_minor_tick_mark: None,
            cat_axis_font_color: None,
            val_axis_font_color: None,
            legend_manual_layout: None,
            val_axis_format_code: None,
            bar_gap_width: None,
            bar_overlap: None,
            data_label_position: None,
            data_label_font_color: None,
            data_label_format_code: None,
            title_font_bold: None,
            cat_axis_font_bold: None,
            val_axis_font_bold: None,
            cat_axis_title_font_size_hpt: None,
            cat_axis_title_font_bold: None,
            cat_axis_title_font_color: None,
            val_axis_title_font_size_hpt: None,
            val_axis_title_font_bold: None,
            val_axis_title_font_color: None,
            chart_border_color: None,
            chart_border_width_emu: None,
            cat_axis_crosses: None,
            cat_axis_crosses_at: None,
            val_axis_crosses: None,
            val_axis_crosses_at: None,
            cat_axis_line_color: None,
            cat_axis_line_width_emu: None,
            val_axis_line_color: None,
            val_axis_line_width_emu: None,
            cat_axis_format_code: None,
            cat_axis_min: None,
            cat_axis_max: None,
            title_manual_layout: None,
            plot_area_manual_layout: None,
            scatter_style: None,
            radar_style: None,
            secondary_val_axis: None,
            hole_size: None,
            first_slice_angle: None,
            cat_axis_font_face: None,
            val_axis_font_face: None,
            cat_axis_title_font_face: None,
            val_axis_title_font_face: None,
            data_label_font_face: None,
            legend_font_face: None,
            legend_font_color: None,
            legend_font_size_hpt: None,
            legend_font_bold: None,
            theme_major_font_latin: None,
            theme_minor_font_latin: None,
            date1904: false,
            disp_blanks_as: None,
            val_axis_major_gridlines: None,
            cat_axis_major_gridlines: None,
            val_axis_gridline_color: None,
            val_axis_gridline_width_emu: None,
            cat_axis_gridline_color: None,
            cat_axis_gridline_width_emu: None,
            val_axis_minor_gridlines: None,
            val_axis_major_unit: None,
            val_axis_minor_unit: None,
            val_axis_log_base: None,
            val_axis_orientation: None,
            cat_axis_orientation: None,
            cat_axis_tick_label_pos: None,
            val_axis_tick_label_pos: None,
            cat_axis_label_rotation: None,
            stock_hi_low_lines: None,
            stock_hi_low_line_color: None,
            stock_up_down_bars: None,
            chartex_box: None,
            chartex_sunburst: None,
            chartex_accents: None,
        };
        let v = serde_json::to_value(&m).unwrap();
        let obj = v.as_object().unwrap();
        // Required scalar keys present with camelCase names, even when None/false.
        assert_eq!(obj["chartType"], "clusteredBar");
        assert!(obj["title"].is_null());
        assert_eq!(obj["showDataLabels"], false);
        assert_eq!(obj["catAxisHidden"], false);
        assert_eq!(obj["catAxisCrossBetween"], "between");
        assert_eq!(obj["valAxisMajorTickMark"], "out");
        assert!(obj["plotAreaBg"].is_null());
        assert_eq!(obj["chartBg"], "FFFFFF");
        assert_eq!(obj["subtotalIndices"], serde_json::json!([]));
        // Optional unset keys dropped from the wire.
        assert!(!obj.contains_key("barGapWidth"));
        assert!(!obj.contains_key("secondaryValAxis"));
        assert!(!obj.contains_key("catAxisFontColor"));
        // date1904 is dropped from the wire when false (default 1900 system).
        assert!(!obj.contains_key("date1904"));
        // Series: required present, optional dropped; array null preserved.
        let s0 = &obj["series"][0];
        assert_eq!(s0["name"], "S1");
        assert_eq!(s0["color"], "FF0000");
        assert_eq!(s0["values"], serde_json::json!([1.0, null, 3.0]));
        assert!(!s0.as_object().unwrap().contains_key("showMarker"));
        // Round-trips back to an equal model (Deserialize parity).
        let back: ChartModel = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn legend_present_with_pos() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:legend><c:legendPos val="t"/></c:legend>
        </c:chart>"#;
        let d = root_of(xml);
        let (show, pos) = extract_legend(d.root_element());
        assert!(show);
        assert_eq!(pos.as_deref(), Some("t"));
    }

    #[test]
    fn legend_absent() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        let (show, pos) = extract_legend(d.root_element());
        assert!(!show);
        assert!(pos.is_none());
    }

    #[test]
    fn bar_gap_overlap_default_to_none() {
        let xml =
            r#"<c:barChart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert_eq!(extract_bar_gap_overlap(d.root_element()), (None, None));
    }

    #[test]
    fn bar_gap_overlap_explicit() {
        let xml = r#"<c:barChart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:gapWidth val="50"/>
            <c:overlap val="100"/>
        </c:barChart>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_bar_gap_overlap(d.root_element()),
            (Some(50), Some(100))
        );
    }

    #[test]
    fn data_label_position() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:plotArea><c:dLbls><c:dLblPos val="ctr"/></c:dLbls></c:plotArea>
        </c:chart>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_data_label_position(d.root_element()).as_deref(),
            Some("ctr")
        );
    }

    #[test]
    fn axis_delete_truthy_variants() {
        for (val, expect) in [
            ("1", true),
            ("0", false),
            ("true", true),
            ("false", false),
            ("True", true),
        ] {
            let xml = format!(
                r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
                <c:delete val="{val}"/>
            </c:valAx>"#
            );
            let d = root_of(&xml);
            assert_eq!(axis_is_deleted(d.root_element()), expect, "val={val}");
        }
    }

    #[test]
    fn axis_delete_default_false() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert!(!axis_is_deleted(d.root_element()));
    }

    #[test]
    fn axis_min_max() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:scaling><c:max val="2500"/><c:min val="0"/></c:scaling>
        </c:valAx>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_axis_min_max(d.root_element()),
            (Some(0.0), Some(2500.0))
        );
    }

    #[test]
    fn series_smooth_present_and_absent() {
        // No `<c:smooth>` → None (straight-polyline default).
        let none =
            root_of(r#"<c:ser xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#);
        assert_eq!(extract_series_smooth(none.root_element()), None);
        // `<c:smooth val="1"/>` → Some(true).
        let on = root_of(
            r#"<c:ser xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:smooth val="1"/></c:ser>"#,
        );
        assert_eq!(extract_series_smooth(on.root_element()), Some(true));
        // `<c:smooth val="0"/>` → Some(false) (explicit off).
        let off = root_of(
            r#"<c:ser xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:smooth val="0"/></c:ser>"#,
        );
        assert_eq!(extract_series_smooth(off.root_element()), Some(false));
        // Bare `<c:smooth/>` → Some(true) (CT_Boolean implied-true).
        let bare = root_of(
            r#"<c:ser xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:smooth/></c:ser>"#,
        );
        assert_eq!(extract_series_smooth(bare.root_element()), Some(true));
    }

    #[test]
    fn disp_blanks_as_variants() {
        // Absent element → None (renderer defaults to "gap").
        let absent = root_of(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart/></c:chartSpace>"#,
        );
        assert_eq!(extract_disp_blanks_as(absent.root_element()), None);
        // Explicit values pass through.
        for want in ["gap", "zero", "span"] {
            let xml = format!(
                r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:dispBlanksAs val="{want}"/></c:chart></c:chartSpace>"#,
            );
            assert_eq!(
                extract_disp_blanks_as(root_of(&xml).root_element()).as_deref(),
                Some(want)
            );
        }
        // Bare `<c:dispBlanksAs/>` → XSD @val default "zero".
        let bare = root_of(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:dispBlanksAs/></c:chart></c:chartSpace>"#,
        );
        assert_eq!(
            extract_disp_blanks_as(bare.root_element()).as_deref(),
            Some("zero")
        );
    }

    #[test]
    fn axis_format_code_skips_general() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:numFmt formatCode="General"/>
        </c:valAx>"#;
        let d = root_of(xml);
        assert!(extract_axis_format_code(d.root_element()).is_none());
    }

    #[test]
    fn axis_format_code_passes_through() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:numFmt formatCode="0.0%"/>
        </c:valAx>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_axis_format_code(d.root_element()).as_deref(),
            Some("0.0%")
        );
    }

    /// Test resolver: returns the schemeClr@val verbatim, or the srgbClr@val
    /// uppercased. Just enough to drive `extract_data_label_font_color`.
    struct StubResolver;
    impl ColorResolver for StubResolver {
        fn resolve_solid_fill(&self, node: Node) -> Option<String> {
            for c in node.children().filter(|n| n.is_element()) {
                match c.tag_name().name() {
                    "srgbClr" => return c.attribute("val").map(|v| v.to_uppercase()),
                    "schemeClr" => return c.attribute("val").map(|v| v.to_string()),
                    _ => {}
                }
            }
            None
        }
    }

    #[test]
    fn data_label_font_color_resolves_via_resolver() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:plotArea>
                <c:dLbls>
                    <c:txPr><a:p><a:r><a:rPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></a:rPr></a:r></a:p></c:txPr>
                </c:dLbls>
            </c:plotArea>
        </c:chart>"#;
        let d = root_of(xml);
        let got = extract_data_label_font_color(d.root_element(), &StubResolver);
        assert_eq!(got.as_deref(), Some("bg1"));
    }

    #[test]
    fn data_label_font_color_skips_label_background_fill() {
        // `<c:spPr><a:solidFill>` (label background) must not be picked up;
        // only the text fill inside `<c:txPr>` counts.
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:plotArea>
                <c:dLbls>
                    <c:spPr><a:solidFill><a:srgbClr val="aabbcc"/></a:solidFill></c:spPr>
                </c:dLbls>
            </c:plotArea>
        </c:chart>"#;
        let d = root_of(xml);
        let got = extract_data_label_font_color(d.root_element(), &StubResolver);
        assert!(
            got.is_none(),
            "spPr fill must not leak into the font color: got {got:?}"
        );
    }

    #[test]
    fn data_label_font_color_first_dlbls_wins() {
        // Mimics Office writers that put a chart-level dLbls block AND
        // per-series ones — the first txPr resolution wins.
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:plotArea>
                <c:dLbls>
                    <c:txPr><a:p><a:r><a:rPr><a:solidFill><a:srgbClr val="ffffff"/></a:solidFill></a:rPr></a:r></a:p></c:txPr>
                </c:dLbls>
                <c:barChart>
                    <c:ser><c:dLbls>
                        <c:txPr><a:p><a:r><a:rPr><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:rPr></a:r></a:p></c:txPr>
                    </c:dLbls></c:ser>
                </c:barChart>
            </c:plotArea>
        </c:chart>"#;
        let d = root_of(xml);
        let got = extract_data_label_font_color(d.root_element(), &StubResolver);
        assert_eq!(got.as_deref(), Some("FFFFFF"));
    }

    #[test]
    fn axis_tick_label_color_from_txpr() {
        let xml = r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:spPr><a:ln><a:solidFill><a:srgbClr val="d9d9d9"/></a:solidFill></a:ln></c:spPr>
            <c:txPr><a:p><a:pPr><a:defRPr><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr>
        </c:catAx>"#;
        let d = root_of(xml);
        // The txPr text fill (bg1) is returned — the spPr line fill must not leak.
        let got = extract_axis_tick_label_color(d.root_element(), &StubResolver);
        assert_eq!(got.as_deref(), Some("bg1"));
    }

    #[test]
    fn axis_tick_label_color_absent() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert!(extract_axis_tick_label_color(d.root_element(), &StubResolver).is_none());
    }

    #[test]
    fn axis_line_style_solid_with_width() {
        let xml = r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:spPr><a:noFill/><a:ln w="9525"><a:solidFill><a:srgbClr val="d9d9d9"/></a:solidFill></a:ln></c:spPr>
        </c:catAx>"#;
        let d = root_of(xml);
        let (color, width, no_fill) = extract_axis_line_style(d.root_element(), &StubResolver);
        assert_eq!(color.as_deref(), Some("D9D9D9"));
        assert_eq!(width, Some(9525));
        assert!(!no_fill);
    }

    #[test]
    fn axis_line_style_nofill_line() {
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:spPr><a:ln w="9525"><a:noFill/></a:ln></c:spPr>
        </c:valAx>"#;
        let d = root_of(xml);
        let (color, width, no_fill) = extract_axis_line_style(d.root_element(), &StubResolver);
        assert!(color.is_none());
        assert_eq!(width, Some(9525));
        assert!(no_fill);
    }

    #[test]
    fn axis_line_style_absent() {
        let xml = r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_axis_line_style(d.root_element(), &StubResolver),
            (None, None, false)
        );
    }

    #[test]
    fn gridline_style_solid_scheme_with_width() {
        // sample-1 slide 5: `<c:majorGridlines><c:spPr><a:ln w="3175">
        // <a:solidFill><a:schemeClr val="accent3"/>` → the explicit gridline
        // colour + 0.25 pt width (3175 EMU) the renderer must honor.
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:majorGridlines><c:spPr><a:ln w="3175"><a:solidFill><a:schemeClr val="accent3"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>
        </c:valAx>"#;
        let d = root_of(xml);
        let (color, width) = extract_gridline_style(d.root_element(), &StubResolver);
        assert_eq!(color.as_deref(), Some("accent3"));
        assert_eq!(width, Some(3175));
    }

    #[test]
    fn gridline_style_present_without_sppr() {
        // `<c:majorGridlines/>` with no `<c:spPr>` → gridlines are requested
        // (presence-only) but carry no explicit colour/width; the renderer keeps
        // its faint default. `(None, None)`.
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:majorGridlines/>
        </c:valAx>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_gridline_style(d.root_element(), &StubResolver),
            (None, None)
        );
    }

    #[test]
    fn gridline_style_absent() {
        // No `<c:majorGridlines>` at all → `(None, None)`.
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_gridline_style(d.root_element(), &StubResolver),
            (None, None)
        );
    }

    #[test]
    fn chartex_axis_hidden_value_only() {
        let xml = r#"<cx:chartSpace xmlns:cx="http://schemas.microsoft.com/office/drawing/2014/chartex">
            <cx:axis id="0"><cx:catScaling/></cx:axis>
            <cx:axis id="1" hidden="1"><cx:valScaling/></cx:axis>
        </cx:chartSpace>"#;
        let d = root_of(xml);
        assert_eq!(extract_chartex_axis_hidden(d.root_element()), (false, true));
    }

    #[test]
    fn chart_title_text_size_bold_srgb() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:title><c:tx><c:rich>
                <a:p><a:pPr><a:defRPr sz="1400" b="1"><a:solidFill><a:srgbClr val="1B4332"/></a:solidFill></a:defRPr></a:pPr>
                <a:r><a:t>Carbon &amp; Growth</a:t></a:r></a:p>
            </c:rich></c:tx></c:title>
        </c:chart>"#;
        let d = root_of(xml);
        let root = d.root_element();
        assert_eq!(
            extract_chart_title_text(root).as_deref(),
            Some("Carbon & Growth")
        );
        assert_eq!(extract_chart_title_size(root), Some(1400));
        assert_eq!(extract_chart_title_bold(root), Some(true));
        assert_eq!(extract_chart_title_srgb(root).as_deref(), Some("1B4332"));
    }

    #[test]
    fn chart_title_text_from_strref_cache() {
        // Title sourced from a strRef cache (`<c:v>`) rather than rich runs.
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:title><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Sales</c:v></c:pt></c:strCache></c:strRef></c:tx></c:title>
        </c:chart>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_chart_title_text(d.root_element()).as_deref(),
            Some("Sales")
        );
    }

    #[test]
    fn chart_title_srgb_skips_non_solidfill_srgb() {
        // An `<a:srgbClr>` that is NOT a direct child of `<a:solidFill>` (here a
        // gradient stop) must be ignored.
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:title><c:tx><c:rich><a:p><a:r><a:rPr>
                <a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="ABCDEF"/></a:gs></a:gsLst></a:gradFill>
            </a:rPr><a:t>T</a:t></a:r></a:p></c:rich></c:tx></c:title>
        </c:chart>"#;
        let d = root_of(xml);
        assert!(extract_chart_title_srgb(d.root_element()).is_none());
    }

    #[test]
    fn chart_title_helpers_absent() {
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        let root = d.root_element();
        assert!(extract_chart_title_text(root).is_none());
        assert!(extract_chart_title_size(root).is_none());
        assert!(extract_chart_title_bold(root).is_none());
        assert!(extract_chart_title_srgb(root).is_none());
    }

    #[test]
    fn chart_title_color_resolves_scheme_and_srgb() {
        // schemeClr (`tx2`) — resolved via the resolver, unlike the srgb-only
        // `extract_chart_title_srgb` which returns None for a scheme color.
        let scheme = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:title><c:tx><c:rich><a:p><a:pPr>
                <a:defRPr><a:solidFill><a:schemeClr val="tx2"/></a:solidFill></a:defRPr>
            </a:pPr><a:r><a:rPr><a:solidFill><a:schemeClr val="tx2"/></a:solidFill></a:rPr><a:t>T</a:t></a:r></a:p></c:rich></c:tx></c:title>
        </c:chart>"#;
        let d = root_of(scheme);
        assert_eq!(
            extract_chart_title_color(d.root_element(), &StubResolver).as_deref(),
            Some("tx2")
        );
        assert!(extract_chart_title_srgb(d.root_element()).is_none());

        // srgbClr — resolved (uppercased by StubResolver) too.
        let srgb = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:title><c:tx><c:rich><a:p><a:r><a:rPr><a:solidFill><a:srgbClr val="1b4332"/></a:solidFill></a:rPr><a:t>T</a:t></a:r></a:p></c:rich></c:tx></c:title>
        </c:chart>"#;
        let d2 = root_of(srgb);
        assert_eq!(
            extract_chart_title_color(d2.root_element(), &StubResolver).as_deref(),
            Some("1B4332")
        );
    }

    #[test]
    fn chart_title_color_skips_title_frame_sppr_fill() {
        // A `<c:title><c:spPr><a:solidFill>` is the title FRAME fill, not the
        // text color; it must be ignored (only run-property fills count).
        let xml = r#"<c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:title>
                <c:tx><c:rich><a:p><a:r><a:t>T</a:t></a:r></a:p></c:rich></c:tx>
                <c:spPr><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></c:spPr>
            </c:title>
        </c:chart>"#;
        let d = root_of(xml);
        assert!(extract_chart_title_color(d.root_element(), &StubResolver).is_none());
    }

    #[test]
    fn axis_title_with_props_resolved_scheme_color() {
        // The resolver-based axis-title variant resolves a schemeClr color.
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:axPos val="l"/>
            <c:title><c:tx><c:rich><a:p><a:r><a:rPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:rPr><a:t>Value</a:t></a:r></a:p></c:rich></c:tx></c:title>
        </c:valAx>"#;
        let d = root_of(xml);
        let (text, _sz, _b, color) =
            extract_axis_title_with_props_resolved(d.root_element(), &StubResolver);
        assert_eq!(text.as_deref(), Some("Value"));
        assert_eq!(color.as_deref(), Some("accent1"));
    }

    #[test]
    fn axis_title_with_props_full() {
        let xml = r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:axPos val="b"/>
            <c:title><c:tx><c:rich>
                <a:p><a:pPr><a:defRPr sz="1000" b="1"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:defRPr></a:pPr>
                <a:r><a:t>Category Axis</a:t></a:r></a:p>
            </c:rich></c:tx></c:title>
        </c:catAx>"#;
        let d = root_of(xml);
        let (text, size, bold, color) = extract_axis_title_with_props(d.root_element());
        assert_eq!(text.as_deref(), Some("Category Axis"));
        assert_eq!(size, Some(1000));
        assert_eq!(bold, Some(true));
        assert_eq!(color.as_deref(), Some("FF0000"));
    }

    #[test]
    fn axis_title_with_props_text_absent_all_none() {
        // Axis with no `<c:title>` → text None gates the props to None even
        // though run props could in theory be read elsewhere.
        let xml = r#"<c:valAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart">
            <c:axPos val="l"/>
        </c:valAx>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_axis_title_with_props(d.root_element()),
            (None, None, None, None)
        );
    }

    #[test]
    fn axis_tick_label_bold_variants() {
        for (b, expect) in [("1", Some(true)), ("0", Some(false)), ("true", Some(true))] {
            let xml = format!(
                r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                    <c:txPr><a:bodyPr/><a:p><a:pPr><a:defRPr b="{b}"/></a:pPr><a:endParaRPr/></a:p></c:txPr>
                </c:catAx>"#
            );
            let d = root_of(&xml);
            assert_eq!(
                extract_axis_tick_label_bold(d.root_element()),
                expect,
                "b={b}"
            );
        }
    }

    #[test]
    fn axis_tick_label_bold_absent() {
        let xml = r#"<c:catAx xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert!(extract_axis_tick_label_bold(d.root_element()).is_none());
    }

    #[test]
    fn chart_space_border_solid() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="1B4332"/></a:solidFill></a:ln></c:spPr>
        </c:chartSpace>"#;
        let d = root_of(xml);
        assert_eq!(
            extract_chart_space_border(d.root_element()),
            (Some("1B4332".to_string()), Some(19050))
        );
    }

    #[test]
    fn chart_space_border_nofill_color_none_width_kept() {
        let xml = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
            <c:spPr><a:ln w="12700"><a:noFill/></a:ln></c:spPr>
        </c:chartSpace>"#;
        let d = root_of(xml);
        // noFill turns the border off → color None, but @w is still reported.
        assert_eq!(
            extract_chart_space_border(d.root_element()),
            (None, Some(12700))
        );
    }

    #[test]
    fn chart_space_border_absent() {
        let xml =
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"/>"#;
        let d = root_of(xml);
        assert_eq!(extract_chart_space_border(d.root_element()), (None, None));
    }

    #[test]
    fn chart_date1904_variants() {
        // §21.2.2.38: CT_Boolean. Element present + val omitted ⇒ true.
        let ns = "http://schemas.openxmlformats.org/drawingml/2006/chart";
        let bare = format!(r#"<c:chartSpace xmlns:c="{ns}"><c:date1904/></c:chartSpace>"#);
        assert!(extract_chart_date1904(root_of(&bare).root_element()));

        let one = format!(r#"<c:chartSpace xmlns:c="{ns}"><c:date1904 val="1"/></c:chartSpace>"#);
        assert!(extract_chart_date1904(root_of(&one).root_element()));

        let word =
            format!(r#"<c:chartSpace xmlns:c="{ns}"><c:date1904 val="true"/></c:chartSpace>"#);
        assert!(extract_chart_date1904(root_of(&word).root_element()));

        let zero = format!(r#"<c:chartSpace xmlns:c="{ns}"><c:date1904 val="0"/></c:chartSpace>"#);
        assert!(!extract_chart_date1904(root_of(&zero).root_element()));

        // Word form of the falsey value: `val="false"` also disables the 1904
        // system (CT_Boolean accepts both "0" and "false").
        let false_word =
            format!(r#"<c:chartSpace xmlns:c="{ns}"><c:date1904 val="false"/></c:chartSpace>"#);
        assert!(!extract_chart_date1904(root_of(&false_word).root_element()));

        // Absent element ⇒ false (default 1900 system).
        let absent = format!(r#"<c:chartSpace xmlns:c="{ns}"/>"#);
        assert!(!extract_chart_date1904(root_of(&absent).root_element()));
    }

    // ── CH8 — pie / doughnut geometry ───────────────────────────────────────

    const C_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
    const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

    #[test]
    fn hole_size_from_doughnut() {
        let xml = format!(
            r#"<c:chart xmlns:c="{C_NS}"><c:plotArea><c:doughnutChart><c:holeSize val="60"/></c:doughnutChart></c:plotArea></c:chart>"#
        );
        assert_eq!(extract_hole_size(root_of(&xml).root_element()), Some(60));
        // Clamped to the ECMA 1–90 range.
        let hi = format!(
            r#"<c:chart xmlns:c="{C_NS}"><c:doughnutChart><c:holeSize val="200"/></c:doughnutChart></c:chart>"#
        );
        assert_eq!(extract_hole_size(root_of(&hi).root_element()), Some(90));
        // A pie chart has no hole → None even if a stray holeSize appears elsewhere.
        let pie = format!(r#"<c:chart xmlns:c="{C_NS}"><c:pieChart/></c:chart>"#);
        assert_eq!(extract_hole_size(root_of(&pie).root_element()), None);
    }

    #[test]
    fn first_slice_angle_from_pie_or_doughnut() {
        let pie = format!(
            r#"<c:chart xmlns:c="{C_NS}"><c:pieChart><c:firstSliceAng val="90"/></c:pieChart></c:chart>"#
        );
        assert_eq!(
            extract_first_slice_angle(root_of(&pie).root_element()),
            Some(90)
        );
        let dn = format!(
            r#"<c:chart xmlns:c="{C_NS}"><c:doughnutChart><c:firstSliceAng val="270"/></c:doughnutChart></c:chart>"#
        );
        assert_eq!(
            extract_first_slice_angle(root_of(&dn).root_element()),
            Some(270)
        );
        // Absent ⇒ None (renderer defaults to 0).
        let none = format!(r#"<c:chart xmlns:c="{C_NS}"><c:pieChart/></c:chart>"#);
        assert_eq!(
            extract_first_slice_angle(root_of(&none).root_element()),
            None
        );
    }

    #[test]
    fn dpt_explosion() {
        let with =
            format!(r#"<c:dPt xmlns:c="{C_NS}"><c:idx val="1"/><c:explosion val="25"/></c:dPt>"#);
        assert_eq!(
            extract_dpt_explosion(root_of(&with).root_element()),
            Some(25)
        );
        let without = format!(r#"<c:dPt xmlns:c="{C_NS}"><c:idx val="1"/></c:dPt>"#);
        assert_eq!(
            extract_dpt_explosion(root_of(&without).root_element()),
            None
        );
    }

    // ── CH10 — chart text font faces ────────────────────────────────────────

    #[test]
    fn axis_tick_and_title_faces() {
        // Tick face lives in the axis `<c:txPr>`; the title face in `<c:title>`.
        // Extractors must NOT cross-contaminate.
        let xml = format!(
            r#"<c:valAx xmlns:c="{C_NS}" xmlns:a="{A_NS}">
                 <c:title><a:p><a:r><a:rPr><a:latin typeface="Georgia"/></a:rPr><a:t>Y</a:t></a:r></a:p></c:title>
                 <c:txPr><a:p><a:pPr><a:defRPr><a:latin typeface="Verdana"/></a:defRPr></a:pPr></a:p></c:txPr>
               </c:valAx>"#
        );
        let root = root_of(&xml);
        let ax = root.root_element();
        assert_eq!(extract_axis_tick_label_face(ax).as_deref(), Some("Verdana"));
        assert_eq!(extract_axis_title_face(ax).as_deref(), Some("Georgia"));
    }

    #[test]
    fn data_label_face_scoped_to_dlbls() {
        let xml = format!(
            r#"<c:chart xmlns:c="{C_NS}" xmlns:a="{A_NS}">
                 <c:plotArea><c:barChart>
                   <c:dLbls><c:txPr><a:p><a:pPr><a:defRPr><a:latin typeface="Consolas"/></a:defRPr></a:pPr></a:p></c:txPr></c:dLbls>
                 </c:barChart></c:plotArea>
               </c:chart>"#
        );
        assert_eq!(
            extract_data_label_face(root_of(&xml).root_element()).as_deref(),
            Some("Consolas")
        );
    }

    #[test]
    fn legend_text_props_face_size_bold() {
        let xml = format!(
            r#"<c:chart xmlns:c="{C_NS}" xmlns:a="{A_NS}">
                 <c:legend><c:legendPos val="b"/>
                   <c:txPr><a:p><a:pPr><a:defRPr sz="1100" b="1"><a:latin typeface="Calibri"/></a:defRPr></a:pPr></a:p></c:txPr>
                 </c:legend>
               </c:chart>"#
        );
        let (face, size, bold) = extract_legend_text_props(root_of(&xml).root_element());
        assert_eq!(face.as_deref(), Some("Calibri"));
        assert_eq!(size, Some(1100));
        assert_eq!(bold, Some(true));
    }

    #[test]
    fn theme_reference_typeface_passes_through() {
        // A `+mn-lt` theme reference is returned verbatim (the renderer resolves
        // it against the theme font scheme).
        let xml = format!(
            r#"<c:valAx xmlns:c="{C_NS}" xmlns:a="{A_NS}">
                 <c:txPr><a:p><a:pPr><a:defRPr><a:latin typeface="+mn-lt"/></a:defRPr></a:pPr></a:p></c:txPr>
               </c:valAx>"#
        );
        assert_eq!(
            extract_axis_tick_label_face(root_of(&xml).root_element()).as_deref(),
            Some("+mn-lt")
        );
    }

    // ── Axis scale model (CH6) ──────────────────────────────────────────────

    #[test]
    fn axis_gridlines_presence() {
        // Value axis with `<c:majorGridlines>` → true; category axis without → false.
        let val = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:majorGridlines/></c:valAx>"#);
        assert!(axis_has_major_gridlines(root_of(&val).root_element()));
        assert!(!axis_has_minor_gridlines(root_of(&val).root_element()));

        let cat = format!(r#"<c:catAx xmlns:c="{C_NS}"/>"#);
        assert!(!axis_has_major_gridlines(root_of(&cat).root_element()));

        let both = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:majorGridlines/><c:minorGridlines/></c:valAx>"#
        );
        assert!(axis_has_major_gridlines(root_of(&both).root_element()));
        assert!(axis_has_minor_gridlines(root_of(&both).root_element()));
    }

    #[test]
    fn axis_major_minor_unit() {
        let xml = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:crossBetween val="between"/><c:majorUnit val="500"/><c:minorUnit val="100"/></c:valAx>"#
        );
        assert_eq!(
            extract_axis_major_unit(root_of(&xml).root_element()),
            Some(500.0)
        );
        assert_eq!(
            extract_axis_minor_unit(root_of(&xml).root_element()),
            Some(100.0)
        );
        // Absent → None (auto step).
        let bare = format!(r#"<c:valAx xmlns:c="{C_NS}"/>"#);
        assert_eq!(extract_axis_major_unit(root_of(&bare).root_element()), None);
        // Non-positive rejected (would wedge the gridline loop).
        let zero = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:majorUnit val="0"/></c:valAx>"#);
        assert_eq!(extract_axis_major_unit(root_of(&zero).root_element()), None);
    }

    #[test]
    fn axis_log_base() {
        let xml = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:scaling><c:logBase val="10"/></c:scaling></c:valAx>"#
        );
        assert_eq!(
            extract_axis_log_base(root_of(&xml).root_element()),
            Some(10.0)
        );
        // Base < 2 is invalid per ST_LogBase → rejected.
        let bad = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:scaling><c:logBase val="1"/></c:scaling></c:valAx>"#
        );
        assert_eq!(extract_axis_log_base(root_of(&bad).root_element()), None);
        // Absent scaling / logBase → None (linear).
        let bare = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:scaling/></c:valAx>"#);
        assert_eq!(extract_axis_log_base(root_of(&bare).root_element()), None);
    }

    #[test]
    fn axis_orientation() {
        let rev = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:scaling><c:orientation val="maxMin"/></c:scaling></c:valAx>"#
        );
        assert_eq!(
            extract_axis_orientation(root_of(&rev).root_element()).as_deref(),
            Some("maxMin")
        );
        let norm = format!(
            r#"<c:valAx xmlns:c="{C_NS}"><c:scaling><c:orientation val="minMax"/></c:scaling></c:valAx>"#
        );
        assert_eq!(
            extract_axis_orientation(root_of(&norm).root_element()).as_deref(),
            Some("minMax")
        );
        // Absent → None (renderer treats as minMax).
        let bare = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:scaling/></c:valAx>"#);
        assert_eq!(
            extract_axis_orientation(root_of(&bare).root_element()),
            None
        );
    }

    #[test]
    fn axis_tick_label_pos_and_rotation() {
        let xml = format!(
            r#"<c:catAx xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:tickLblPos val="low"/><c:txPr><a:bodyPr rot="-2700000"/></c:txPr></c:catAx>"#
        );
        assert_eq!(
            extract_axis_tick_label_pos(root_of(&xml).root_element()).as_deref(),
            Some("low")
        );
        assert_eq!(
            extract_axis_tick_label_rotation(root_of(&xml).root_element()),
            Some(-2_700_000)
        );
        // Absent → None (renderer treats as nextTo / 0°).
        let bare = format!(r#"<c:catAx xmlns:c="{C_NS}"/>"#);
        assert_eq!(
            extract_axis_tick_label_pos(root_of(&bare).root_element()),
            None
        );
        assert_eq!(
            extract_axis_tick_label_rotation(root_of(&bare).root_element()),
            None
        );
    }

    #[test]
    fn series_trendlines_parse() {
        // No trendline → None (byte-stable).
        let none = format!(r#"<c:ser xmlns:c="{C_NS}"/>"#);
        assert_eq!(
            extract_series_trendlines(root_of(&none).root_element(), &StubResolver),
            None
        );
        // A linear fit + a period-3 moving average, the linear one with a red line.
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
                 <c:trendline>
                   <c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="ff0000"/></a:solidFill></a:ln></c:spPr>
                   <c:trendlineType val="linear"/>
                   <c:dispEq val="1"/>
                   <c:dispRSqr val="1"/>
                 </c:trendline>
                 <c:trendline>
                   <c:trendlineType val="movingAvg"/>
                   <c:period val="3"/>
                 </c:trendline>
               </c:ser>"#
        );
        let got = extract_series_trendlines(root_of(&xml).root_element(), &StubResolver).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].trendline_type, "linear");
        assert_eq!(got[0].line_color.as_deref(), Some("FF0000"));
        assert_eq!(got[0].line_width_emu, Some(19050));
        assert_eq!(got[0].disp_eq, Some(true));
        assert_eq!(got[0].disp_r_sqr, Some(true));
        assert_eq!(got[1].trendline_type, "movingAvg");
        assert_eq!(got[1].period, Some(3));
        assert_eq!(got[1].line_color, None);
    }

    // ========================================================================
    // `parse_chart_part` direct contract tests.
    //
    // These call the shared entry point itself (not the per-crate wrappers),
    // so a future edit that breaks the parse — in this PR or in PR2 (xlsx
    // switch) / PR3 (chartEx) — fails here first. Each test asserts concrete
    // output values (chart-type strings, hex colors, font names, axis
    // presence/absence) rather than just "parses without panicking", so the
    // parse *contract* is pinned, not merely its shape.

    /// Minimal theme-aware resolver for `parse_chart_part` tests: resolves
    /// `<a:srgbClr>` verbatim (uppercased, matching real resolvers' hex
    /// normalization) and `<a:schemeClr>` against a small fixed table covering
    /// the slots real decks use for chart text/borders. Also overrides the
    /// theme major/minor Latin font hooks so CH10 theme-fallback fields can be
    /// exercised without pulling in a crate's full theme parser.
    struct FixtureResolver;

    impl ColorResolver for FixtureResolver {
        fn resolve_solid_fill(&self, node: Node) -> Option<String> {
            let c = node.children().find(|n| {
                n.is_element() && matches!(n.tag_name().name(), "srgbClr" | "schemeClr")
            })?;
            match c.tag_name().name() {
                "srgbClr" => c.attribute("val").map(|v| v.to_uppercase()),
                "schemeClr" => match c.attribute("val")? {
                    "accent1" => Some("4472C4".to_string()),
                    "accent2" => Some("ED7D31".to_string()),
                    "accent3" => Some("A5A5A5".to_string()),
                    "tx1" | "dk1" => Some("000000".to_string()),
                    "bg1" | "lt1" => Some("FFFFFF".to_string()),
                    _ => None,
                },
                _ => None,
            }
        }

        fn theme_major_font_latin(&self) -> Option<String> {
            Some("Calibri Light".to_string())
        }

        fn theme_minor_font_latin(&self) -> Option<String> {
            Some("Calibri".to_string())
        }

        fn resolve_series_accent(&self, idx: usize) -> Option<String> {
            // Cycle a 6-accent palette exactly like the docx resolver so chartEx
            // box/sunburst tests can assert the branch/series colors.
            const ACCENTS: [&str; 6] = ["5B9BD5", "ED7D31", "A5A5A5", "FFC000", "4472C4", "70AD47"];
            Some(ACCENTS[idx % 6].to_string())
        }
    }

    fn chart_space_of(xml: &str) -> Document<'_> {
        Document::parse(xml).expect("parse chartSpace fixture")
    }

    #[test]
    fn ct_boolean_auto_title_deleted_bare_suppresses_fallback_title() {
        // §21.2.2.7 `<c:autoTitleDeleted/>` ⇒ true ⇒ the single-series name is
        // NOT promoted to a fallback chart title. A bare element must read true;
        // its absence (control) leaves the auto title, so the series name shows.
        let with_bare = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:chart>
                <c:autoTitleDeleted/>
                <c:plotArea><c:lineChart>
                  <c:ser><c:idx val="0"/><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>OnlySeries</c:v></c:pt></c:strCache></c:strRef></c:tx>
                    <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
                </c:lineChart></c:plotArea>
              </c:chart></c:chartSpace>"#
        );
        let d = chart_space_of(&with_bare);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("chart");
        assert_eq!(
            m.title, None,
            "bare <c:autoTitleDeleted/> ⇒ no fallback title"
        );

        let without = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:chart>
                <c:plotArea><c:lineChart>
                  <c:ser><c:idx val="0"/><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>OnlySeries</c:v></c:pt></c:strCache></c:strRef></c:tx>
                    <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
                </c:lineChart></c:plotArea>
              </c:chart></c:chartSpace>"#
        );
        let d2 = chart_space_of(&without);
        let m2 = parse_chart_part(d2.root_element(), &FixtureResolver).expect("chart");
        assert_eq!(
            m2.title.as_deref(),
            Some("OnlySeries"),
            "no autoTitleDeleted ⇒ series name is the fallback title"
        );
    }

    #[test]
    fn ct_boolean_chart_level_marker_bare_enables_markers() {
        // §21.2.2.33 `<c:lineChart><c:marker/>` ⇒ true ⇒ line series show markers
        // even without a per-series `<c:marker>`. A bare element must read true.
        let bare = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:chart>
                <c:plotArea><c:lineChart>
                  <c:marker/>
                  <c:ser><c:idx val="0"/>
                    <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
                </c:lineChart></c:plotArea>
              </c:chart></c:chartSpace>"#
        );
        let d = chart_space_of(&bare);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("chart");
        assert_eq!(
            m.series[0].show_marker,
            Some(true),
            "bare <c:marker/> ⇒ markers enabled"
        );

        // Control: `<c:marker val="0"/>` ⇒ markers off for a line series.
        let off = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}"><c:chart>
                <c:plotArea><c:lineChart>
                  <c:marker val="0"/>
                  <c:ser><c:idx val="0"/>
                    <c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser>
                </c:lineChart></c:plotArea>
              </c:chart></c:chartSpace>"#
        );
        let d2 = chart_space_of(&off);
        let m2 = parse_chart_part(d2.root_element(), &FixtureResolver).expect("chart");
        assert_eq!(
            m2.series[0].show_marker,
            Some(false),
            "<c:marker val=\"0\"/> ⇒ markers off"
        );
    }

    /// (a) Bar chart with the full decoration set: title (size/bold/color),
    /// legend, styled category + value axes, gap/overlap, chartSpace border,
    /// and value-axis major gridlines. Every field asserted here is a distinct
    /// probe `parse_chart_part` wires up; a regression in any one shows here
    /// without needing a full-document golden diff.
    #[test]
    fn parse_chart_part_bar_full_decoration() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart>
                <c:title><c:tx><c:rich>
                  <a:p><a:pPr><a:defRPr sz="1800" b="1"><a:solidFill><a:srgbClr val="1b4332"/></a:solidFill></a:defRPr></a:pPr>
                  <a:r><a:t>Quarterly Revenue</a:t></a:r></a:p>
                </c:rich></c:tx></c:title>
                <c:plotArea>
                  <c:barChart>
                    <c:barDir val="col"/>
                    <c:grouping val="clustered"/>
                    <c:gapWidth val="80"/>
                    <c:overlap val="-10"/>
                    <c:ser>
                      <c:idx val="0"/>
                      <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Revenue</c:v></c:pt></c:strCache></c:strRef></c:tx>
                      <c:spPr><a:solidFill><a:srgbClr val="2d6a4f"/></a:solidFill></c:spPr>
                      <c:cat><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt></c:strCache></c:cat>
                      <c:val><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:val>
                    </c:ser>
                    <c:axId val="1"/>
                    <c:axId val="2"/>
                  </c:barChart>
                  <c:catAx>
                    <c:axId val="1"/>
                    <c:axPos val="b"/>
                    <c:spPr><a:ln><a:solidFill><a:srgbClr val="808080"/></a:solidFill></a:ln></c:spPr>
                  </c:catAx>
                  <c:valAx>
                    <c:axId val="2"/>
                    <c:axPos val="l"/>
                    <c:majorGridlines><c:spPr><a:ln w="3175"><a:solidFill><a:schemeClr val="accent3"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>
                    <c:scaling><c:min val="0"/><c:max val="30"/></c:scaling>
                  </c:valAx>
                </c:plotArea>
                <c:legend><c:legendPos val="b"/></c:legend>
              </c:chart>
              <c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="1b4332"/></a:solidFill></a:ln></c:spPr>
            </c:chartSpace>"#
        );
        let doc = chart_space_of(&xml);
        let m = parse_chart_part(doc.root_element(), &FixtureResolver).expect("bar chart parses");

        assert_eq!(m.chart_type, "clusteredBar");
        assert_eq!(m.title.as_deref(), Some("Quarterly Revenue"));
        assert_eq!(m.title_font_size_hpt, Some(1800));
        assert_eq!(m.title_font_bold, Some(true));
        // `parse_chart_part` now resolves the title's `<a:solidFill>` via the
        // `ColorResolver` (schemeClr resolution was added — see
        // `extract_chart_title_color`). The fixture's title carries
        // `<a:srgbClr val="1b4332">`, which the resolver returns uppercased.
        // (This assertion previously pinned `None` as a known limitation; the
        // limitation is now fixed, so the expected value flips to the resolved
        // hex — a deliberate, visible contract change.)
        assert_eq!(m.title_font_color.as_deref(), Some("1B4332"));
        assert_eq!(m.categories, vec!["Q1".to_string(), "Q2".to_string()]);
        assert_eq!(m.series.len(), 1);
        assert_eq!(m.series[0].name, "Revenue");
        assert_eq!(m.series[0].values, vec![Some(10.0), Some(20.0)]);
        assert_eq!(m.series[0].color.as_deref(), Some("2D6A4F"));
        assert!(m.show_legend);
        assert_eq!(m.legend_pos.as_deref(), Some("b"));
        assert_eq!(m.bar_gap_width, Some(80));
        assert_eq!(m.bar_overlap, Some(-10));
        assert_eq!(m.val_min, Some(0.0));
        assert_eq!(m.val_max, Some(30.0));
        assert_eq!(m.val_axis_major_gridlines, Some(true));
        assert_eq!(m.cat_axis_major_gridlines, Some(false));
        // The value-axis `<c:majorGridlines><c:spPr><a:ln>` carries an explicit
        // `accent3` colour (resolver → A5A5A5) and a 3175 EMU (0.25 pt) width.
        assert_eq!(m.val_axis_gridline_color.as_deref(), Some("A5A5A5"));
        assert_eq!(m.val_axis_gridline_width_emu, Some(3175));
        // The category axis has no gridlines element → no gridline style.
        assert_eq!(m.cat_axis_gridline_color, None);
        assert_eq!(m.cat_axis_gridline_width_emu, None);
        assert_eq!(m.cat_axis_line_color.as_deref(), Some("808080"));
        // `extract_chart_space_border` reads `<a:srgbClr@val>` directly (not
        // through the resolver, unlike series/title colors), so the case is
        // whatever the XML wrote — lowercase here — not uppercased.
        assert_eq!(m.chart_border_color.as_deref(), Some("1b4332"));
        assert_eq!(m.chart_border_width_emu, Some(19050));
        assert!(!m.cat_axis_hidden);
        assert!(!m.val_axis_hidden);
    }

    /// (b) Combo chart: a bar series on the primary value axis plus a line
    /// series bound to a SECONDARY value axis (`axPos="r"`). Verifies the
    /// series↔axId binding produces `series_type: "line"` and
    /// `use_secondary_axis: true` on the line series only, and that
    /// `secondary_val_axis` is populated from the right-hand `<c:valAx>`.
    #[test]
    fn parse_chart_part_combo_with_secondary_axis() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart><c:plotArea>
                <c:barChart>
                  <c:barDir val="col"/>
                  <c:grouping val="clustered"/>
                  <c:ser>
                    <c:idx val="0"/>
                    <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Units</c:v></c:pt></c:strCache></c:strRef></c:tx>
                    <c:cat><c:strCache><c:pt idx="0"><c:v>Jan</c:v></c:pt><c:pt idx="1"><c:v>Feb</c:v></c:pt></c:strCache></c:cat>
                    <c:val><c:numCache><c:pt idx="0"><c:v>5</c:v></c:pt><c:pt idx="1"><c:v>7</c:v></c:pt></c:numCache></c:val>
                  </c:ser>
                  <c:axId val="1"/>
                  <c:axId val="2"/>
                </c:barChart>
                <c:lineChart>
                  <c:grouping val="standard"/>
                  <c:ser>
                    <c:idx val="1"/>
                    <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Margin %</c:v></c:pt></c:strCache></c:strRef></c:tx>
                    <c:cat><c:strCache><c:pt idx="0"><c:v>Jan</c:v></c:pt><c:pt idx="1"><c:v>Feb</c:v></c:pt></c:strCache></c:cat>
                    <c:val><c:numCache><c:pt idx="0"><c:v>0.3</c:v></c:pt><c:pt idx="1"><c:v>0.4</c:v></c:pt></c:numCache></c:val>
                  </c:ser>
                  <c:axId val="1"/>
                  <c:axId val="3"/>
                </c:lineChart>
                <c:catAx><c:axId val="1"/><c:axPos val="b"/></c:catAx>
                <c:valAx>
                  <c:axId val="2"/>
                  <c:axPos val="l"/>
                  <c:crosses val="autoZero"/>
                </c:valAx>
                <c:valAx>
                  <c:axId val="3"/>
                  <c:axPos val="r"/>
                  <c:crosses val="max"/>
                  <c:scaling><c:min val="0"/><c:max val="1"/></c:scaling>
                  <c:majorUnit val="0.25"/>
                  <c:title><c:tx><c:rich><a:p><a:r><a:t>Margin</a:t></a:r></a:p></c:rich></c:tx></c:title>
                </c:valAx>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        );
        let doc = chart_space_of(&xml);
        let m = parse_chart_part(doc.root_element(), &FixtureResolver).expect("combo chart parses");

        assert_eq!(m.chart_type, "clusteredBar");
        assert_eq!(m.series.len(), 2);

        let bar_series = &m.series[0];
        assert_eq!(bar_series.name, "Units");
        // Every series now carries its chart-group type (the renderer keys line
        // vs. non-line off this; a bar series is `Some("bar")`, treated as
        // non-line, identical in rendering to the old `None`).
        assert_eq!(bar_series.series_type.as_deref(), Some("bar"));
        assert_eq!(bar_series.use_secondary_axis, None);

        let line_series = &m.series[1];
        assert_eq!(line_series.name, "Margin %");
        assert_eq!(line_series.series_type.as_deref(), Some("line"));
        assert_eq!(line_series.use_secondary_axis, Some(true));

        let sec = m.secondary_val_axis.expect("secondary axis populated");
        assert_eq!(sec.min, Some(0.0));
        assert_eq!(sec.max, Some(1.0));
        assert_eq!(sec.title.as_deref(), Some("Margin"));
        assert!(!sec.hidden);
        // #738: an explicit `<c:majorUnit>` on the secondary axis (§21.2.2.103)
        // is threaded into the model (was silently dropped before).
        assert_eq!(sec.major_unit, Some(0.25));
        // The primary value axis declared no majorUnit → stays None.
        assert_eq!(m.val_axis_major_unit, None);
    }

    /// (c) Doughnut chart with per-point `<c:dPt>` colors, `showPercent`, and
    /// `holeSize`/`firstSliceAng`. Doughnut (not pie) is used because
    /// `extract_hole_size` only ever matches a `<c:doughnutChart>` — a pie
    /// fixture would leave `hole_size` permanently `None`.
    #[test]
    fn parse_chart_part_doughnut_dpt_colors_and_geometry() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart><c:plotArea>
                <c:doughnutChart>
                  <c:holeSize val="45"/>
                  <c:firstSliceAng val="90"/>
                  <c:ser>
                    <c:idx val="0"/>
                    <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Share</c:v></c:pt></c:strCache></c:strRef></c:tx>
                    <c:dPt><c:idx val="0"/><c:spPr><a:solidFill><a:srgbClr val="ff0000"/></a:solidFill></c:spPr></c:dPt>
                    <c:dPt><c:idx val="1"/><c:spPr><a:solidFill><a:srgbClr val="00ff00"/></a:solidFill></c:spPr></c:dPt>
                    <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:cat>
                    <c:val><c:numCache><c:pt idx="0"><c:v>60</c:v></c:pt><c:pt idx="1"><c:v>40</c:v></c:pt></c:numCache></c:val>
                    <c:dLbls><c:showPercent val="1"/></c:dLbls>
                  </c:ser>
                </c:doughnutChart>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        );
        let doc = chart_space_of(&xml);
        let m =
            parse_chart_part(doc.root_element(), &FixtureResolver).expect("doughnut chart parses");

        assert_eq!(m.chart_type, "doughnut");
        assert_eq!(m.hole_size, Some(45));
        assert_eq!(m.first_slice_angle, Some(90));
        assert!(m.show_data_labels);

        let colors = m.series[0]
            .data_point_colors
            .as_ref()
            .expect("dPt colors populated");
        assert_eq!(colors[0].as_deref(), Some("FF0000"));
        assert_eq!(colors[1].as_deref(), Some("00FF00"));
    }

    /// (d) A date-category axis (`<c:dateAx>` instead of `<c:catAx>`) combined
    /// with `<c:date1904/>`. `parse_chart_part` treats `dateAx` identically to
    /// `catAx` for every cat-axis probe (hidden/format-code/etc.) — this pins
    /// that the dateAx path is actually reached (not silently skipped because
    /// the finder only looked for `catAx`).
    #[test]
    fn parse_chart_part_date_axis_and_date1904() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:date1904/>
              <c:chart><c:plotArea>
                <c:lineChart>
                  <c:grouping val="standard"/>
                  <c:ser>
                    <c:idx val="0"/>
                    <c:tx><c:v>Temp</c:v></c:tx>
                    <c:cat><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:cat>
                    <c:val><c:numCache><c:pt idx="0"><c:v>21</c:v></c:pt><c:pt idx="1"><c:v>23</c:v></c:pt></c:numCache></c:val>
                  </c:ser>
                </c:lineChart>
                <c:dateAx>
                  <c:axPos val="b"/>
                  <c:numFmt formatCode="m/d/yyyy"/>
                </c:dateAx>
                <c:valAx><c:axPos val="l"/></c:valAx>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        );
        let doc = chart_space_of(&xml);
        let m =
            parse_chart_part(doc.root_element(), &FixtureResolver).expect("dateAx chart parses");

        assert_eq!(m.chart_type, "line");
        assert!(m.date1904);
        assert_eq!(m.cat_axis_format_code.as_deref(), Some("m/d/yyyy"));
        assert!(!m.cat_axis_hidden);
    }

    /// (e) `chart_type` normalization for stacked/percentStacked. As of CH13,
    /// `parse_chart_part` routes bar/line/area type detection through the shared
    /// `canonical_chart_type` helper (previously an inline match duplicated the
    /// logic and — as a latent bug — folded a percentStacked BAR down to plain
    /// `stackedBar`, so the renderer's `stackedBarPct` 100%-normalization never
    /// fired for a parsed chart). It now distinguishes the percent variant for
    /// BAR (`stackedBarPct` / `stackedBarHPct`) and AREA (`stackedAreaPct`),
    /// matching the LINE behavior and the standalone helper's own matrix test.
    #[test]
    fn parse_chart_part_stacked_percent_stacked_chart_type() {
        fn bar_chart_type(grouping: &str, bar_dir: &str) -> String {
            let xml = format!(
                r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea>
                  <c:barChart>
                    <c:barDir val="{bar_dir}"/>
                    <c:grouping val="{grouping}"/>
                    <c:ser><c:idx val="0"/>
                      <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt></c:strCache></c:cat>
                      <c:val><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:val>
                    </c:ser>
                  </c:barChart>
                </c:plotArea></c:chart></c:chartSpace>"#
            );
            let doc = chart_space_of(&xml);
            parse_chart_part(doc.root_element(), &FixtureResolver)
                .unwrap()
                .chart_type
        }
        fn line_chart_type(grouping: &str) -> String {
            let xml = format!(
                r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea>
                  <c:lineChart>
                    <c:grouping val="{grouping}"/>
                    <c:ser><c:idx val="0"/>
                      <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt></c:strCache></c:cat>
                      <c:val><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:val>
                    </c:ser>
                  </c:lineChart>
                </c:plotArea></c:chart></c:chartSpace>"#
            );
            let doc = chart_space_of(&xml);
            parse_chart_part(doc.root_element(), &FixtureResolver)
                .unwrap()
                .chart_type
        }

        assert_eq!(bar_chart_type("stacked", "col"), "stackedBar");
        // CH13: percentStacked now maps to the Pct canonical variant (the
        // renderer normalizes those to 100%), fixing the prior fold-to-stacked.
        assert_eq!(bar_chart_type("percentStacked", "col"), "stackedBarPct");
        assert_eq!(bar_chart_type("percentStacked", "bar"), "stackedBarHPct");
        // Line + area also distinguish percentStacked.
        assert_eq!(line_chart_type("percentStacked"), "stackedLinePct");
    }

    /// (f) Two structural "not a chart" shapes: no `<c:plotArea>` at all, and
    /// a `<c:plotArea>` present but declaring zero `<c:ser>` series. Both must
    /// return `None` rather than an empty/degenerate `ChartModel`.
    #[test]
    fn parse_chart_part_returns_none_for_missing_plot_area_or_empty_series() {
        let no_plot_area = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:title/></c:chart></c:chartSpace>"#
        );
        assert!(parse_chart_part(
            chart_space_of(&no_plot_area).root_element(),
            &FixtureResolver
        )
        .is_none());

        let empty_series = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea>
                <c:barChart><c:barDir val="col"/><c:grouping val="clustered"/></c:barChart>
              </c:plotArea></c:chart></c:chartSpace>"#
        );
        assert!(parse_chart_part(
            chart_space_of(&empty_series).root_element(),
            &FixtureResolver
        )
        .is_none());
    }

    /// §21.2.2.198: a scatter series whose `<c:spPr><a:ln>` is `<a:noFill/>`
    /// has its connecting line turned OFF, overriding the group-level
    /// `<c:scatterStyle val="lineMarker">` (§21.2.2.42). The parser must set
    /// `line_hidden = Some(true)` so the renderer draws markers only — the
    /// sample-30 sheet-1 scatter shape. A series with a paintable line leaves
    /// `line_hidden = None`.
    #[test]
    fn parse_chart_part_scatter_series_line_nofill_sets_line_hidden() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart><c:plotArea>
                <c:scatterChart>
                  <c:scatterStyle val="lineMarker"/>
                  <c:ser>
                    <c:idx val="0"/>
                    <c:spPr><a:ln w="25400"><a:noFill/></a:ln></c:spPr>
                    <c:xVal><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:xVal>
                    <c:yVal><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt><c:pt idx="1"><c:v>20</c:v></c:pt></c:numCache></c:yVal>
                  </c:ser>
                  <c:ser>
                    <c:idx val="1"/>
                    <c:spPr><a:ln w="19050"><a:solidFill><a:srgbClr val="ED7D31"/></a:solidFill></a:ln></c:spPr>
                    <c:xVal><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:xVal>
                    <c:yVal><c:numCache><c:pt idx="0"><c:v>5</c:v></c:pt><c:pt idx="1"><c:v>8</c:v></c:pt></c:numCache></c:yVal>
                  </c:ser>
                  <c:axId val="1"/><c:axId val="2"/>
                </c:scatterChart>
                <c:valAx><c:axId val="1"/><c:crossAx val="2"/></c:valAx>
                <c:valAx><c:axId val="2"/><c:crossAx val="1"/></c:valAx>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        );
        let m = parse_chart_part(chart_space_of(&xml).root_element(), &FixtureResolver)
            .expect("scatter chart parses");
        assert_eq!(m.chart_type, "scatter");
        assert_eq!(m.scatter_style.as_deref(), Some("lineMarker"));
        // Series 0: explicit `<a:noFill/>` line → line_hidden set.
        assert_eq!(
            m.series[0].line_hidden,
            Some(true),
            "a `<a:ln><a:noFill/>` series line must record line_hidden"
        );
        // Series 1: a paintable line → line_hidden stays None (group style governs).
        assert_eq!(
            m.series[1].line_hidden, None,
            "a series with a solid line must NOT set line_hidden"
        );
    }

    /// §21.2.2.47: a per-point `<c:dLbl>` carries its own show-flag group and
    /// text style, overriding the series-level `<c:dLbls>` (§21.2.2.49) for that
    /// point. sample-14 slide-7's pie sets `showCatName=0 showPercent=1` + white
    /// per slice while the series default is `showCatName=1` black. The parser
    /// must surface both the series default AND the per-point flag / color
    /// overrides, and mark a genuine `<c:delete>` distinctly from a style-only
    /// `<c:dLbl>` (which has an empty `text`).
    #[test]
    fn parse_chart_part_pie_per_point_dlbl_overrides_series_defaults() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart><c:plotArea>
                <c:pieChart>
                  <c:ser>
                    <c:idx val="0"/>
                    <c:dLbls>
                      <c:dLbl>
                        <c:idx val="0"/>
                        <c:txPr><a:p><a:pPr><a:defRPr b="1"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr>
                        <c:showVal val="0"/><c:showCatName val="0"/><c:showSerName val="0"/><c:showPercent val="1"/>
                      </c:dLbl>
                      <c:dLbl>
                        <c:idx val="1"/>
                        <c:delete val="1"/>
                      </c:dLbl>
                      <c:txPr><a:p><a:pPr><a:defRPr b="1"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:defRPr></a:pPr></a:p></c:txPr>
                      <c:showVal val="0"/><c:showCatName val="1"/><c:showSerName val="0"/><c:showPercent val="1"/>
                    </c:dLbls>
                    <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:cat>
                    <c:val><c:numCache><c:pt idx="0"><c:v>60</c:v></c:pt><c:pt idx="1"><c:v>40</c:v></c:pt></c:numCache></c:val>
                  </c:ser>
                </c:pieChart>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        );
        let m = parse_chart_part(chart_space_of(&xml).root_element(), &FixtureResolver)
            .expect("pie chart parses");
        assert_eq!(m.chart_type, "pie");
        let def = m.series[0]
            .series_data_labels
            .as_ref()
            .expect("series-level dLbls present");
        // Series default: category name ON, black text.
        assert!(def.show_cat_name, "series default shows category name");
        assert!(def.show_percent);
        assert_eq!(def.font_color.as_deref(), Some("000000"));

        let ovs = m.series[0]
            .data_label_overrides
            .as_ref()
            .expect("per-point overrides present");
        let ov0 = ovs.iter().find(|o| o.idx == 0).expect("idx 0 override");
        // Per-point idx 0: category name OFF, percent ON, white — overrides the
        // series default so this slice renders as white percent-only.
        assert_eq!(ov0.show_cat_name, Some(false));
        assert_eq!(ov0.show_percent, Some(true));
        assert_eq!(ov0.font_color.as_deref(), Some("FFFFFF"));
        assert_ne!(ov0.deleted, Some(true), "a style-only dLbl is not a delete");

        let ov1 = ovs.iter().find(|o| o.idx == 1).expect("idx 1 override");
        // Per-point idx 1: a genuine `<c:delete>` → flagged so the renderer skips it.
        assert_eq!(ov1.deleted, Some(true));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Direct unit tests for the extractors moved from the xlsx parser into
    // this shared module. These call the functions themselves (not through
    // `parse_chart_part`) so a regression in one is pinpointed rather than
    // surfacing only as a diff in a much larger golden `ChartModel`.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn parse_marker_block_symbol_size_fill_line() {
        let xml = format!(
            r#"<c:marker xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:symbol val="circle"/>
              <c:size val="6"/>
              <c:spPr>
                <a:solidFill><a:srgbClr val="ff0000"/></a:solidFill>
                <a:ln><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:ln>
              </c:spPr>
            </c:marker>"#
        );
        let d = root_of(&xml);
        let (symbol, size, fill, line) =
            parse_marker_block(Some(d.root_element()), &FixtureResolver);
        assert_eq!(symbol.as_deref(), Some("circle"));
        assert_eq!(size, Some(6.0));
        assert_eq!(fill.as_deref(), Some("FF0000"));
        assert_eq!(line.as_deref(), Some("4472C4"));
    }

    #[test]
    fn parse_marker_block_none_node_returns_all_none() {
        assert_eq!(
            parse_marker_block(None, &FixtureResolver),
            (None, None, None, None)
        );
    }

    #[test]
    fn parse_marker_block_symbol_none_no_sppr() {
        let xml = format!(r#"<c:marker xmlns:c="{C_NS}"><c:symbol val="none"/></c:marker>"#);
        let d = root_of(&xml);
        let (symbol, size, fill, line) =
            parse_marker_block(Some(d.root_element()), &FixtureResolver);
        assert_eq!(symbol.as_deref(), Some("none"));
        assert_eq!(size, None);
        assert_eq!(fill, None);
        assert_eq!(line, None);
    }

    #[test]
    fn parse_error_bars_fixed_val_both_directions() {
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:errBars>
                <c:errDir val="y"/>
                <c:errBarType val="both"/>
                <c:errValType val="fixedVal"/>
                <c:val val="2.5"/>
                <c:spPr><a:ln w="12700"><a:solidFill><a:srgbClr val="333333"/></a:solidFill><a:prstDash val="dash"/></a:ln></c:spPr>
              </c:errBars>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let values = vec![Some(10.0), Some(20.0), None];
        let bars = parse_error_bars(d.root_element(), &values, &FixtureResolver);
        assert_eq!(bars.len(), 1);
        let b = &bars[0];
        assert_eq!(b.dir, "y");
        assert_eq!(b.bar_type, "both");
        assert_eq!(b.plus, vec![Some(2.5), Some(2.5), Some(2.5)]);
        assert_eq!(b.minus, vec![Some(2.5), Some(2.5), Some(2.5)]);
        assert!(!b.no_end_cap);
        assert_eq!(b.color.as_deref(), Some("333333"));
        assert_eq!(b.line_width_emu, Some(12700));
        assert_eq!(b.dash.as_deref(), Some("dash"));
    }

    #[test]
    fn parse_error_bars_percentage_scales_per_point() {
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}">
              <c:errBars>
                <c:errDir val="x"/>
                <c:errBarType val="plus"/>
                <c:errValType val="percentage"/>
                <c:val val="10"/>
                <c:noEndCap val="1"/>
              </c:errBars>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let values = vec![Some(100.0), Some(-50.0), None];
        let bars = parse_error_bars(d.root_element(), &values, &FixtureResolver);
        assert_eq!(bars.len(), 1);
        let b = &bars[0];
        assert_eq!(b.dir, "x");
        assert!(b.no_end_cap);
        // 10% of |value|; the None slot stays None (nothing to scale).
        assert_eq!(b.plus, vec![Some(10.0), Some(5.0), None]);
        assert_eq!(b.minus, vec![Some(10.0), Some(5.0), None]);
    }

    #[test]
    fn parse_error_bars_absent_returns_empty() {
        let xml = format!(r#"<c:ser xmlns:c="{C_NS}"><c:val/></c:ser>"#);
        let d = root_of(&xml);
        assert!(parse_error_bars(d.root_element(), &[], &FixtureResolver).is_empty());
    }

    #[test]
    fn parse_series_data_labels_defaults_and_per_point_override() {
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:dLbls>
                <c:numFmt formatCode="0.0%"/>
                <c:dLbl>
                  <c:idx val="1"/>
                  <c:tx><c:rich><a:p><a:r><a:t>Custom</a:t></a:r></a:p></c:rich></c:tx>
                  <c:dLblPos val="outEnd"/>
                </c:dLbl>
                <c:showVal val="1"/>
                <c:showCatName val="0"/>
                <c:showSerName val="0"/>
                <c:showPercent val="1"/>
                <c:dLblPos val="ctr"/>
              </c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (defaults, overrides) =
            parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        let defaults = defaults.expect("series-level dLbls present");
        assert!(defaults.show_val);
        assert!(!defaults.show_cat_name);
        assert!(!defaults.show_ser_name);
        assert!(defaults.show_percent);
        assert_eq!(defaults.position.as_deref(), Some("ctr"));
        assert_eq!(defaults.format_code.as_deref(), Some("0.0%"));

        assert_eq!(overrides.len(), 1);
        let o = &overrides[0];
        assert_eq!(o.idx, 1);
        assert_eq!(o.text, "Custom");
        assert_eq!(o.position.as_deref(), Some("outEnd"));
    }

    // ── CT_Boolean bare-element defaults (issue #806) ───────────────────────
    //
    // dml-chart.xsd defines `CT_Boolean` with `val` `default="true"`. Every
    // element typed `CT_Boolean` (delete / show* / showLeaderLines / marker /
    // noEndCap / autoTitleDeleted / …) therefore means TRUE when the element is
    // PRESENT but the `val` attribute is OMITTED. Office always writes an
    // explicit `val="0|1"`, so these probes drive the bare form that only a
    // hand-authored / third-party file emits — the latent divergence the issue
    // flags. `val="0"` must still read false, and an ABSENT element keeps its
    // own semantic default (off).

    #[test]
    fn ct_boolean_axis_delete_bare_element_is_true() {
        // §21.2.2.40 `<c:delete/>` on an axis ⇒ axis deleted (val default true).
        let bare_xml = format!(r#"<c:catAx xmlns:c="{C_NS}"><c:delete/></c:catAx>"#);
        let bare = root_of(&bare_xml);
        assert!(
            axis_is_deleted(bare.root_element()),
            "bare <c:delete/> ⇒ axis deleted"
        );
        let off_xml = format!(r#"<c:catAx xmlns:c="{C_NS}"><c:delete val="0"/></c:catAx>"#);
        let off = root_of(&off_xml);
        assert!(
            !axis_is_deleted(off.root_element()),
            "val=\"0\" ⇒ not deleted"
        );
        let absent_xml = format!(r#"<c:catAx xmlns:c="{C_NS}"/>"#);
        let absent = root_of(&absent_xml);
        assert!(
            !axis_is_deleted(absent.root_element()),
            "no <c:delete> ⇒ axis shown"
        );
    }

    #[test]
    fn ct_boolean_series_dlbl_show_flags_bare_are_true() {
        // §21.2.2.187/.180/.185/.183 series-level show* flags. A bare
        // <c:showVal/> etc. ⇒ true; the shared parser must not collapse it to
        // false. `<c:showSerName val="0"/>` stays false (explicit override).
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:dLbls>
                <c:showVal/>
                <c:showCatName/>
                <c:showSerName val="0"/>
                <c:showPercent/>
              </c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (defaults, _) = parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        let defaults = defaults.expect("dLbls present");
        assert!(defaults.show_val, "bare <c:showVal/> ⇒ true");
        assert!(defaults.show_cat_name, "bare <c:showCatName/> ⇒ true");
        assert!(
            !defaults.show_ser_name,
            "<c:showSerName val=\"0\"/> ⇒ false"
        );
        assert!(defaults.show_percent, "bare <c:showPercent/> ⇒ true");
    }

    #[test]
    fn ct_boolean_series_show_leader_lines_bare_is_true() {
        // §21.2.2.183 `<c:showLeaderLines/>` ⇒ true (val default).
        let cache = std::collections::HashMap::new();
        let bare =
            format!(r#"<c:ser xmlns:c="{C_NS}"><c:dLbls><c:showLeaderLines/></c:dLbls></c:ser>"#);
        let d = root_of(&bare);
        let (defaults, _) = parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        assert!(
            defaults.expect("dLbls present").show_leader_lines,
            "bare <c:showLeaderLines/> ⇒ true"
        );
    }

    #[test]
    fn ct_boolean_per_point_dlbl_bare_delete_and_flags_are_true() {
        // §21.2.2.43 per-point `<c:delete/>` ⇒ that point's label removed.
        // §21.2.2.47 per-point show* ⇒ Some(true) for a bare flag (overrides the
        // series default for that point), Some(false) for val="0".
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:dLbls>
                <c:dLbl>
                  <c:idx val="0"/>
                  <c:delete/>
                </c:dLbl>
                <c:dLbl>
                  <c:idx val="2"/>
                  <c:showPercent/>
                  <c:showCatName val="0"/>
                </c:dLbl>
                <c:showVal val="1"/>
              </c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (_, overrides) = parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        let del = overrides
            .iter()
            .find(|o| o.idx == 0)
            .expect("idx 0 override");
        assert_eq!(
            del.deleted,
            Some(true),
            "bare per-point <c:delete/> ⇒ deleted"
        );
        let flags = overrides
            .iter()
            .find(|o| o.idx == 2)
            .expect("idx 2 override");
        assert_eq!(
            flags.show_percent,
            Some(true),
            "bare <c:showPercent/> ⇒ Some(true)"
        );
        assert_eq!(flags.show_cat_name, Some(false), "val=\"0\" ⇒ Some(false)");
    }

    #[test]
    fn ct_boolean_err_bars_no_end_cap_bare_is_true() {
        // §21.2.2.117 `<c:noEndCap/>` ⇒ true (no I-beam end caps).
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:errBars>
                <c:errBarType val="both"/>
                <c:errValType val="fixedVal"/>
                <c:noEndCap/>
                <c:val val="1"/>
              </c:errBars>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let bars = parse_error_bars(d.root_element(), &[Some(1.0)], &FixtureResolver);
        assert_eq!(bars.len(), 1);
        assert!(bars[0].no_end_cap, "bare <c:noEndCap/> ⇒ true");
    }

    #[test]
    fn parse_series_data_labels_callout_box_and_leader_lines() {
        // Mirror of sample-25 (Word pie callout labels): the series `<c:dLbls>`
        // carries a `<c:spPr>` box (white fill + coloured border), a per-point
        // `<c:dLbl>` with its own box, and `<c:showLeaderLines>` +
        // `<c:leaderLines>` style. All must round-trip into the model.
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:dLbls>
                <c:dLbl>
                  <c:idx val="0"/>
                  <c:spPr>
                    <a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>
                    <a:ln w="12700"><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></a:ln>
                  </c:spPr>
                  <c:showCatName val="1"/>
                  <c:showPercent val="1"/>
                </c:dLbl>
                <c:spPr>
                  <a:solidFill><a:srgbClr val="FEFEFE"/></a:solidFill>
                  <a:ln w="12700"><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></a:ln>
                </c:spPr>
                <c:showVal val="0"/>
                <c:showCatName val="1"/>
                <c:showPercent val="1"/>
                <c:showLeaderLines val="1"/>
                <c:leaderLines>
                  <c:spPr><a:ln w="9525"><a:solidFill><a:srgbClr val="A6A6A6"/></a:solidFill></a:ln></c:spPr>
                </c:leaderLines>
              </c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (defaults, overrides) =
            parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        let defaults = defaults.expect("series-level dLbls present");
        let box_ = defaults.label_box.expect("series callout box");
        assert_eq!(box_.fill.as_deref(), Some("FEFEFE"));
        assert_eq!(box_.border_color.as_deref(), Some("4472C4"));
        assert_eq!(box_.border_width_emu, Some(12700));
        assert!(defaults.show_leader_lines);
        assert_eq!(defaults.leader_line_color.as_deref(), Some("A6A6A6"));
        assert_eq!(defaults.leader_line_width_emu, Some(9525));

        assert_eq!(overrides.len(), 1);
        let o = &overrides[0];
        assert_eq!(o.idx, 0);
        let obox = o.label_box.as_ref().expect("per-point callout box");
        assert_eq!(obox.fill.as_deref(), Some("FFFFFF"));
        assert_eq!(obox.border_color.as_deref(), Some("4472C4"));
        assert_eq!(obox.border_width_emu, Some(12700));
    }

    #[test]
    fn parse_series_data_labels_no_box_leaves_callout_fields_unset() {
        // A plain `<c:dLbls>` with no `<c:spPr>` / leader lines must NOT
        // synthesize a callout box (keeps the historical plain-label path).
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}">
              <c:dLbls><c:showPercent val="1"/></c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (defaults, _) = parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        let defaults = defaults.expect("series-level dLbls present");
        assert!(defaults.label_box.is_none());
        assert!(!defaults.show_leader_lines);
        assert!(defaults.leader_line_color.is_none());
    }

    #[test]
    fn parse_series_data_labels_deleted_point_has_empty_text() {
        let cache = std::collections::HashMap::new();
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}">
              <c:dLbls>
                <c:dLbl><c:idx val="0"/><c:delete val="1"/></c:dLbl>
              </c:dLbls>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let (_, overrides) = parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].text, "");
    }

    #[test]
    fn parse_series_data_labels_absent_returns_none_and_empty() {
        let xml = format!(r#"<c:ser xmlns:c="{C_NS}"></c:ser>"#);
        let d = root_of(&xml);
        let cache = std::collections::HashMap::new();
        let (defaults, overrides) =
            parse_series_data_labels(d.root_element(), &FixtureResolver, &cache);
        assert!(defaults.is_none());
        assert!(overrides.is_empty());
    }

    /// Sparse `<c:pt idx>` cache: `ptCount=11` but only two points are present
    /// (`idx=1` and `idx=9`). The result must be sized to the declared
    /// `ptCount`, not to the number of `<c:pt>` elements present, and every
    /// unlisted index must stay the empty-string placeholder (not shifted).
    #[test]
    fn collect_str_cache_positional_sparse_ptcount_and_gaps() {
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}">
              <c:cat><c:strCache>
                <c:ptCount val="11"/>
                <c:pt idx="1"><c:v>Feb</c:v></c:pt>
                <c:pt idx="9"><c:v>Oct</c:v></c:pt>
              </c:strCache></c:cat>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let cats = collect_str_cache_positional(d.root_element(), "cat");
        assert_eq!(cats.len(), 11);
        assert_eq!(cats[0], "");
        assert_eq!(cats[1], "Feb");
        assert_eq!(cats[9], "Oct");
        assert_eq!(cats[10], "");
    }

    #[test]
    fn collect_str_cache_positional_missing_container_is_empty() {
        let xml = format!(r#"<c:ser xmlns:c="{C_NS}"></c:ser>"#);
        let d = root_of(&xml);
        assert!(collect_str_cache_positional(d.root_element(), "cat").is_empty());
    }

    /// Companion numeric collector: same sparse/idx=1-start shape, but with
    /// `None` gaps instead of empty strings, and one genuinely missing `<c:v>`
    /// (idx present, value absent) which must also collapse to `None`.
    #[test]
    fn collect_num_cache_positional_sparse_ptcount_and_gaps() {
        let xml = format!(
            r#"<c:ser xmlns:c="{C_NS}">
              <c:val><c:numCache>
                <c:ptCount val="11"/>
                <c:pt idx="1"><c:v>42</c:v></c:pt>
                <c:pt idx="9"><c:v>7</c:v></c:pt>
              </c:numCache></c:val>
            </c:ser>"#
        );
        let d = root_of(&xml);
        let vals = collect_num_cache_positional(d.root_element(), "val");
        assert_eq!(vals.len(), 11);
        assert_eq!(vals[0], None);
        assert_eq!(vals[1], Some(42.0));
        assert_eq!(vals[9], Some(7.0));
        assert_eq!(vals[10], None);
    }

    #[test]
    fn collect_num_cache_positional_missing_container_is_empty() {
        let xml = format!(r#"<c:ser xmlns:c="{C_NS}"></c:ser>"#);
        let d = root_of(&xml);
        assert!(collect_num_cache_positional(d.root_element(), "val").is_empty());
    }

    #[test]
    fn extract_radar_style_present_and_absent() {
        let xml = format!(
            r#"<c:radarChart xmlns:c="{C_NS}"><c:radarStyle val="marker"/></c:radarChart>"#
        );
        let d = root_of(&xml);
        assert_eq!(
            extract_radar_style(d.root_element()).as_deref(),
            Some("marker")
        );

        let none_xml = format!(r#"<c:barChart xmlns:c="{C_NS}"></c:barChart>"#);
        let d2 = root_of(&none_xml);
        assert!(extract_radar_style(d2.root_element()).is_none());
    }

    #[test]
    fn extract_axis_crosses_reads_crosses_and_crosses_at() {
        let xml = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:crosses val="max"/></c:valAx>"#);
        let d = root_of(&xml);
        assert_eq!(
            extract_axis_crosses(d.root_element()),
            (Some("max".to_string()), None)
        );

        let xml2 = format!(r#"<c:valAx xmlns:c="{C_NS}"><c:crossesAt val="3.5"/></c:valAx>"#);
        let d2 = root_of(&xml2);
        assert_eq!(extract_axis_crosses(d2.root_element()), (None, Some(3.5)));

        let xml3 = format!(r#"<c:valAx xmlns:c="{C_NS}"></c:valAx>"#);
        let d3 = root_of(&xml3);
        assert_eq!(extract_axis_crosses(d3.root_element()), (None, None));
    }

    #[test]
    fn extract_manual_layout_full_and_defaults() {
        let xml = format!(
            r#"<c:layout xmlns:c="{C_NS}"><c:manualLayout>
              <c:layoutTarget val="inner"/>
              <c:xMode val="edge"/><c:yMode val="edge"/>
              <c:x val="0.1"/><c:y val="0.2"/><c:w val="0.5"/><c:h val="0.6"/>
            </c:manualLayout></c:layout>"#
        );
        let d = root_of(&xml);
        let layout = extract_manual_layout(d.root_element()).expect("manualLayout present");
        assert_eq!(layout.x_mode, "edge");
        assert_eq!(layout.y_mode, "edge");
        assert_eq!(layout.layout_target.as_deref(), Some("inner"));
        assert_eq!(layout.x, 0.1);
        assert_eq!(layout.y, 0.2);
        assert_eq!(layout.w, Some(0.5));
        assert_eq!(layout.h, Some(0.6));
    }

    #[test]
    fn extract_manual_layout_absent_returns_none() {
        let xml = format!(r#"<c:layout xmlns:c="{C_NS}"></c:layout>"#);
        let d = root_of(&xml);
        assert!(extract_manual_layout(d.root_element()).is_none());
    }

    // ── `parse_chartex_part` direct contract tests ──────────────────────────
    //
    // The chartEx counterpart to the `parse_chart_part_*` tests above. These
    // call the shared `parse_chartex_part` (not the pptx wrapper) so a
    // regression in the chartEx structure walk — categories, values, subtotal
    // indices, series/per-label colours (resolved through the `ColorResolver`),
    // axis visibility, gap-width fraction→percent conversion, and the theme
    // fallback faces — is pinpointed here. `FixtureResolver` resolves
    // `<a:schemeClr val="accent1">`→`4472C4`, `tx1`→`000000`, and reports
    // `Calibri Light` / `Calibri` as the theme major/minor faces.

    const CX_NS: &str = "http://schemas.microsoft.com/office/drawing/2014/chartex";
    const CS_NS: &str = "http://schemas.microsoft.com/office/drawing/2012/chartStyle";

    /// (a) Waterfall with the full decoration set: a category dimension, a value
    /// dimension with negatives, `<cx:subtotals>` (idx 0 is implicit, idx 5 is
    /// explicit), a series `<cx:spPr>` fill, per-idx `<cx:dataLabel>` colours
    /// (positives → tx1, negatives → accent1), a hidden value axis, and a
    /// `<cx:catScaling gapWidth="0.8">` fraction (→ legacy 80%). Mirrors the
    /// sample-2 waterfall the golden JSON was captured from.
    #[test]
    fn parse_chartex_part_waterfall_full_contract() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="{A_NS}">
              <cx:chartData>
                <cx:data id="0">
                  <cx:strDim type="cat">
                    <cx:lvl ptCount="4">
                      <cx:pt idx="0">Start</cx:pt>
                      <cx:pt idx="1">Up</cx:pt>
                      <cx:pt idx="2">Down</cx:pt>
                      <cx:pt idx="3">End</cx:pt>
                    </cx:lvl>
                  </cx:strDim>
                  <cx:numDim type="val">
                    <cx:lvl ptCount="4">
                      <cx:pt idx="0">100</cx:pt>
                      <cx:pt idx="1">40</cx:pt>
                      <cx:pt idx="2">-30</cx:pt>
                      <cx:pt idx="3">110</cx:pt>
                    </cx:lvl>
                  </cx:numDim>
                </cx:data>
              </cx:chartData>
              <cx:chart>
                <cx:plotArea>
                  <cx:plotAreaRegion>
                    <cx:series layoutId="waterfall">
                      <cx:spPr><a:solidFill><a:srgbClr val="196eca"/></a:solidFill></cx:spPr>
                      <cx:dataLabels pos="outEnd">
                        <cx:dataLabel idx="0">
                          <cx:txPr><a:p><a:pPr><a:defRPr><a:solidFill><a:schemeClr val="tx1"/></a:solidFill></a:defRPr></a:pPr></a:p></cx:txPr>
                        </cx:dataLabel>
                        <cx:dataLabel idx="2">
                          <cx:txPr><a:p><a:pPr><a:defRPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></a:defRPr></a:pPr></a:p></cx:txPr>
                        </cx:dataLabel>
                      </cx:dataLabels>
                      <cx:subtotals>
                        <cx:idx val="0"/>
                        <cx:idx val="3"/>
                      </cx:subtotals>
                    </cx:series>
                  </cx:plotAreaRegion>
                  <cx:axis id="0"><cx:catScaling gapWidth="0.8"/></cx:axis>
                  <cx:axis id="1" hidden="1"><cx:valScaling/></cx:axis>
                </cx:plotArea>
              </cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m =
            parse_chartex_part(d.root_element(), &FixtureResolver, None).expect("waterfall parses");

        assert_eq!(m.chart_type, "waterfall");
        assert_eq!(m.categories, vec!["Start", "Up", "Down", "End"]);
        assert_eq!(m.series.len(), 1);
        assert_eq!(
            m.series[0].values,
            vec![Some(100.0), Some(40.0), Some(-30.0), Some(110.0)]
        );
        // Series fill resolved through the resolver (srgbClr uppercased).
        assert_eq!(m.series[0].color.as_deref(), Some("196ECA"));
        // Per-idx label colours: idx0/idx3 unset (None), idx0 tx1→000000,
        // idx2 accent1→4472C4. Presence of any override materializes the vec.
        let dl = m.series[0]
            .data_label_colors
            .as_ref()
            .expect("per-label colours present");
        assert_eq!(dl.len(), 4);
        assert_eq!(dl[0].as_deref(), Some("000000"));
        assert_eq!(dl[1], None);
        assert_eq!(dl[2].as_deref(), Some("4472C4"));
        assert_eq!(dl[3], None);
        // Subtotal indices: idx0 always implicit, explicit idx3 added, the
        // redundant explicit idx0 is de-duplicated away (0 is skipped).
        assert_eq!(m.subtotal_indices, vec![0, 3]);
        // gapWidth fraction 0.8 → legacy percent 80.
        assert_eq!(m.bar_gap_width, Some(80));
        // Hidden value axis, visible category axis.
        assert!(!m.cat_axis_hidden);
        assert!(m.val_axis_hidden);
        // Theme fallback faces threaded from the resolver (NIT-2: not a direct
        // `theme.get("+mj-lt")`).
        assert_eq!(m.theme_major_font_latin.as_deref(), Some("Calibri Light"));
        assert_eq!(m.theme_minor_font_latin.as_deref(), Some("Calibri"));
    }

    /// (b) Treemap: leaf categories + values, no `<cx:subtotals>` (so only the
    /// implicit idx 0), no per-label colours, and no `gapWidth` (→ `None`). The
    /// generic dimension walk carries the treemap leaves exactly like the
    /// waterfall bars, so the same parse serves both layouts.
    #[test]
    fn parse_chartex_part_treemap_hierarchy_values() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="{A_NS}">
              <cx:chartData>
                <cx:data id="0">
                  <cx:strDim type="cat">
                    <cx:lvl ptCount="3">
                      <cx:pt idx="0">North</cx:pt>
                      <cx:pt idx="1">South</cx:pt>
                      <cx:pt idx="2">East</cx:pt>
                    </cx:lvl>
                  </cx:strDim>
                  <cx:numDim type="val">
                    <cx:lvl ptCount="3">
                      <cx:pt idx="0">50</cx:pt>
                      <cx:pt idx="1">30</cx:pt>
                      <cx:pt idx="2">20</cx:pt>
                    </cx:lvl>
                  </cx:numDim>
                </cx:data>
              </cx:chartData>
              <cx:chart>
                <cx:plotArea>
                  <cx:plotAreaRegion>
                    <cx:series layoutId="treemap"/>
                  </cx:plotAreaRegion>
                </cx:plotArea>
              </cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m =
            parse_chartex_part(d.root_element(), &FixtureResolver, None).expect("treemap parses");

        assert_eq!(m.chart_type, "treemap");
        assert_eq!(m.categories, vec!["North", "South", "East"]);
        assert_eq!(m.series[0].values, vec![Some(50.0), Some(30.0), Some(20.0)]);
        assert_eq!(m.series[0].color, None);
        assert_eq!(m.series[0].data_label_colors, None);
        // No `<cx:subtotals>` → only the implicit idx 0.
        assert_eq!(m.subtotal_indices, vec![0]);
        // No `<cx:catScaling gapWidth>` → unset (renderer default applies).
        assert_eq!(m.bar_gap_width, None);
        assert!(!m.cat_axis_hidden);
        assert!(!m.val_axis_hidden);
    }

    /// (c) A `<cx:chartSpace>` with no `<cx:series>` is not a chartEx chart —
    /// `parse_chartex_part` returns `None` rather than an empty model.
    #[test]
    fn parse_chartex_part_returns_none_without_series() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}"><cx:chart><cx:plotArea/></cx:chart></cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        assert!(parse_chartex_part(d.root_element(), &FixtureResolver, None).is_none());
    }

    /// (d) Newlines inside a category `<cx:pt>` are flattened to spaces (Office
    /// writes multi-line axis labels this way; the renderer wants a single
    /// line). Mirrors the sample-2 "FY2024\n1Q営業利益" categories.
    #[test]
    fn parse_chartex_part_category_newline_flattened() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}">
              <cx:chartData><cx:data id="0">
                <cx:strDim type="cat"><cx:lvl ptCount="1"><cx:pt idx="0">FY2024
1Q</cx:pt></cx:lvl></cx:strDim>
                <cx:numDim type="val"><cx:lvl ptCount="1"><cx:pt idx="0">5</cx:pt></cx:lvl></cx:numDim>
              </cx:data></cx:chartData>
              <cx:chart><cx:plotArea><cx:plotAreaRegion>
                <cx:series layoutId="waterfall"/>
              </cx:plotAreaRegion></cx:plotArea></cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m = parse_chartex_part(d.root_element(), &FixtureResolver, None).expect("parses");
        assert_eq!(m.categories, vec!["FY2024 1Q"]);
    }

    // ── CH15: chartEx boxWhisker / sunburst structured parse ─────────────────

    /// A box-and-whisker chart with two series, each referencing its own
    /// `<cx:data>` (via `<cx:dataId>`) of RAW sample points grouped across two
    /// categories. Verifies: (a) categories unique-in-order, (b) each series'
    /// points binned by category, (c) series colored by the cycled theme accent,
    /// (d) `<cx:visibility>` / `<cx:statistics>` flags threaded, (e) the title
    /// is parsed and the accent palette exposed.
    #[test]
    fn parse_chartex_part_boxwhisker_two_series_binned_by_category() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="{A_NS}">
              <cx:chartData>
                <cx:data id="0">
                  <cx:strDim type="cat"><cx:lvl ptCount="3">
                    <cx:pt idx="0">Cat A</cx:pt><cx:pt idx="1">Cat A</cx:pt><cx:pt idx="2">Cat B</cx:pt>
                  </cx:lvl></cx:strDim>
                  <cx:numDim type="val"><cx:lvl ptCount="3">
                    <cx:pt idx="0">1</cx:pt><cx:pt idx="1">3</cx:pt><cx:pt idx="2">10</cx:pt>
                  </cx:lvl></cx:numDim>
                </cx:data>
                <cx:data id="1">
                  <cx:strDim type="cat"><cx:lvl ptCount="3">
                    <cx:pt idx="0">Cat A</cx:pt><cx:pt idx="1">Cat B</cx:pt><cx:pt idx="2">Cat B</cx:pt>
                  </cx:lvl></cx:strDim>
                  <cx:numDim type="val"><cx:lvl ptCount="3">
                    <cx:pt idx="0">5</cx:pt><cx:pt idx="1">7</cx:pt><cx:pt idx="2">9</cx:pt>
                  </cx:lvl></cx:numDim>
                </cx:data>
              </cx:chartData>
              <cx:chart>
                <cx:title><cx:tx><cx:rich><a:p><a:r><a:t>My box chart</a:t></a:r></a:p></cx:rich></cx:tx></cx:title>
                <cx:plotArea><cx:plotAreaRegion>
                  <cx:series layoutId="boxWhisker">
                    <cx:tx><cx:txData><cx:v>Series1</cx:v></cx:txData></cx:tx>
                    <cx:dataId val="0"/>
                    <cx:layoutPr>
                      <cx:visibility meanLine="0" meanMarker="1" nonoutliers="0" outliers="1"/>
                      <cx:statistics quartileMethod="exclusive"/>
                    </cx:layoutPr>
                  </cx:series>
                  <cx:series layoutId="boxWhisker">
                    <cx:tx><cx:txData><cx:v>Series2</cx:v></cx:txData></cx:tx>
                    <cx:dataId val="1"/>
                    <cx:layoutPr>
                      <cx:visibility meanLine="1" meanMarker="0" nonoutliers="1" outliers="0"/>
                      <cx:statistics quartileMethod="inclusive"/>
                    </cx:layoutPr>
                  </cx:series>
                </cx:plotAreaRegion></cx:plotArea>
              </cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m = parse_chartex_part(d.root_element(), &FixtureResolver, None)
            .expect("boxWhisker parses");
        assert_eq!(m.chart_type, "boxWhisker");
        assert_eq!(m.title.as_deref(), Some("My box chart"));
        assert_eq!(
            m.chartex_accents.as_deref(),
            Some(
                &["5B9BD5", "ED7D31", "A5A5A5", "FFC000", "4472C4", "70AD47"].map(String::from)[..]
            )
        );
        let box_data = m.chartex_box.expect("box data present");
        assert_eq!(box_data.categories, vec!["Cat A", "Cat B"]);
        assert_eq!(box_data.series.len(), 2);

        let s0 = &box_data.series[0];
        assert_eq!(s0.name, "Series1");
        assert_eq!(s0.color.as_deref(), Some("5B9BD5")); // accent1 (series idx 0)
                                                         // Series1: Cat A got points 1 & 3, Cat B got 10.
        assert_eq!(s0.values_by_category, vec![vec![1.0, 3.0], vec![10.0]]);
        assert!(s0.mean_marker && !s0.mean_line && s0.show_outliers && !s0.show_nonoutliers);
        assert_eq!(s0.quartile_method, "exclusive");

        let s1 = &box_data.series[1];
        assert_eq!(s1.name, "Series2");
        assert_eq!(s1.color.as_deref(), Some("ED7D31")); // accent2 (series idx 1)
                                                         // Series2: Cat A got 5, Cat B got 7 & 9.
        assert_eq!(s1.values_by_category, vec![vec![5.0], vec![7.0, 9.0]]);
        assert!(!s1.mean_marker && s1.mean_line && !s1.show_outliers && s1.show_nonoutliers);
        assert_eq!(s1.quartile_method, "inclusive");
    }

    /// A sunburst with three `<cx:lvl>` (Leaf / Stem / Branch, in document
    /// order) and a `<cx:numDim type="size">`. Verifies each row's path is built
    /// root→leaf (Branch first) with empty trailing (leaf) cells trimmed so a
    /// node that is itself a leaf terminates early, and that sizes attach by idx.
    #[test]
    fn parse_chartex_part_sunburst_hierarchy_paths_trim_empty_tail() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="{A_NS}">
              <cx:chartData><cx:data id="0">
                <cx:strDim type="cat">
                  <cx:lvl ptCount="3">
                    <cx:pt idx="0">Leaf 1</cx:pt><cx:pt idx="1"/><cx:pt idx="2">Leaf 3</cx:pt>
                  </cx:lvl>
                  <cx:lvl ptCount="3">
                    <cx:pt idx="0">Stem 1</cx:pt><cx:pt idx="1">Leaf 2</cx:pt><cx:pt idx="2">Stem 2</cx:pt>
                  </cx:lvl>
                  <cx:lvl ptCount="3">
                    <cx:pt idx="0">Branch 1</cx:pt><cx:pt idx="1">Branch 1</cx:pt><cx:pt idx="2">Branch 2</cx:pt>
                  </cx:lvl>
                </cx:strDim>
                <cx:numDim type="size"><cx:lvl ptCount="3">
                  <cx:pt idx="0">22</cx:pt><cx:pt idx="1">17</cx:pt><cx:pt idx="2">18</cx:pt>
                </cx:lvl></cx:numDim>
              </cx:data></cx:chartData>
              <cx:chart>
                <cx:title><cx:tx><cx:rich><a:p><a:r><a:t>My sunburst</a:t></a:r></a:p></cx:rich></cx:tx></cx:title>
                <cx:plotArea><cx:plotAreaRegion>
                  <cx:series layoutId="sunburst"><cx:dataId val="0"/></cx:series>
                </cx:plotAreaRegion></cx:plotArea>
              </cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m =
            parse_chartex_part(d.root_element(), &FixtureResolver, None).expect("sunburst parses");
        assert_eq!(m.chart_type, "sunburst");
        assert_eq!(m.title.as_deref(), Some("My sunburst"));
        let sb = m.chartex_sunburst.expect("sunburst data present");
        assert_eq!(sb.rows.len(), 3);
        // Row 0: full Branch→Stem→Leaf chain.
        assert_eq!(sb.rows[0].path, vec!["Branch 1", "Stem 1", "Leaf 1"]);
        assert_eq!(sb.rows[0].size, 22.0);
        // Row 1: empty Leaf cell → path terminates at Stem ("Leaf 2" is itself a leaf).
        assert_eq!(sb.rows[1].path, vec!["Branch 1", "Leaf 2"]);
        assert_eq!(sb.rows[1].size, 17.0);
        // Row 2: full chain under a different branch.
        assert_eq!(sb.rows[2].path, vec!["Branch 2", "Stem 2", "Leaf 3"]);
        assert_eq!(sb.rows[2].size, 18.0);
    }

    /// A waterfall chart (the pre-CH15 chartEx path) must NOT get any of the new
    /// boxWhisker/sunburst structured fields — they stay `None`, keeping the wire
    /// byte-stable for the flat-model chartEx renderers.
    #[test]
    fn parse_chartex_part_waterfall_leaves_chartex_box_sunburst_none() {
        let xml = format!(
            r#"<cx:chartSpace xmlns:cx="{CX_NS}">
              <cx:chartData><cx:data id="0">
                <cx:strDim type="cat"><cx:lvl ptCount="1"><cx:pt idx="0">A</cx:pt></cx:lvl></cx:strDim>
                <cx:numDim type="val"><cx:lvl ptCount="1"><cx:pt idx="0">5</cx:pt></cx:lvl></cx:numDim>
              </cx:data></cx:chartData>
              <cx:chart><cx:plotArea><cx:plotAreaRegion>
                <cx:series layoutId="waterfall"/>
              </cx:plotAreaRegion></cx:plotArea></cx:chart>
            </cx:chartSpace>"#
        );
        let d = chart_space_of(&xml);
        let m =
            parse_chartex_part(d.root_element(), &FixtureResolver, None).expect("waterfall parses");
        assert!(m.chartex_box.is_none());
        assert!(m.chartex_sunburst.is_none());
        assert!(m.chartex_accents.is_none());
        assert!(m.title.is_none());
    }

    /// `<cs:title><cs:defRPr sz>` in a chartStyle part extracts the title size
    /// (hpt). Word's default modern chart style writes 1400 (14pt).
    #[test]
    fn extract_chartex_style_title_size_reads_cs_defrpr() {
        let style = format!(
            r#"<cs:chartStyle xmlns:cs="{CS_NS}" xmlns:a="{A_NS}">
              <cs:title><cs:defRPr sz="1400" b="0"/></cs:title>
            </cs:chartStyle>"#
        );
        assert_eq!(extract_chartex_style_title_size(&style), Some(1400));
        // No <cs:title> / no sz → None.
        assert!(extract_chartex_style_title_size(&format!(
            r#"<cs:chartStyle xmlns:cs="{CS_NS}"><cs:dataPoint/></cs:chartStyle>"#
        ))
        .is_none());
        // Malformed XML → None (not a panic).
        assert!(extract_chartex_style_title_size("<not xml").is_none());
    }

    /// A chartEx title with no inline `sz` falls back to the chartStyle part's
    /// `<cs:title>` size; an inline `sz` on the `<cx:title>` rich text wins over
    /// the style part; and with no style part at all the size is `None` (the
    /// renderer's area-proportional fallback).
    #[test]
    fn parse_chartex_part_title_size_resolves_from_style_part() {
        let chart_xml = |title_rpr: &str| {
            format!(
                r#"<cx:chartSpace xmlns:cx="{CX_NS}" xmlns:a="{A_NS}">
                  <cx:chartData><cx:data id="0">
                    <cx:strDim type="cat"><cx:lvl ptCount="1"><cx:pt idx="0">Leaf</cx:pt></cx:lvl></cx:strDim>
                    <cx:numDim type="size"><cx:lvl ptCount="1"><cx:pt idx="0">1</cx:pt></cx:lvl></cx:numDim>
                  </cx:data></cx:chartData>
                  <cx:chart>
                    <cx:title><cx:tx><cx:rich><a:p><a:pPr>{title_rpr}</a:pPr>
                      <a:r><a:t>T</a:t></a:r></a:p></cx:rich></cx:tx></cx:title>
                    <cx:plotArea><cx:plotAreaRegion>
                      <cx:series layoutId="sunburst"><cx:dataId val="0"/></cx:series>
                    </cx:plotAreaRegion></cx:plotArea>
                  </cx:chart>
                </cx:chartSpace>"#
            )
        };
        let style = format!(
            r#"<cs:chartStyle xmlns:cs="{CS_NS}"><cs:title><cs:defRPr sz="1400"/></cs:title></cs:chartStyle>"#
        );

        // No inline sz + style part → style part's 1400.
        let x0 = chart_xml("<a:defRPr/>");
        let d0 = chart_space_of(&x0);
        let m0 = parse_chartex_part(d0.root_element(), &FixtureResolver, Some(&style)).unwrap();
        assert_eq!(m0.title_font_size_hpt, Some(1400));

        // Inline sz on the title wins over the style part.
        let x1 = chart_xml(r#"<a:defRPr sz="2000"/>"#);
        let d1 = chart_space_of(&x1);
        let m1 = parse_chartex_part(d1.root_element(), &FixtureResolver, Some(&style)).unwrap();
        assert_eq!(m1.title_font_size_hpt, Some(2000));

        // No style part and no inline sz → None (renderer fallback).
        let x2 = chart_xml("<a:defRPr/>");
        let d2 = chart_space_of(&x2);
        let m2 = parse_chartex_part(d2.root_element(), &FixtureResolver, None).unwrap();
        assert_eq!(m2.title_font_size_hpt, None);
    }

    // ── CH13: 3D flattening / stock / ofPie type detection ───────────────────

    /// Build a minimal `<c:chartSpace>` whose plot area holds a single
    /// chart-group element (`group_xml`) with one series. Used by the CH13
    /// type-detection probes below.
    fn chart_space_with_group(group_xml: &str) -> String {
        format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart><c:plotArea>
                {group_xml}
                <c:catAx><c:axId val="1"/><c:axPos val="b"/></c:catAx>
                <c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx>
              </c:plotArea></c:chart>
            </c:chartSpace>"#
        )
    }

    const CH13_SER: &str = r#"<c:ser><c:idx val="0"/>
        <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:cat>
        <c:val><c:numCache><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>7</c:v></c:pt></c:numCache></c:val>
      </c:ser>"#;

    /// §21.2.2.140 pie3DChart flattens to a plain 2D `pie`. The `<a:scene3d>` /
    /// `<c:varyColors>` decoration is ignored; the series/cat/val flow through.
    #[test]
    fn parse_chart_part_pie3d_flattens_to_pie() {
        let group = format!(r#"<c:pie3DChart><c:varyColors val="1"/>{CH13_SER}</c:pie3DChart>"#);
        let xml_p = chart_space_with_group(&group);
        let d = chart_space_of(&xml_p);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("pie3D parses");
        assert_eq!(m.chart_type, "pie");
        assert_eq!(m.series.len(), 1);
        assert_eq!(m.series[0].values, vec![Some(3.0), Some(7.0)]);
        assert_eq!(m.categories, vec!["A".to_string(), "B".to_string()]);
    }

    /// §21.2.2.15 bar3DChart with `barDir=col` + `grouping=stacked` flattens to
    /// `stackedBar` (the `<c:gapDepth>` 3D-only attr is ignored).
    #[test]
    fn parse_chart_part_bar3d_flattens_by_grouping_and_dir() {
        let group = format!(
            r#"<c:bar3DChart><c:barDir val="col"/><c:grouping val="stacked"/><c:gapDepth val="150"/>{CH13_SER}</c:bar3DChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("bar3D parses");
        assert_eq!(m.chart_type, "stackedBar");

        // barDir=bar (horizontal) + clustered → clusteredBarH.
        let group_h = format!(
            r#"<c:bar3DChart><c:barDir val="bar"/><c:grouping val="clustered"/>{CH13_SER}</c:bar3DChart>"#
        );
        let xml_h = chart_space_with_group(&group_h);
        let d2 = chart_space_of(&xml_h);
        let m2 = parse_chart_part(d2.root_element(), &FixtureResolver).expect("bar3D-h parses");
        assert_eq!(m2.chart_type, "clusteredBarH");
    }

    /// §21.2.2.96 line3DChart → `line`; §21.2.2.4 area3DChart(stacked) →
    /// `stackedArea`.
    #[test]
    fn parse_chart_part_line3d_area3d_flatten() {
        let line = format!(r#"<c:line3DChart>{CH13_SER}</c:line3DChart>"#);
        let xml_l = chart_space_with_group(&line);
        let dl = chart_space_of(&xml_l);
        assert_eq!(
            parse_chart_part(dl.root_element(), &FixtureResolver)
                .unwrap()
                .chart_type,
            "line"
        );
        let area =
            format!(r#"<c:area3DChart><c:grouping val="stacked"/>{CH13_SER}</c:area3DChart>"#);
        let xml_a = chart_space_with_group(&area);
        let da = chart_space_of(&xml_a);
        assert_eq!(
            parse_chart_part(da.root_element(), &FixtureResolver)
                .unwrap()
                .chart_type,
            "stackedArea"
        );
    }

    /// §21.2.2.198 stockChart → `stock` (its high/low/close series flow through
    /// the shared collectors unchanged).
    #[test]
    fn parse_chart_part_stock_detected() {
        let hi = r#"<c:ser><c:idx val="0"/><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>High</c:v></c:pt></c:strCache></c:strRef></c:tx>
            <c:cat><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:cat>
            <c:val><c:numCache><c:pt idx="0"><c:v>55</c:v></c:pt></c:numCache></c:val></c:ser>"#;
        let group = format!(
            r#"<c:stockChart>{hi}<c:hiLowLines><c:spPr><a:ln><a:solidFill><a:srgbClr val="808080"/></a:solidFill></a:ln></c:spPr></c:hiLowLines></c:stockChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("stock parses");
        assert_eq!(m.chart_type, "stock");
        assert_eq!(m.series.len(), 1);
        assert_eq!(m.series[0].name, "High");
        assert_eq!(m.series[0].values, vec![Some(55.0)]);
        // hiLowLines present + its resolved line color; no upDownBars in fixture.
        assert_eq!(m.stock_hi_low_lines, Some(true));
        assert_eq!(m.stock_hi_low_line_color.as_deref(), Some("808080"));
        assert_eq!(m.stock_up_down_bars, None);
    }

    /// A stock chart WITHOUT `<c:hiLowLines>` but WITH `<c:upDownBars>`: the
    /// hi-lo flag is `Some(false)` (element absent) and up/down bars are
    /// recognized as `Some(true)` even though the renderer does not draw them.
    #[test]
    fn parse_chart_part_stock_up_down_bars_recognized() {
        let ser = r#"<c:ser><c:idx val="0"/>
            <c:cat><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt></c:numCache></c:cat>
            <c:val><c:numCache><c:pt idx="0"><c:v>5</c:v></c:pt></c:numCache></c:val></c:ser>"#;
        let group = format!(r#"<c:stockChart>{ser}<c:upDownBars/></c:stockChart>"#);
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("stock parses");
        assert_eq!(m.stock_hi_low_lines, Some(false));
        assert_eq!(m.stock_hi_low_line_color, None);
        assert_eq!(m.stock_up_down_bars, Some(true));
    }

    /// §21.2.2.126 ofPieChart → `pie` (main-pie-only fallback). Uses the
    /// two-point CH13_SER for the type-only assertion; the full-contract test
    /// below adds the secondary-plot elements.
    #[test]
    fn parse_chart_part_ofpie_flattens_to_pie() {
        let group = format!(
            r#"<c:ofPieChart><c:ofPieType val="pie"/><c:varyColors val="1"/>{CH13_SER}</c:ofPieChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("ofPie parses");
        assert_eq!(m.chart_type, "pie");
        assert_eq!(m.series[0].values, vec![Some(3.0), Some(7.0)]);
    }

    /// Full ofPieChart contract: the secondary-plot elements (`<c:splitType>`,
    /// `<c:splitPos>`, `<c:secondPieSize>`, `<c:serLines>`) are IGNORED — the
    /// whole series still becomes a single `pie` whose every data point is a
    /// slice, and `<c:varyColors>` cycles the accent palette across the slices.
    /// A `bar` `ofPieType` flattens the same way. Pins the "draw one combined
    /// pie" decision so a future edit can't silently start honoring the split.
    #[test]
    fn parse_chart_part_ofpie_full_contract_ignores_split() {
        let ser = r#"<c:ser><c:idx val="0"/>
            <c:cat><c:strCache>
              <c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt>
              <c:pt idx="2"><c:v>C</c:v></c:pt><c:pt idx="3"><c:v>D</c:v></c:pt>
            </c:strCache></c:cat>
            <c:val><c:numCache>
              <c:pt idx="0"><c:v>40</c:v></c:pt><c:pt idx="1"><c:v>30</c:v></c:pt>
              <c:pt idx="2"><c:v>20</c:v></c:pt><c:pt idx="3"><c:v>10</c:v></c:pt>
            </c:numCache></c:val></c:ser>"#;
        // bar-of-pie with a custom split of the last two points into the bar,
        // plus connector series-lines — all of which we ignore.
        let group = format!(
            r#"<c:ofPieChart>
                <c:ofPieType val="bar"/>
                <c:varyColors val="1"/>
                {ser}
                <c:gapWidth val="100"/>
                <c:splitType val="pos"/>
                <c:splitPos val="2"/>
                <c:secondPieSize val="75"/>
                <c:serLines/>
            </c:ofPieChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("ofPie parses");
        assert_eq!(m.chart_type, "pie");
        // Every one of the four points is present as a slice value (nothing was
        // diverted to a phantom secondary plot).
        assert_eq!(
            m.series[0].values,
            vec![Some(40.0), Some(30.0), Some(20.0), Some(10.0)]
        );
        // varyColors cycled the accent palette across all four slices.
        let colors = m.series[0]
            .data_point_colors
            .as_ref()
            .expect("varyColors slice palette");
        assert_eq!(colors[0].as_deref(), Some("4472C4")); // accent1
        assert_eq!(colors[1].as_deref(), Some("ED7D31")); // accent2
        assert_eq!(colors[2].as_deref(), Some("A5A5A5")); // accent3
        assert_eq!(colors[3].as_deref(), Some("FFC000")); // accent4
    }

    /// A resolver that DOES supply the default series accent palette (like the
    /// real docx/xlsx resolvers), used to pin the §21.2.2.227 `<c:varyColors>`
    /// per-slice accent fill. `FixtureResolver` returns `None` for accents so it
    /// cannot exercise this path.
    struct AccentResolver;
    impl ColorResolver for AccentResolver {
        fn resolve_solid_fill(&self, node: Node) -> Option<String> {
            node.children()
                .find(|n| n.is_element() && n.tag_name().name() == "srgbClr")
                .and_then(|n| attr(&n, "val"))
                .map(|v| v.to_uppercase())
        }
        fn resolve_series_accent(&self, idx: usize) -> Option<String> {
            // Six-accent cycle, matching `theme.accent[(idx % 6) + 1]`.
            const ACCENTS: [&str; 6] = ["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"];
            Some(ACCENTS[idx % 6].to_string())
        }
    }

    /// §21.2.2.227 varyColors (default ON for pie): each slice without an
    /// explicit `<c:dPt>` fill takes the theme accent for its point index, so a
    /// docx/xlsx pie matches Office instead of the renderer's built-in palette.
    /// The one slice that DOES carry a `<c:dPt>` fill keeps it.
    #[test]
    fn parse_chart_part_pie_vary_colors_fills_accents() {
        let ser = r#"<c:ser><c:idx val="0"/>
            <c:dPt><c:idx val="0"/><c:spPr><a:solidFill><a:srgbClr val="112233"/></a:solidFill></c:spPr></c:dPt>
            <c:cat><c:strCache>
              <c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt><c:pt idx="2"><c:v>C</c:v></c:pt>
            </c:strCache></c:cat>
            <c:val><c:numCache>
              <c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt><c:pt idx="2"><c:v>3</c:v></c:pt>
            </c:numCache></c:val></c:ser>"#;
        // No <c:varyColors> element → defaults to ON for the pie family.
        let group = format!(r#"<c:pieChart>{ser}</c:pieChart>"#);
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("pie parses");
        let colors = m.series[0]
            .data_point_colors
            .as_ref()
            .expect("varyColors populates slice palette");
        assert_eq!(colors[0].as_deref(), Some("112233")); // explicit dPt wins
        assert_eq!(colors[1].as_deref(), Some("ED7D31")); // accent2
        assert_eq!(colors[2].as_deref(), Some("A5A5A5")); // accent3

        // varyColors="0" disables the per-slice accent fill: only the explicit
        // dPt color remains, the rest fall back to None (renderer palette).
        let group_off = format!(r#"<c:pieChart><c:varyColors val="0"/>{ser}</c:pieChart>"#);
        let xml_off = chart_space_with_group(&group_off);
        let d2 = chart_space_of(&xml_off);
        let m2 = parse_chart_part(d2.root_element(), &AccentResolver).expect("pie parses");
        let colors2 = m2.series[0].data_point_colors.as_ref().unwrap();
        assert_eq!(colors2[0].as_deref(), Some("112233"));
        assert_eq!(colors2[1], None);
        assert_eq!(colors2[2], None);
    }

    /// A SINGLE-series bar chart with `<c:varyColors>` ABSENT varies by point by
    /// default (issue #931): each data point takes the accent for its index and
    /// the chart-level flag is set. This is the sample-17/18 shape — a lone
    /// column series whose four bars render in the rotating theme palette even
    /// though the file carries no `<c:varyColors>` element.
    #[test]
    fn parse_chart_part_bar_single_series_varies_by_default() {
        let group = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{CH13_SER}</c:barChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("bar parses");
        assert_eq!(m.vary_colors, Some(true));
        let colors = m.series[0]
            .data_point_colors
            .as_ref()
            .expect("default vary populates per-point palette");
        assert_eq!(colors[0].as_deref(), Some("4472C4")); // accent1
        assert_eq!(colors[1].as_deref(), Some("ED7D31")); // accent2
    }

    /// A SINGLE-series bar chart with an explicit `<c:varyColors val="0"/>`
    /// keeps its one per-series color (Office records the forced-single-color
    /// choice this way — sample-1.xlsx / sample-14 chart8/9). No per-point fill,
    /// no chart-level flag.
    #[test]
    fn parse_chart_part_bar_single_series_vary_off_keeps_series_color() {
        let group = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>{CH13_SER}</c:barChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("bar parses");
        assert_eq!(m.vary_colors, None);
        assert!(m.series[0].data_point_colors.is_none());
    }

    /// §21.2.2.227 varyColors on a SINGLE-series bar/column chart: each data
    /// point (bar) without an explicit `<c:dPt>` fill takes the theme accent for
    /// its point index, and the chart-level `vary_colors` flag is set so the
    /// core renderer colors each bar per point and lists one legend entry per
    /// point (issue #931). The point that carries a `<c:dPt>` fill keeps it.
    #[test]
    fn parse_chart_part_bar_vary_colors_single_series_fills_accents() {
        let ser = r#"<c:ser><c:idx val="0"/>
            <c:dPt><c:idx val="0"/><c:spPr><a:solidFill><a:srgbClr val="112233"/></a:solidFill></c:spPr></c:dPt>
            <c:cat><c:strCache>
              <c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt>
              <c:pt idx="2"><c:v>C</c:v></c:pt><c:pt idx="3"><c:v>D</c:v></c:pt>
            </c:strCache></c:cat>
            <c:val><c:numCache>
              <c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt>
              <c:pt idx="2"><c:v>3</c:v></c:pt><c:pt idx="3"><c:v>4</c:v></c:pt>
            </c:numCache></c:val></c:ser>"#;
        let group = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="1"/>{ser}</c:barChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("bar parses");
        assert_eq!(m.vary_colors, Some(true));
        let colors = m.series[0]
            .data_point_colors
            .as_ref()
            .expect("varyColors populates per-point palette");
        assert_eq!(colors[0].as_deref(), Some("112233")); // explicit dPt wins
        assert_eq!(colors[1].as_deref(), Some("ED7D31")); // accent2
        assert_eq!(colors[2].as_deref(), Some("A5A5A5")); // accent3
        assert_eq!(colors[3].as_deref(), Some("FFC000")); // accent4
    }

    /// §21.2.2.227 varyColors on a MULTI-series bar chart is a no-op for the
    /// per-point fill: Office keeps per-series colors when several series share
    /// the axes, so neither the per-point palette nor the chart-level flag is
    /// emitted. Only the single-series case (issue #931) varies by point.
    #[test]
    fn parse_chart_part_bar_vary_colors_multi_series_keeps_series_colors() {
        let ser0 = r#"<c:ser><c:idx val="0"/>
            <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:cat>
            <c:val><c:numCache><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numCache></c:val></c:ser>"#;
        let ser1 = r#"<c:ser><c:idx val="1"/>
            <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strCache></c:cat>
            <c:val><c:numCache><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>4</c:v></c:pt></c:numCache></c:val></c:ser>"#;
        let group = format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="1"/>{ser0}{ser1}</c:barChart>"#
        );
        let xml = chart_space_with_group(&group);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &AccentResolver).expect("bar parses");
        assert_eq!(m.vary_colors, None);
        assert!(m.series[0].data_point_colors.is_none());
        assert!(m.series[1].data_point_colors.is_none());
    }

    /// A named single series in `<c:tx>` — reused by the auto-title tests. `idx`
    /// distinguishes the two series in the multi-series fixture.
    fn named_ser(idx: u32, name: &str) -> String {
        format!(
            r#"<c:ser><c:idx val="{idx}"/>
              <c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>{name}</c:v></c:pt></c:strCache></c:strRef></c:tx>
              <c:cat><c:strCache><c:pt idx="0"><c:v>A</c:v></c:pt></c:strCache></c:cat>
              <c:val><c:numCache><c:pt idx="0"><c:v>3</c:v></c:pt></c:numCache></c:val>
            </c:ser>"#
        )
    }

    /// A `<c:chart>` with an optional `<c:autoTitleDeleted val=…>`, NO explicit
    /// `<c:title>` text, and the given series in a bar plot area. Models the
    /// sample-25 shape (auto-title chart: title frame present but empty, so the
    /// synthesized title comes from the series name).
    fn chart_space_auto_title(auto_title_deleted: Option<&str>, sers: &str) -> String {
        let atd = auto_title_deleted
            .map(|v| format!(r#"<c:autoTitleDeleted val="{v}"/>"#))
            .unwrap_or_default();
        format!(
            r#"<c:chartSpace xmlns:c="{C_NS}" xmlns:a="{A_NS}">
              <c:chart>
                {atd}
                <c:plotArea>
                  <c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{sers}</c:barChart>
                  <c:catAx><c:axId val="1"/><c:axPos val="b"/></c:catAx>
                  <c:valAx><c:axId val="2"/><c:axPos val="l"/></c:valAx>
                </c:plotArea>
              </c:chart>
            </c:chartSpace>"#
        )
    }

    /// ECMA-376 §21.2.2.7 auto-title: a chart with NO explicit title text,
    /// `autoTitleDeleted` absent (⇒ auto title may show), and EXACTLY ONE named
    /// series adopts that series' name as the chart title. Ground truth:
    /// sample-25.docx — pie3D with a lone "Production in 2017" series and an
    /// empty title frame — where Word shows "Production in 2017" as the title.
    #[test]
    fn parse_chart_part_auto_title_single_series() {
        let xml = chart_space_auto_title(None, &named_ser(0, "Production in 2017"));
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("parses");
        // The series name is promoted VERBATIM (the `cap="all"` uppercase is a
        // rendering-layer transform we do not apply at parse time).
        assert_eq!(m.title.as_deref(), Some("Production in 2017"));

        // An explicit `autoTitleDeleted val="0"` behaves identically (0 ⇒ auto
        // title may be shown).
        let xml0 = chart_space_auto_title(Some("0"), &named_ser(0, "Production in 2017"));
        let d0 = chart_space_of(&xml0);
        let m0 = parse_chart_part(d0.root_element(), &FixtureResolver).expect("parses");
        assert_eq!(m0.title.as_deref(), Some("Production in 2017"));
    }

    /// §21.2.2.7 `autoTitleDeleted val="1"` (or `"true"`) suppresses the auto
    /// title even for a single named series — Word shows no title.
    #[test]
    fn parse_chart_part_auto_title_deleted_shows_no_title() {
        for v in ["1", "true"] {
            let xml = chart_space_auto_title(Some(v), &named_ser(0, "Production in 2017"));
            let d = chart_space_of(&xml);
            let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("parses");
            assert_eq!(m.title, None, "autoTitleDeleted={v} should suppress title");
        }
    }

    /// §21.2.2.7 auto-title applies ONLY to single-series charts. With TWO
    /// series, Word shows no synthesized title (a lone series name would be
    /// misleading), so `title` stays `None`.
    #[test]
    fn parse_chart_part_auto_title_multi_series_none() {
        let sers = format!(
            "{}{}",
            named_ser(0, "Series One"),
            named_ser(1, "Series Two")
        );
        let xml = chart_space_auto_title(None, &sers);
        let d = chart_space_of(&xml);
        let m = parse_chart_part(d.root_element(), &FixtureResolver).expect("parses");
        assert_eq!(m.series.len(), 2);
        assert_eq!(m.title, None);
    }

    struct FormulaResolver;

    impl ChartReferenceResolver for FormulaResolver {
        fn resolve_strings(&mut self, formula: &str) -> Option<Vec<String>> {
            Some(match formula {
                "Name" => vec!["Resolved series".into()],
                "X" => vec!["1".into(), "2".into()],
                "CachedCats" => vec!["live value must not win".into()],
                _ => return None,
            })
        }

        fn resolve_numbers(&mut self, formula: &str) -> Option<Vec<Option<f64>>> {
            Some(match formula {
                "Y" => vec![Some(10.0), Some(20.0)],
                "Size" => vec![Some(3.0), Some(5.0)],
                _ => return None,
            })
        }
    }

    #[test]
    fn parse_chart_part_resolves_cacheless_bubble_fields() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea><c:bubbleChart>
              <c:ser><c:idx val="0"/><c:order val="0"/>
                <c:tx><c:strRef><c:f>Name</c:f></c:strRef></c:tx>
                <c:xVal><c:numRef><c:f>X</c:f></c:numRef></c:xVal>
                <c:yVal><c:numRef><c:f>Y</c:f></c:numRef></c:yVal>
                <c:bubbleSize><c:numRef><c:f>Size</c:f></c:numRef></c:bubbleSize>
              </c:ser>
            </c:bubbleChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = root_of(&xml);
        let mut references = FormulaResolver;
        let chart =
            parse_chart_part_with_references(doc.root_element(), &FixtureResolver, &mut references)
                .expect("bubble chart parses");

        assert_eq!(chart.categories, vec!["1", "2"]);
        assert_eq!(chart.series[0].name, "Resolved series");
        assert_eq!(chart.series[0].categories, None);
        assert_eq!(chart.series[0].values, vec![Some(10.0), Some(20.0)]);
        assert_eq!(
            chart.series[0].bubble_sizes,
            Some(vec![Some(3.0), Some(5.0)])
        );
    }

    #[test]
    fn parse_chart_part_keeps_unresolved_cacheless_bubble_sizes_absent() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea><c:bubbleChart>
              <c:ser><c:idx val="0"/><c:order val="0"/>
                <c:xVal><c:numRef><c:f>X</c:f></c:numRef></c:xVal>
                <c:yVal><c:numRef><c:f>Y</c:f></c:numRef></c:yVal>
                <c:bubbleSize><c:numRef><c:f>Size</c:f></c:numRef></c:bubbleSize>
              </c:ser>
            </c:bubbleChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = root_of(&xml);
        let chart = parse_chart_part(doc.root_element(), &FixtureResolver)
            .expect("cacheless bubble chart still parses");

        assert_eq!(chart.categories, Vec::<String>::new());
        assert_eq!(chart.series[0].categories, None);
        assert_eq!(chart.series[0].values, Vec::<Option<f64>>::new());
        assert_eq!(chart.series[0].bubble_sizes, None);
    }

    struct CountingCategoryResolver {
        string_calls: usize,
    }

    impl ChartReferenceResolver for CountingCategoryResolver {
        fn resolve_strings(&mut self, formula: &str) -> Option<Vec<String>> {
            self.string_calls += 1;
            (formula == "Cats").then(|| vec!["A".into(), "B".into(), "C".into()])
        }

        fn resolve_numbers(&mut self, _formula: &str) -> Option<Vec<Option<f64>>> {
            None
        }
    }

    #[test]
    fn parse_chart_part_resolves_shared_categories_once_without_series_clones() {
        let first = r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:cat><c:strRef><c:f>Cats</c:f></c:strRef></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>1</c:v></c:pt></c:numLit></c:val></c:ser>"#;
        let repeated = r#"<c:ser><c:idx val="1"/><c:order val="1"/><c:cat><c:strRef><c:f>Cats</c:f></c:strRef></c:cat><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>2</c:v></c:pt></c:numLit></c:val></c:ser>"#;
        let category_less: String = (2..12)
            .map(|idx| format!(r#"<c:ser><c:idx val="{idx}"/><c:order val="{idx}"/><c:val><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>{idx}</c:v></c:pt></c:numLit></c:val></c:ser>"#))
            .collect();
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea><c:lineChart>{first}{repeated}{category_less}</c:lineChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = root_of(&xml);
        let mut references = CountingCategoryResolver { string_calls: 0 };
        let chart =
            parse_chart_part_with_references(doc.root_element(), &FixtureResolver, &mut references)
                .expect("multi-series chart parses");

        assert_eq!(references.string_calls, 1);
        assert_eq!(chart.categories, vec!["A", "B", "C"]);
        assert_eq!(chart.series.len(), 12);
        assert!(chart
            .series
            .iter()
            .all(|series| series.categories.is_none()));
    }

    #[test]
    fn parse_chart_part_does_not_reuse_first_x_for_unresolved_distinct_scatter_x() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea><c:scatterChart>
              <c:ser><c:idx val="0"/><c:order val="0"/><c:xVal><c:numRef><c:f>X</c:f></c:numRef></c:xVal><c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:yVal></c:ser>
              <c:ser><c:idx val="1"/><c:order val="1"/><c:xVal><c:numRef><c:f>UnavailableX</c:f></c:numRef></c:xVal><c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>4</c:v></c:pt></c:numLit></c:yVal></c:ser>
              <c:ser><c:idx val="2"/><c:order val="2"/><c:xVal><c:numRef><c:f>X</c:f><c:numCache><c:ptCount val="0"/></c:numCache></c:numRef></c:xVal><c:yVal><c:numLit><c:ptCount val="1"/><c:pt idx="0"><c:v>5</c:v></c:pt></c:numLit></c:yVal></c:ser>
            </c:scatterChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = root_of(&xml);
        let mut references = FormulaResolver;
        let chart =
            parse_chart_part_with_references(doc.root_element(), &FixtureResolver, &mut references)
                .expect("two-series scatter parses");

        assert_eq!(chart.categories, vec!["1", "2"]);
        assert_eq!(chart.series[0].categories, None);
        assert_eq!(chart.series[1].categories, Some(Vec::new()));
        assert_eq!(chart.series[2].categories, Some(Vec::new()));
    }

    #[test]
    fn parse_chart_part_preserves_authored_multilevel_cache() {
        let xml = format!(
            r#"<c:chartSpace xmlns:c="{C_NS}"><c:chart><c:plotArea><c:barChart>
              <c:barDir val="col"/><c:ser><c:idx val="0"/><c:order val="0"/>
                <c:cat><c:multiLvlStrRef><c:f>CachedCats</c:f><c:multiLvlStrCache>
                  <c:ptCount val="2"/><c:lvl><c:pt idx="0"><c:v>Authored A</c:v></c:pt><c:pt idx="1"><c:v>Authored B</c:v></c:pt></c:lvl>
                </c:multiLvlStrCache></c:multiLvlStrRef></c:cat>
                <c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:val>
              </c:ser>
            </c:barChart></c:plotArea></c:chart></c:chartSpace>"#
        );
        let doc = root_of(&xml);
        let mut references = FormulaResolver;
        let chart =
            parse_chart_part_with_references(doc.root_element(), &FixtureResolver, &mut references)
                .expect("bar chart parses");

        assert_eq!(chart.categories, vec!["Authored A", "Authored B"]);
        assert_eq!(chart.series[0].categories, None);
    }
}
