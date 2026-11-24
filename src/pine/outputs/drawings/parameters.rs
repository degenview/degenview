//! The parameters of every drawing function, in TradingView's order: the one list the runtime binds
//! a call's arguments with (positional or by name) and that the checker and the editor read.

/// `line.new`, `label.new` and `box.new` also take two `chart.point`s (or one) in place of
/// coordinates; this is the parameter list of that form.
pub fn point_parameters(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "line.new" => &[
            "first_point",
            "second_point",
            "xloc",
            "extend",
            "color",
            "style",
            "width",
            "force_overlay",
        ],
        "label.new" => &[
            "point",
            "text",
            "xloc",
            "yloc",
            "color",
            "style",
            "textcolor",
            "size",
            "textalign",
            "tooltip",
            "text_font_family",
            "force_overlay",
            "text_formatting",
        ],
        "box.new" => &[
            "top_left",
            "bottom_right",
            "border_color",
            "border_width",
            "border_style",
            "extend",
            "xloc",
            "bgcolor",
            "text",
            "text_size",
            "text_color",
            "text_halign",
            "text_valign",
            "text_wrap",
            "text_font_family",
            "force_overlay",
            "text_formatting",
        ],
        _ => return None,
    })
}

/// The parameters of the drawing function `name` (`label.set_xy`, `table.cell`, `chart.point.now`),
/// the handle first for a function that works on one. Empty for a name that is not a drawing
/// function. A constructor with a `chart.point` form lists the coordinates form here; see
/// [`point_parameters`].
pub fn parameters(name: &str) -> &'static [&'static str] {
    match name {
        "line.new" => &[
            "x1",
            "y1",
            "x2",
            "y2",
            "xloc",
            "extend",
            "color",
            "style",
            "width",
            "force_overlay",
        ],
        "label.new" => &[
            "x",
            "y",
            "text",
            "xloc",
            "yloc",
            "color",
            "style",
            "textcolor",
            "size",
            "textalign",
            "tooltip",
            "text_font_family",
            "force_overlay",
            "text_formatting",
        ],
        "box.new" => &[
            "left",
            "top",
            "right",
            "bottom",
            "border_color",
            "border_width",
            "border_style",
            "extend",
            "xloc",
            "bgcolor",
            "text",
            "text_size",
            "text_color",
            "text_halign",
            "text_valign",
            "text_wrap",
            "text_font_family",
            "force_overlay",
            "text_formatting",
        ],
        "linefill.new" => &["line1", "line2", "color"],
        "polyline.new" => &[
            "points",
            "curved",
            "closed",
            "xloc",
            "line_color",
            "fill_color",
            "line_style",
            "line_width",
            "force_overlay",
        ],
        "table.new" => &[
            "position",
            "columns",
            "rows",
            "bgcolor",
            "frame_color",
            "frame_width",
            "border_color",
            "border_width",
            "force_overlay",
        ],
        "table.cell" => &[
            "table_id",
            "column",
            "row",
            "text",
            "width",
            "height",
            "text_color",
            "text_halign",
            "text_valign",
            "text_size",
            "bgcolor",
            "tooltip",
            "text_font_family",
            "text_formatting",
        ],
        "table.merge_cells" | "table.clear" => &[
            "table_id",
            "start_column",
            "start_row",
            "end_column",
            "end_row",
        ],
        "chart.point" | "chart.point.new" => &["time", "index", "price"],
        "chart.point.from_index" => &["index", "price"],
        "chart.point.from_time" => &["time", "price"],
        "chart.point.now" => &["price"],
        "chart.point.copy" => &["id"],

        "line.set_x1" | "line.set_x2" | "label.set_x" => &["id", "x"],
        "line.set_y1" | "line.set_y2" | "label.set_y" => &["id", "y"],
        "line.set_xy1" | "line.set_xy2" | "label.set_xy" => &["id", "x", "y"],
        "line.set_first_point"
        | "line.set_second_point"
        | "label.set_point"
        | "box.set_top_left_point"
        | "box.set_bottom_right_point" => &["id", "point"],
        "line.set_xloc" => &["id", "x1", "x2", "xloc"],
        "line.set_color"
        | "label.set_color"
        | "box.set_bgcolor"
        | "box.set_border_color"
        | "linefill.set_color" => &["id", "color"],
        "line.set_width" | "box.set_border_width" => &["id", "width"],
        "line.set_style" | "label.set_style" | "box.set_border_style" => &["id", "style"],
        "line.set_extend" | "box.set_extend" => &["id", "extend"],
        "line.get_price" => &["id", "x"],

        "label.set_xloc" => &["id", "x", "xloc"],
        "label.set_yloc" => &["id", "yloc"],
        "label.set_text" | "box.set_text" => &["id", "text"],
        "label.set_textcolor" => &["id", "textcolor"],
        "label.set_size" => &["id", "size"],
        "label.set_tooltip" => &["id", "tooltip"],
        "label.set_textalign" => &["id", "textalign"],

        "box.set_left" => &["id", "left"],
        "box.set_right" => &["id", "right"],
        "box.set_top" => &["id", "top"],
        "box.set_bottom" => &["id", "bottom"],
        "box.set_lefttop" => &["id", "left", "top"],
        "box.set_rightbottom" => &["id", "right", "bottom"],
        "box.set_xloc" => &["id", "left", "right", "xloc"],
        "box.set_text_color" => &["id", "text_color"],
        "box.set_text_size" => &["id", "text_size"],
        "box.set_text_halign" => &["id", "text_halign"],
        "box.set_text_valign" => &["id", "text_valign"],
        "box.set_text_wrap" => &["id", "text_wrap"],

        "label.set_text_font_family" | "box.set_text_font_family" => &["id", "text_font_family"],
        "label.set_text_formatting" | "box.set_text_formatting" => &["id", "text_formatting"],

        "table.cell_set_text" => &["table_id", "column", "row", "text"],
        "table.cell_set_text_color" => &["table_id", "column", "row", "text_color"],
        "table.cell_set_bgcolor" => &["table_id", "column", "row", "bgcolor"],
        "table.cell_set_width" => &["table_id", "column", "row", "width"],
        "table.cell_set_height" => &["table_id", "column", "row", "height"],
        "table.cell_set_text_halign" => &["table_id", "column", "row", "text_halign"],
        "table.cell_set_text_valign" => &["table_id", "column", "row", "text_valign"],
        "table.cell_set_text_size" => &["table_id", "column", "row", "text_size"],
        "table.cell_set_tooltip" => &["table_id", "column", "row", "tooltip"],
        "table.cell_set_text_font_family" => &["table_id", "column", "row", "text_font_family"],
        "table.cell_set_text_formatting" => &["table_id", "column", "row", "text_formatting"],
        "table.set_position" => &["table_id", "position"],
        "table.set_bgcolor" => &["table_id", "bgcolor"],
        "table.set_border_color" => &["table_id", "border_color"],
        "table.set_border_width" => &["table_id", "border_width"],
        "table.set_frame_color" => &["table_id", "frame_color"],
        "table.set_frame_width" => &["table_id", "frame_width"],
        "table.delete" => &["table_id"],

        "line.delete" | "line.copy" | "line.get_x1" | "line.get_y1" | "line.get_x2"
        | "line.get_y2" | "label.delete" | "label.copy" | "label.get_x" | "label.get_y"
        | "label.get_text" | "box.delete" | "box.copy" | "box.get_left" | "box.get_right"
        | "box.get_top" | "box.get_bottom" | "linefill.delete" | "linefill.get_line1"
        | "linefill.get_line2" | "polyline.delete" => &["id"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        BOX_MEMBERS, CHART_POINT_NAMES, LABEL_MEMBERS, LINE_MEMBERS, LINEFILL_MEMBERS,
        POLYLINE_MEMBERS, TABLE_MEMBERS,
    };
    use super::*;

    #[test]
    fn every_drawing_function_has_parameters() {
        let namespaces: [(&str, &[&str]); 6] = [
            ("line", LINE_MEMBERS),
            ("label", LABEL_MEMBERS),
            ("box", BOX_MEMBERS),
            ("linefill", LINEFILL_MEMBERS),
            ("table", TABLE_MEMBERS),
            ("polyline", POLYLINE_MEMBERS),
        ];
        let mut missing: Vec<String> = Vec::new();
        for (namespace, members) in namespaces {
            for member in members {
                let name = format!("{namespace}.{member}");
                if parameters(&name).is_empty() {
                    missing.push(name);
                }
            }
        }
        for name in CHART_POINT_NAMES.iter().copied().chain(["chart.point"]) {
            if parameters(name).is_empty() {
                missing.push(name.to_owned());
            }
        }
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn a_function_that_takes_a_handle_names_it_first() {
        assert_eq!(parameters("label.set_xy"), ["id", "x", "y"]);
        assert_eq!(
            parameters("table.cell_set_text"),
            ["table_id", "column", "row", "text"]
        );
        assert!(parameters("label.nope").is_empty());
    }

    #[test]
    fn the_point_form_ends_where_the_coordinate_form_does() {
        for name in ["line.new", "label.new", "box.new"] {
            let plain = parameters(name);
            let point = point_parameters(name).unwrap();
            assert_eq!(plain.last(), point.last(), "{name}");
        }
        assert!(point_parameters("table.new").is_none());
    }
}
