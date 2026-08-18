//! Isolated Word compatibility rules for DrawingML group transforms.

use ooxml_common::drawing::{DrawingGroupTransform, DrawingRect};

/// Apply Word's exact-quarter-turn scale order for a directly grouped leaf.
///
/// ECMA-376 Part 1 Annex L §L.4.7.4 accumulates scale on authored axes.
/// [MS-OE376] §2.1.1360 defines the Office group ratio as `ext / chExt`.
/// Word-produced reference output additionally shows that a directly grouped
/// leaf at an exact odd quarter turn uses that ratio on post-rotation page
/// axes. The single-group restriction is deliberate: nested rotated groups
/// require a hierarchy-aware retained transform and are not generalized from
/// this observation.
pub(crate) fn apply_word_direct_group_rect(
    transform: DrawingGroupTransform,
    rect: DrawingRect,
) -> DrawingRect {
    let mapped = transform.apply_rect(rect);
    let quarter_turns = (rect.rotation_degrees / 90.0).round();
    let exact_quarter_turn = (rect.rotation_degrees - quarter_turns * 90.0).abs() < 1e-9;
    if transform.group_depth() != 1
        || !exact_quarter_turn
        || (quarter_turns as i64).rem_euclid(2) != 1
    {
        return mapped;
    }

    let center_x = mapped.x + mapped.width / 2.0;
    let center_y = mapped.y + mapped.height / 2.0;
    let width = rect.width * transform.scale_y;
    let height = rect.height * transform.scale_x;
    DrawingRect {
        x: center_x - width / 2.0,
        y: center_y - height / 2.0,
        width,
        height,
        ..mapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ooxml_common::drawing::DrawingGroupSpec;

    fn group(rotation_degrees: f64) -> DrawingGroupSpec {
        DrawingGroupSpec {
            off_x: 0.0,
            off_y: 0.0,
            ext_x: 127_000.0,
            ext_y: 254_000.0,
            child_off_x: 0.0,
            child_off_y: 0.0,
            child_ext_x: 127_000.0,
            child_ext_y: 127_000.0,
            rotation_degrees,
            flip_h: false,
            flip_v: false,
        }
    }

    fn leaf(rotation_degrees: f64) -> DrawingRect {
        DrawingRect {
            x: 0.0,
            y: 50_800.0,
            width: 127_000.0,
            height: 25_400.0,
            rotation_degrees,
            flip_h: false,
            flip_v: false,
        }
    }

    #[test]
    fn exchanges_axes_for_a_direct_exact_quarter_turn() {
        let mapped =
            apply_word_direct_group_rect(DrawingGroupTransform::from_group(group(0.0)), leaf(90.0));
        assert!((mapped.x + 63_500.0).abs() < 1e-6);
        assert!((mapped.y - 114_300.0).abs() < 1e-6);
        assert!((mapped.width - 254_000.0).abs() < 1e-6);
        assert!((mapped.height - 25_400.0).abs() < 1e-6);
    }

    #[test]
    fn leaves_nested_rotated_groups_on_the_annex_l_path() {
        let nested = DrawingGroupTransform::from_group(group(90.0)).compose_group(group(0.0));
        let mapped = apply_word_direct_group_rect(nested, leaf(0.0));
        let normative = nested.apply_rect(leaf(0.0));
        assert_eq!(mapped, normative);
        assert_eq!(nested.group_depth(), 2);
    }
}
