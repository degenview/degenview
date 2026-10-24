//! The Table tool: placing it, editing cells with the keyboard, resizing, rows and columns, and
//! pinning it to the screen.

use super::*;
use crate::model::table::{DEFAULT_COLUMN_WIDTH, DEFAULT_ROW_HEIGHT, TableData};
use menu::TableOp;

fn table_of(rig: &Rig) -> TableData {
    match rig.last().geometry() {
        Some(Geometry::Table(table)) => table.clone(),
        other => panic!("not a table: {other:?}"),
    }
}

/// A table just placed: its first cell is open for typing.
fn new_table() -> (Rig, u64) {
    let mut rig = Rig::with_tool(Tool::Table);
    rig.click(POINTS[0]);
    rig.settle();
    let id = rig.last().id();
    (rig, id)
}

/// A table placed and left, selected, the crosshair armed, no cell selected.
fn resting_table() -> (Rig, u64) {
    let (mut rig, id) = new_table();
    rig.key(Key::Escape);
    rig.settle();
    rig.key(Key::Escape);
    rig.settle();
    rig.tool = Tool::Cursor;
    rig.selected = vec![id];
    rig.settle();
    assert!(rig.layer.table_cell.is_none() && !rig.layer.is_editing_text());
    (rig, id)
}

fn type_text(rig: &mut Rig, text: &str) {
    rig.settle();
    rig.frame(vec![Event::Text(text.to_owned())]);
}

/// Where cell (`row`, `column`) of table `id` is on screen.
fn cell_center(rig: &mut Rig, id: u64, row: usize, column: usize) -> Pos2 {
    rig.screen_of(id)
        .grid
        .as_ref()
        .and_then(|grid| grid.cell_rect(row, column))
        .expect("a cell")
        .center()
}

// MARK: Placing and typing

#[test]
fn a_new_table_is_added_at_once_and_opens_its_first_cell_for_typing() {
    let (mut rig, id) = new_table();
    assert_eq!(rig.drawings.len(), 1);
    assert_eq!(rig.undo_steps, 1);
    assert!(rig.layer.is_editing_text());
    assert_eq!(rig.layer.table_cell, Some((id, 0, 0)));
    let table = table_of(&rig);
    assert_eq!((table.row_count(), table.column_count()), (3, 3));
    assert!(rig.px(table.at).distance(POINTS[0]) < 1.0);
    assert_eq!(rig.tool, Tool::Table, "the tool stays armed");
    type_text(&mut rig, "Entry");
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(table_of(&rig).cell(0, 0), "Entry");
    assert_eq!(rig.undo_steps, 2, "a cell is one edit");
    assert!(!rig.layer.is_editing_text());
    assert_eq!(
        rig.layer.table_cell,
        Some((id, 0, 0)),
        "the cell stays selected"
    );
}

#[test]
fn tab_commits_a_cell_and_opens_the_next_each_cell_one_step() {
    let (mut rig, id) = new_table();
    type_text(&mut rig, "a");
    rig.key(Key::Tab);
    assert!(rig.layer.is_editing_text());
    assert_eq!(rig.layer.table_cell, Some((id, 0, 1)));
    type_text(&mut rig, "b");
    rig.key(Key::Tab);
    type_text(&mut rig, "c");
    rig.key(Key::Enter);
    rig.settle();
    let table = table_of(&rig);
    assert_eq!(
        [table.cell(0, 0), table.cell(0, 1), table.cell(0, 2)],
        ["a", "b", "c"]
    );
    assert_eq!(rig.undo_steps, 1 + 3, "placing, then one step a cell");
    // Shift+Tab goes back, and an empty cell passed over is not a step.
    let (mut rig, id) = new_table();
    rig.key(Key::Tab);
    rig.settle();
    assert_eq!(rig.layer.table_cell, Some((id, 0, 1)));
    rig.modifiers = Modifiers::SHIFT;
    rig.key(Key::Tab);
    rig.modifiers = Modifiers::NONE;
    rig.settle();
    assert_eq!(rig.layer.table_cell, Some((id, 0, 0)));
    assert!(rig.layer.is_editing_text());
    assert_eq!(rig.undo_steps, 1, "nothing typed, nothing changed");
}

