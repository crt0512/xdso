//! interactive shell on the scopes own linux.
//!
//! ```text
//! cargo run -p xdso-usb --example shell
//! echo 'uname -a' | cargo run -p xdso-usb --example shell
//! ```
//!
//! its busybox running as root, and its not a pty : no cwd, no env, nothing
//! persists between lines. this example doesnt pretend otherwise, the gui does
//! the faking.

use std::io::{BufRead, Write};

use xdso_usb::Scope;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut scope = Scope::open()?;
    eprintln!("connected. each line runs as one command, nothing persists. ctrl+d to quit");

    let stdin = std::io::stdin();
    loop {
        eprint!("dso$ ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            eprintln!();
            return Ok(());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if matches!(line, "exit" | "quit") {
            return Ok(());
        }
        match scope.shell(line) {
            Ok(out) => print!("{out}"),
            Err(e) => eprintln!("[{e}]"),
        }
        std::io::stdout().flush()?;
    }
}
