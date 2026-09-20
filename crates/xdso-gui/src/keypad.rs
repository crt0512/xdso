//! computer mode : every front panel key as a plain grid of buttons.
//!
//! generated straight from keyprotocol.inf so it cant go stale. if hantek ever
//! adds a key to that file it turns up here with an auto generated label and
//! still works, because the code is just the line number.
//!
//! the plus and minus keys autorepeat if you hold them, everything else is a
//! plain click. [`crate::hold`] explains why thats not just a nicety.
//!
//! F1 to F7 are left out on purpose. they pick whatever the on screen menu is
//! offering, so they live next to the display in [`crate::panel::f_column`]
//! where they mean something, rather than floating in a grid. F8 stays here,
//! because on the real scope it isnt against the screen either.

use egui::{Button, Ui, Vec2};

use xdso_proto::keys;

use crate::hold;

const BTN: Vec2 = Vec2::new(96.0, 22.0);

/// draw the grid, return the key code if one got clicked
pub fn draw(ui: &mut Ui) -> Option<u8> {
    let mut pressed = None;
    let gap = 3.0;

    // work out how many buttons actually fit on a row and centre that block,
    // otherwise the grid hugs the left and leaves a ragged gap down the right
    // wherever the window width doesnt divide neatly
    let avail = ui.available_width();
    let per_row = ((avail + gap) / (BTN.x + gap)).floor().max(1.0);
    let block = per_row * BTN.x + (per_row - 1.0) * gap;
    let indent = ((avail - block) * 0.5).max(0.0);

    ui.horizontal(|ui| {
        ui.add_space(indent);
        // a wrapped layout constrained to exactly the block width, so it
        // breaks rows where we said it would rather than where the panel ends
        ui.allocate_ui_with_layout(
            Vec2::new(block, ui.available_height()),
            egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
            |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                for key in keys().iter().filter(|k| !is_side_menu_key(k.inf_name)) {
                    let mut button =
                        Button::new(egui::RichText::new(key.label.as_ref()).size(10.5));
                    if let Some(sc) = key.shortcut {
                        let tag = if sc == ' ' { "spc".into() } else { sc.to_string() };
                        button = button.shortcut_text(
                            egui::RichText::new(tag).size(10.0).color(crate::theme::SHORTCUT),
                        );
                    }
                    let r = ui.add_sized(BTN, button);
                    // the step keys fire on the press and keep firing while
                    // you hold them, the rest fire once on the release like
                    // any normal button. see [`crate::hold`]
                    let go = if hold::repeats(key.inf_name) {
                        hold::fired(ui, &r)
                    } else {
                        r.clicked()
                    };
                    if go {
                        pressed = Some(key.code);
                        // otherwise the button keeps keyboard focus and the
                        // next space bar press re-clicks it as well as firing
                        // Run/Stop
                        r.surrender_focus();
                    }
                }
            },
        );
    });
    pressed
}

/// the seven keys that sit down the side of the display.
///
/// `FN-7-KEY` is **not** one of them. on the real scope its off on its own in
/// the row under the menu block rather than against the screen, so it stays in
/// the grid. leaving it out of both places made it unreachable in computer
/// mode, which is exactly what happened the first time
fn is_side_menu_key(inf_name: &str) -> bool {
    inf_name
        .strip_prefix("FN-")
        .and_then(|rest| rest.strip_suffix("-KEY"))
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| n < 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_side_menu_keys_get_held_back() {
        assert!(is_side_menu_key("FN-0-KEY"));
        assert!(is_side_menu_key("FN-6-KEY"));
        // F8 lives in the grid, its not against the screen on the real scope
        assert!(!is_side_menu_key("FN-7-KEY"));
        assert!(!is_side_menu_key("FN-MLEFT-KEY"));
        assert!(!is_side_menu_key("FN-MZERO-KEY"));
        assert!(!is_side_menu_key("CT-RS-KEY"));
    }

    #[test]
    fn every_key_is_reachable_in_computer_mode() {
        // the grid plus the seven side menu buttons has to cover the lot,
        // otherwise theres a key you simply cannot press. F8 fell down exactly
        // this hole the first time round
        let in_grid: Vec<u8> =
            keys().iter().filter(|k| !is_side_menu_key(k.inf_name)).map(|k| k.code).collect();
        for k in keys() {
            assert!(
                in_grid.contains(&k.code) || k.code < 7,
                "{} is not reachable anywhere",
                k.inf_name
            );
        }
        assert_eq!(in_grid.len(), keys().len() - 7);
    }
}