#[test]
fn tab_wraps_round_the_grid_in_reading_order() {
    let (mut rig, id) = resting_table();
    rig.layer.table_cell = Some((id, 2, 2));
    rig.key(Key::Tab);
    assert_eq!(rig.layer.table_cell, Some((id, 0, 0)), "past the last cell");
    rig.modifiers = Modifiers::SHIFT;
    rig.key(Key::Tab);
    rig.modifiers = Modifiers::NONE;
    assert_eq!(rig.layer.table_cell, Some((id, 2, 2)), "before the first");
    rig.layer.table_cell = Some((id, 0, 2));
    rig.key(Key::Tab);
    assert_eq!(rig.layer.table_cell, Some((id, 1, 0)), "on to the next row");
}

#[test]
fn escape_in_a_cell_keeps_what_was_there() {
    let (mut rig, _) = new_table();
    type_text(&mut rig, "never");
    rig.escape_consumed = false;
    rig.key(Key::Escape);
    rig.settle();
    assert!(!rig.layer.is_editing_text());
    assert_eq!(table_of(&rig).cell(0, 0), "");
    assert_eq!(rig.undo_steps, 1);
    assert!(rig.escape_consumed);
}

// MARK: A selected cell

#[test]
fn a_selected_cell_is_moved_with_the_arrows_edited_with_return_or_typing_and_cleared_with_delete() {
    let (mut rig, id) = resting_table();
    rig.layer.table_cell = Some((id, 0, 0));
    rig.key(Key::ArrowRight);
    rig.key(Key::ArrowDown);
    assert_eq!(rig.layer.table_cell, Some((id, 1, 1)));
    rig.key(Key::ArrowDown);
    rig.key(Key::ArrowDown);
    rig.key(Key::ArrowRight);
    rig.key(Key::ArrowRight);
    assert_eq!(
        rig.layer.table_cell,
        Some((id, 2, 2)),
        "held at the edge of the grid"
    );
    rig.key(Key::ArrowUp);
    rig.key(Key::ArrowLeft);
    assert_eq!(rig.layer.table_cell, Some((id, 1, 1)));
    // Typing starts editing the cell with what was typed.
    let steps = rig.undo_steps;
    rig.frame(vec![Event::Text("xyz".to_owned())]);
    assert!(rig.layer.is_editing_text());
    rig.settle();
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(table_of(&rig).cell(1, 1), "xyz");
    assert_eq!(rig.undo_steps, steps + 1);
    // Return edits what is there, adding to it.
    rig.key(Key::Enter);
    assert!(rig.layer.is_editing_text());
    type_text(&mut rig, "!");
    rig.key(Key::Enter);
    rig.settle();
    assert_eq!(table_of(&rig).cell(1, 1), "xyz!");
    // Delete clears the cell, not the table; again, with nothing to clear, is no step.
    let steps = rig.undo_steps;
    rig.key(Key::Delete);
    assert_eq!(table_of(&rig).cell(1, 1), "");
    assert_eq!(rig.undo_steps, steps + 1);
    assert_eq!(rig.drawings.len(), 1);
    rig.key(Key::Delete);
    assert_eq!(rig.undo_steps, steps + 1);
    // Escape lets go of the cell and keeps the table selected.
    rig.escape_consumed = false;
    rig.key(Key::Escape);
    assert!(rig.layer.table_cell.is_none() && rig.escape_consumed);
    assert_eq!(rig.selected, vec![id]);
}

