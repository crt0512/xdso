//! dumping the current traces to a csv, for when you want to do something we
//! dont do.
//!
//! the picker runs on its own thread. `rfd` blocks while the dialog is up,
//! and doing that on the ui thread would freeze the window for as long as
//! youre browsing. the samples get handed over to that thread so what lands
//! in the file is what was on screen when you clicked, not whatever has
//! scrolled past by the time you pick a name.

use std::path::Path;
use std::sync::mpsc::Sender;

use crate::sidebar::Volts;

/// ask where to put it, then write it. returns through `done` either way, so
/// the ui can say what happened
pub fn ask_and_save(volts: Volts, dt: f64, done: Sender<String>) {
    std::thread::Builder::new()
        .name("xdso-csv".into())
        .spawn(move || {
            let suggested = format!("xdso-{}.csv", stamp());
            let picked = rfd::FileDialog::new()
                .set_title("save waveform as csv")
                .set_file_name(&suggested)
                .add_filter("csv", &["csv"])
                .save_file();

            let msg = match picked {
                Some(path) => match write(&path, &volts, dt) {
                    // rfd hands back a path, the name is the interesting bit
                    Ok(n) => format!(
                        "saved {n} rows to {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    Err(e) => format!("csv failed: {e}"),
                },
                // rfd cant tell us apart a cancel from the portal not being
                // there at all, so say both
                None => "csv cancelled, or no file dialog available".into(),
            };
            let _ = done.send(msg);
        })
        .expect("spawning a thread should not fail");
}

/// write `time,ch1,ch2` to `path`, and say how many rows went in.
///
/// only the channels that are actually on get a column, and the header says
/// which is which. time starts at zero at the left edge of the graticule,
/// because the scope doesnt tell us where the trigger point sits in the
/// buffer and a made up offset would be worse than none
pub fn write(path: &Path, volts: &Volts, dt: f64) -> Result<usize, std::io::Error> {
    use std::io::Write;

    let file = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(file);

    let on: Vec<usize> = (0..2).filter(|&i| volts[i].is_some()).collect();
    if on.is_empty() {
        return Err(std::io::Error::other("no channels are on"));
    }

    write!(w, "time_s")?;
    for &i in &on {
        write!(w, ",ch{}_v", i + 1)?;
    }
    writeln!(w)?;

    let rows = on.iter().filter_map(|&i| volts[i].as_ref().map(Vec::len)).min().unwrap_or(0);
    for n in 0..rows {
        write!(w, "{:.9}", n as f64 * dt)?;
        for &i in &on {
            write!(w, ",{:.6}", volts[i].as_ref().expect("checked above")[n])?;
        }
        writeln!(w)?;
    }
    w.flush()?;
    Ok(rows)
}

/// seconds since the epoch, for the suggested filename. a real timestamp
/// would mean pulling in chrono for one line and i cant be bothered
fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
