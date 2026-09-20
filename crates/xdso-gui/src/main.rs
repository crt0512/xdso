//! fast live view for the hantek DSO5000P (DSO5072P/5102P/5202P).
//!
//! the obvious way to mirror a scope is to ask it for its screen, and it is a
//! trap. Screenshot ships the whole 800x480x16bpp framebuffer, 768000 bytes,
//! and the scope paces every ~10 KB protocol packet at ~12.7 ms no matter what
//! the host does. 77 packets, ~980 ms, every single frame. 1 fps, forever.
//!
//! so dont ship pixels. ReadSampleData gives you the same waveform the scope
//! is drawing in 3200 bytes per channel, and ReadSettings gives you its entire
//! front panel state in 208 bytes. render the display here instead and you get
//! 10 fps odd, plus measurements the scope wont even tell you over usb.
//!
//! press g when you actually need to see the scopes own menus. that still
//! costs you a second, but now its a second you asked for instead of one you
//! pay every frame.

mod app;
mod csv;
mod hold;
mod keypad;
mod layout;
mod panel;
mod plot;
mod png;
mod settings;
mod shell;
mod sidebar;
mod theme;
mod trail;

use xdso_proto::Channel;

const USAGE: &str = "\
xdso - live view for the hantek DSO5000P

usage:
  xdso [--ch 1|2]

options:
  --ch 1|2    poll only that channel. roughly doubles the frame rate since
              each channel costs its own command round trip
  -h, --help  this

keys:
  g           flip between the waveform and the scopes own screen at 1 fps
  p           save a png of this window
  esc         quit, or leave hacker mode if youre in it
  everything else is a front panel key, see the labels on the buttons.
  in hacker mode the keyboard belongs to the shell instead
";

fn main() -> eframe::Result {
    let force = match parse_args() {
        Ok(f) => f,
        Err(msg) => {
            eprintln!("{msg}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let mut viewport = eframe::egui::ViewportBuilder::default();
    if let Some(icon) = app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport: viewport
            // scope mode has a lot to fit in : display, F keys, measurements
            // and the whole fake front panel. give it room to start with
            .with_inner_size([1460.0, 860.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("xdso"),
        ..Default::default()
    };

    eframe::run_native("xdso", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, force)))))
}

/// baked in so theres still only one file to copy about. if it ever fails to
/// decode we just go without an icon rather than refusing to start over a
/// picture
fn app_icon() -> Option<eframe::egui::IconData> {
    let (rgba, width, height) = png::decode_rgba(png::ICON_BYTES)?;
    Some(eframe::egui::IconData { rgba, width, height })
}

/// theres one flag. pulling in a whole arg parsing crate for one flag felt
/// silly so heres twenty lines instead
fn parse_args() -> Result<Option<Channel>, String> {
    let mut args = std::env::args().skip(1);
    let mut force = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--ch" => {
                force = match args.next().as_deref() {
                    Some("1") => Some(Channel::Ch1),
                    Some("2") => Some(Channel::Ch2),
                    other => return Err(format!("--ch wants 1 or 2, got {other:?}")),
                }
            }
            other => return Err(format!("dont know what {other} means")),
        }
    }
    Ok(force)
}