#[test]
fn a_cell_of_the_selected_table_is_selected_by_a_click_and_edited_by_a_double_click() {
    let (mut rig, id) = resting_table();
    let at = cell_center(&mut rig, id, 1, 2);
    rig.click(at);
    assert_eq!(rig.layer.table_cell, Some((id, 1, 2)));
    assert!(!rig.layer.is_editing_text());
    rig.double_click(at);
    assert!(rig.layer.is_editing_text());
    // A table that is not selected is selected by the click first, no cell yet.
    let (mut rig, id) = resting_table();
    rig.selected.clear();
    rig.settle();
    let at = cell_center(&mut rig, id, 1, 1);
    rig.click(at);
    assert_eq!(rig.selected, vec![id]);
    assert!(rig.layer.table_cell.is_none());
    // Selecting another drawing, or none, lets go of the cell.
    rig.click(at);
    assert!(rig.layer.table_cell.is_some());
    rig.click(pos2(900.0, 30.0));
    rig.settle();
    assert!(rig.layer.table_cell.is_none());
}

#[test]
fn a_locked_table_cannot_be_edited() {
    let (mut rig, id) = resting_table();
    rig.drawings[0].set_locked(true);
    rig.settle();
    let at = cell_center(&mut rig, id, 0, 0);
    rig.double_click(at);
    assert!(!rig.layer.is_editing_text());
    assert!(rig.layer.table_cell.is_none());
}

#[test]
fn pasting_spreadsheet_text_fills_cells_from_the_selected_one_in_one_step() {
    let (mut rig, id) = resting_table();
    rig.layer.table_cell = Some((id, 1, 1));
    let steps = rig.undo_steps;
    rig.frame(vec![Event::Paste("a\tb\tc\nd\te\tf".to_owned())]);
    let table = table_of(&rig);
    assert_eq!(table.cell(1, 1), "a");
    assert_eq!(table.cell(2, 3), "f");
    assert_eq!((table.row_count(), table.column_count()), (3, 4));
    assert_eq!(rig.undo_steps, steps + 1);
    // Cut takes the cell's text away (the text goes to the clipboard).
    rig.frame(vec![Event::Cut]);
    assert_eq!(table_of(&rig).cell(1, 1), "");
    assert_eq!(rig.undo_steps, steps + 2);
    // Copy changes nothing in the table.
    rig.frame(vec![Event::Copy]);
    assert_eq!(rig.undo_steps, steps + 2);
}

#[test]
fn a_selected_cell_tells_the_workspace_to_leave_the_clipboard_keys_alone() {
    let (mut rig, id) = resting_table();
    rig.layer.table_cell = Some((id, 0, 0));
    rig.frame(Vec::new());
    assert!(crate::ui::take_table_cell_selected(&rig.ctx));
    assert!(
        !crate::ui::take_table_cell_selected(&rig.ctx),
        "it is taken once"
    );
    rig.layer.table_cell = None;
    rig.frame(Vec::new());
    assert!(!crate::ui::take_table_cell_selected(&rig.ctx));
}

// MARK: Resizing

#[test]
fn dragging_a_border_resizes_that_column_or_row_alone() {
    let (mut rig, id) = resting_table();
    let before = table_of(&rig);
    let screen = rig.screen_of(id);
    let column = screen
        .dividers
        .iter()
        .find(|d| d.part == Part::Column(0))
        .unwrap();
    let grab = column.a.lerp(column.b, 0.5);
    rig.drag(grab, grab + vec2(40.0, 0.0));
    let after = table_of(&rig);
    assert!(
        (after.columns[0] - (DEFAULT_COLUMN_WIDTH + 40.0)).abs() < 1.5,
        "{:?}",
        after.columns
    );
    assert_eq!(after.columns[1], DEFAULT_COLUMN_WIDTH);
    assert_eq!(after.rows, before.rows);
    assert_eq!(after.at, before.at, "a border drag does not move the table");
    assert_eq!(rig.undo_steps, 2);
    let screen = rig.screen_of(id);
    let row = screen
        .dividers
        .iter()
        .find(|d| d.part == Part::Row(1))
        .unwrap();
    let grab = row.a.lerp(row.b, 0.5);
    rig.drag(grab, grab + vec2(0.0, 25.0));
    let after = table_of(&rig);
    assert!(
        (after.rows[1] - (DEFAULT_ROW_HEIGHT + 25.0)).abs() < 1.5,
        "{:?}",
        after.rows
    );
    assert_eq!(after.rows[0], DEFAULT_ROW_HEIGHT);
    assert_eq!(rig.undo_steps, 3);
    // Dragged to nothing it stops at the narrowest.
    let screen = rig.screen_of(id);
    let column = screen
        .dividers
        .iter()
        .find(|d| d.part == Part::Column(1))
        .unwrap();
    let grab = column.a.lerp(column.b, 0.5);
    rig.drag(grab, grab - vec2(400.0, 0.0));
    assert_eq!(
        table_of(&rig).columns[1],
        crate::model::table::COLUMN_WIDTH_RANGE.0
    );
}

