//! press and hold to autorepeat.
//!
//! the position and volts keys on a scope are the kind you hold down, not the
//! kind you click forty times. egui only hands you a click on the *release*
//! though, so a held button does exactly nothing until you let go, which is
//! the opposite of what you want.
//!
//! so this keeps a tiny per widget timer in egui memory instead : fire once
//! the moment the button goes down, wait [`DELAY`], then keep firing every
//! [`RATE`] until its let go.
//!
//! [`RATE`] isnt a free choice. every repeat is a usb command and the scope
//! wants ~20 ms of quiet between commands, so going much faster than this just
//! builds a queue that carries on stepping after youve taken your finger off,
//! which feels broken.

use egui::{Id, Response, Ui};

/// how long you hold before it starts repeating.
///
/// long enough that an ordinary click never repeats by accident, short enough
/// that holding it feels like holding it rather than like nothing happening
pub const DELAY: f64 = 0.3;

/// and how long between repeats once its going. 20 a second
pub const RATE: f64 = 0.05;

/// does this key want autorepeat, going by its name in keyprotocol.inf.
///
/// every `...SUB-KEY` and `...ADD-KEY` is a step of something : channel
/// position, volts per div, timebase, horizontal position, trigger level.
/// all of them are things you nudge repeatedly. the menu and mode keys are
/// not, and a menu key that fired twenty times a second would be a menace
pub fn repeats(inf_name: &str) -> bool {
    inf_name.ends_with("SUB-KEY") || inf_name.ends_with("ADD-KEY")
}

/// what we remember about a button thats currently down
#[derive(Clone, Copy, Default)]
struct Held {
    /// when it went down
    since: f64,
    /// when we last fired
    last: f64,
    /// was it down last frame. tells a fresh press from a continuing one
    down: bool,
}

/// true on the frames this button should fire, for a plain widget
pub fn fired(ui: &Ui, resp: &Response) -> bool {
    fired_at(ui, resp.id, resp.is_pointer_button_down_on())
}

/// same, but for something that isnt its own widget.
///
/// the knobs hit test their two step squares inside one big response, so they
/// have no ids of their own to hang the timer off. they pass a child id and
/// work out `down` themselves
pub fn fired_at(ui: &Ui, id: Id, down: bool) -> bool {
    let now = ui.input(|i| i.time);
    let mut st: Held = ui.data(|d| d.get_temp(id)).unwrap_or_default();

    if !down {
        if st.down {
            st.down = false;
            ui.data_mut(|d| d.insert_temp(id, st));
        }
        return false;
    }

    // a held button isnt a pointer event, so nothing would repaint and the
    // timer would just sit there. ask for the next tick ourselves
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RATE));

    if !st.down {
        // fresh press. fire straight away, so a single click still feels
        // instant rather than waiting on the release
        ui.data_mut(|d| d.insert_temp(id, Held { since: now, last: now, down: true }));
        return true;
    }
    if now - st.since >= DELAY && now - st.last >= RATE {
        st.last = now;
        ui.data_mut(|d| d.insert_temp(id, st));
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_step_keys_repeat() {
        // the things you nudge
        assert!(repeats("VT-CH1-PSUB-KEY"));
        assert!(repeats("VT-CH2-VBADD-KEY"));
        assert!(repeats("HZ-TBSUB-KEY"));
        assert!(repeats("TG-PADD-KEY"));
        // and the things you very much do not want firing twenty times a second
        assert!(!repeats("VT-CH1-PZERO-KEY"));
        assert!(!repeats("CT-AUTOSET-KEY"));
        assert!(!repeats("FN-0-KEY"));
        assert!(!repeats("CT-RS-KEY"));
        assert!(!repeats("TG-PHALF-KEY"));
    }

    #[test]
    fn every_step_key_on_the_scope_is_covered() {
        // if hantek ever spells one differently this catches it. thats the
        // eight position steps (two channels, horizontal, trigger level), the
        // four volts per div ones and the timebase pair
        let n = xdso_proto::keys().iter().filter(|k| repeats(k.inf_name)).count();
        assert_eq!(n, 14, "expected 14 step keys, got {n}");
    }
}
