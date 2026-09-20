//! read a file off the scope, or push one onto it.
//!
//! ```text
//! cargo run -p xdso-usb --example files -- get /sys.inf
//! cargo run -p xdso-usb --example files -- put ./local.txt /tmp/remote.txt
//! ```

use std::time::Instant;
use xdso_usb::Scope;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut scope = Scope::open()?;

    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["get", remote] => {
            let t = Instant::now();
            let data = scope.read_file(remote)?;
            let dt = t.elapsed();
            let rate = data.len() as f64 / dt.as_secs_f64() / 1024.0;
            println!("{remote}: {} bytes in {dt:?} ({rate:.0} KB/s)", data.len());
            println!("--- first 300 bytes ---");
            println!("{}", String::from_utf8_lossy(&data[..300.min(data.len())]));
        }
        ["put", local, remote] => {
            let data = std::fs::read(local)?;
            let t = Instant::now();
            scope.write_file(remote, &data)?;
            let dt = t.elapsed();
            println!("{local} -> {remote}: {} bytes in {dt:?}", data.len());
            // read it straight back to prove it survived
            let back = scope.read_file(remote)?;
            println!("read back {} bytes, identical: {}", back.len(), back == data);
        }
        _ => {
            eprintln!("usage: files get <remote> | files put <local> <remote>");
            std::process::exit(2);
        }
    }
    Ok(())
}