#[test]
fn the_corner_resizes_the_whole_table_and_the_top_left_moves_it() {
    let (mut rig, id) = resting_table();
    let before = table_of(&rig);
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Box(edit::BoxHandle::BottomRight))
        .unwrap()
        .pos;
    rig.drag(corner, corner + vec2(60.0, 30.0));
    let after = table_of(&rig);
    let (w0, h0) = before.size();
    let (w1, h1) = after.size();
    assert!(
        (w1 - (w0 + 60.0)).abs() < 2.0 && (h1 - (h0 + 30.0)).abs() < 2.0,
        "{w1} {h1}"
    );
    assert_eq!(after.at, before.at);
    // Every column grew by the same factor.
    assert!(
        (after.columns[0] / before.columns[0] - after.columns[2] / before.columns[2]).abs() < 0.01
    );
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Anchor(0))
        .unwrap()
        .pos;
    rig.drag(corner, corner + vec2(30.0, 20.0));
    let moved = rig.px(table_of(&rig).at) - rig.px(before.at);
    assert!((moved - vec2(30.0, 20.0)).length() < 1.5, "{moved:?}");
    assert_eq!(table_of(&rig).size(), after.size());
}

#[test]
fn a_row_grows_to_hold_wrapped_text_and_a_cell_never_spills() {
    let (mut rig, id) = resting_table();
    let plain = rig.screen_of(id).grid.clone().unwrap();
    let mut long = rig.last().clone();
    long.set_table_cell(
        1,
        1,
        &"a very long cell of words that has to wrap ".repeat(6),
    );
    rig.drawings = vec![long];
    let tall = rig.screen_of(id).grid.clone().unwrap();
    assert!(
        tall.ys[2] - tall.ys[1] > plain.ys[2] - plain.ys[1] + 20.0,
        "the row grew"
    );
    assert_eq!(
        tall.ys[1] - tall.ys[0],
        plain.ys[1] - plain.ys[0],
        "the others did not"
    );
    assert_eq!(tall.xs, plain.xs, "and no column moved");
    let screen = rig.screen_of(id);
    assert_eq!(screen.cell_texts.len(), 1);
    assert_eq!(screen.cell_texts[0].0, tall_rect(&tall, 1, 1));
}

fn tall_rect(grid: &super::super::shape::Grid, row: usize, column: usize) -> Rect {
    grid.cell_rect(row, column).unwrap()
}

// MARK: Rows and columns

fn run(rig: &mut Rig, id: u64, op: TableOp) {
    rig.run(Action::Table(op), &[id]);
}

