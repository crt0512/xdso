//! small layout helpers that egui doesnt give us.

use egui::Ui;

/// draw `body` vertically centred inside `room` points of height.
///
/// egui lays things out top down and only finds out how tall something was
/// after its drawn it, so theres no honest way to centre in one pass. we
/// remember how tall it came out last frame and centre on that instead, which
/// costs one frame of lag whenever the content changes size. nobody will ever
/// see it.
///
/// on the very first frame theres nothing remembered, so we dont centre at
/// all rather than guess and have everything jump a frame later.
pub fn center_v<R>(ui: &mut Ui, salt: &str, room: f32, body: impl FnOnce(&mut Ui) -> R) -> R {
    let id = ui.id().with(salt);
    let last: Option<f32> = ui.data(|d| d.get_temp(id));
    // content taller than the room means no padding and it just scrolls
    let pad = last.map_or(0.0, |h| ((room - h) * 0.5).max(0.0));
    ui.add_space(pad);
    let r = ui.vertical(body);
    ui.data_mut(|d| d.insert_temp(id, r.response.rect.height()));
    r.inner
}