#[test]
fn rows_and_columns_are_added_after_the_selected_cell_and_deleted_with_it() {
    let (mut rig, id) = resting_table();
    let mut numbered = rig.last().clone();
    for (index, (row, column)) in (0..3).flat_map(|r| (0..3).map(move |c| (r, c))).enumerate() {
        numbered.set_table_cell(row, column, &index.to_string());
    }
    rig.drawings = vec![numbered];
    rig.layer.table_cell = Some((id, 1, 1));
    let steps = rig.undo_steps;
    run(&mut rig, id, TableOp::AddRow);
    let table = table_of(&rig);
    assert_eq!(table.row_count(), 4);
    assert_eq!(
        (table.cell(1, 1), table.cell(2, 1), table.cell(3, 1)),
        ("4", "", "7")
    );
    assert_eq!(rig.undo_steps, steps + 1);
    run(&mut rig, id, TableOp::AddColumn);
    let table = table_of(&rig);
    assert_eq!(table.column_count(), 4);
    assert_eq!(
        (table.cell(0, 1), table.cell(0, 2), table.cell(0, 3)),
        ("1", "", "2")
    );
    run(&mut rig, id, TableOp::DeleteRow);
    run(&mut rig, id, TableOp::DeleteColumn);
    let table = table_of(&rig);
    assert_eq!((table.row_count(), table.column_count()), (3, 3));
    assert_eq!(
        table.cell(1, 1),
        "",
        "the row and column of the selected cell went"
    );
    assert!(rig.layer.table_cell.is_some_and(|(_, r, c)| r < 3 && c < 3));
    // Without a cell selected, new rows and columns go on the end, and nothing is deleted.
    rig.layer.table_cell = None;
    run(&mut rig, id, TableOp::AddRow);
    assert_eq!(table_of(&rig).row_count(), 4);
    let steps = rig.undo_steps;
    run(&mut rig, id, TableOp::DeleteRow);
    assert_eq!(table_of(&rig).row_count(), 4);
    assert_eq!(rig.undo_steps, steps, "no cell, no deletion");
}

#[test]
fn the_last_row_and_column_stay_and_the_grid_stops_at_its_limits() {
    let (mut rig, id) = resting_table();
    rig.layer.table_cell = Some((id, 0, 0));
    for _ in 0..2 {
        run(&mut rig, id, TableOp::DeleteRow);
        run(&mut rig, id, TableOp::DeleteColumn);
    }
    let table = table_of(&rig);
    assert_eq!((table.row_count(), table.column_count()), (1, 1));
    let steps = rig.undo_steps;
    run(&mut rig, id, TableOp::DeleteRow);
    run(&mut rig, id, TableOp::DeleteColumn);
    assert_eq!(rig.undo_steps, steps, "the last ones stay");
    for _ in 0..60 {
        run(&mut rig, id, TableOp::AddRow);
        run(&mut rig, id, TableOp::AddColumn);
    }
    let table = table_of(&rig);
    assert_eq!(
        (table.row_count(), table.column_count()),
        (
            crate::model::table::MAX_ROWS,
            crate::model::table::MAX_COLUMNS
        )
    );
    assert!(table.is_valid());
}

#[test]
fn a_right_click_selects_the_cell_the_menu_is_about() {
    let (mut rig, id) = resting_table();
    let at = cell_center(&mut rig, id, 2, 1);
    rig.right_click(at);
    assert_eq!(rig.layer.table_cell, Some((id, 2, 1)));
    assert!(rig.layer.menu.is_some());
}

// MARK: Pinning to the screen

#[test]
fn a_pinned_table_stays_where_it_is_on_the_screen_and_keeps_its_place_through_a_resize() {
    let (mut rig, id) = resting_table();
    let origin =
        |rig: &Rig, proj: &Projection| super::super::table_shapes::origin(&table_of(rig), proj);
    let corner = origin(&rig, &rig.proj());
    run(&mut rig, id, TableOp::TogglePin);
    let table = table_of(&rig);
    assert!(table.screen.is_some());
    assert!(
        rig.last().anchors().is_empty(),
        "a pinned table has no chart anchor"
    );
    assert!(
        origin(&rig, &rig.proj()).distance(corner) < 0.01,
        "pinning does not move it"
    );
    // The chart scrolled, zoomed and rescaled: a table at a chart point would have moved, the
    // pinned one has not.
    let scrolled = Projection {
        slot_width: 40.0,
        start: 3,
        ..rig.proj()
    };
    assert!(origin(&rig, &scrolled).distance(corner) < 0.01);
    let mut chart = table.clone();
    chart.screen = None;
    assert!(super::super::table_shapes::origin(&chart, &scrolled).distance(corner) > 20.0);
    // A smaller plot keeps the same share of it.
    let smaller = Projection {
        plot: Rect::from_min_size(PLOT.min, PLOT.size() * 0.5),
        ..rig.proj()
    };
    let place = table.screen.unwrap();
    let expected = PLOT.min + vec2(place.x * 500.0, place.y * 200.0);
    assert!(origin(&rig, &smaller).distance(expected) < 0.01);
    // Unpinning leaves it where it stands.
    run(&mut rig, id, TableOp::TogglePin);
    let table = table_of(&rig);
    assert!(table.screen.is_none());
    assert!(rig.px(table.at).distance(corner) < 0.5);
    assert_eq!(rig.last().anchors().len(), 1);
}

#[test]
fn a_pinned_table_is_moved_and_duplicated_in_screen_terms() {
    let (mut rig, id) = resting_table();
    run(&mut rig, id, TableOp::TogglePin);
    let before = table_of(&rig).screen.unwrap();
    let steps = rig.undo_steps;
    let grab = rig.body(id);
    rig.drag(grab, grab + vec2(50.0, 20.0));
    let after = table_of(&rig).screen.unwrap();
    assert!(
        (after.x - before.x - 50.0 / 1000.0).abs() < 0.002,
        "{before:?} {after:?}"
    );
    assert!((after.y - before.y - 20.0 / 400.0).abs() < 0.002);
    assert_eq!(rig.undo_steps, steps + 1);
    // The top-left handle moves it the same way.
    let screen = rig.screen_of(id);
    let corner = screen
        .handles
        .iter()
        .find(|h| h.part == Part::Anchor(0))
        .unwrap()
        .pos;
    rig.drag(corner, corner + vec2(-30.0, 0.0));
    let moved = table_of(&rig).screen.unwrap();
    assert!((moved.x - after.x + 30.0 / 1000.0).abs() < 0.002);
    // Duplicating offsets the copy and leaves it pinned.
    run_duplicate(&mut rig, id);
    assert_eq!(rig.drawings.len(), 2);
    let copy = match rig.drawings[1].geometry() {
        Some(Geometry::Table(table)) => table.screen.unwrap(),
        _ => panic!(),
    };
    assert!(copy.x > moved.x && copy.y > moved.y);
    // A table cannot be dragged off the plot: its place is held inside it.
    rig.drawings.truncate(1);
    rig.settle();
    let grab = rig.body(id);
    rig.drag(grab, grab + vec2(-900.0, -380.0));
    let held = table_of(&rig).screen.unwrap();
    assert!(held.x >= 0.0 && held.y >= 0.0);
}

fn run_duplicate(rig: &mut Rig, id: u64) {
    rig.run(Action::Duplicate, &[id]);
}

#[test]
fn a_table_round_trips_with_its_cells_sizes_and_place() {
    let (mut rig, id) = resting_table();
    let mut table = rig.last().clone();
    table.set_table_cell(0, 0, "Entry");
    table.set_table_cell(2, 2, "日本語\nline two");
    rig.drawings = vec![table];
    run(&mut rig, id, TableOp::TogglePin);
    let json = serde_json::to_value(rig.last()).unwrap();
    let back: Drawing = serde_json::from_value(json).unwrap();
    assert_eq!(&back, rig.last());
    // A table with a broken grid is read back as unreadable, never half-drawn.
    let mut broken = serde_json::to_value(rig.last()).unwrap();
    let cells = broken["Shape"]["geometry"]["table"]["cells"]
        .as_array_mut()
        .unwrap();
    cells.pop();
    let rows = vec![broken];
    let stored = serde_json::to_string(&rows).unwrap();
    #[derive(serde::Deserialize)]
    struct Holder {
        #[serde(with = "crate::model::drawings::lenient")]
        drawings: Vec<Drawing>,
    }
    let holder: Holder = serde_json::from_str(&format!("{{\"drawings\":{stored}}}")).unwrap();
    assert!(matches!(holder.drawings[0], Drawing::Unreadable(_)));
}
