//! poke a real scope and print what comes back.
//!
//! this is the thing to run when something is broken and you dont know if its
//! the scope, the cable, udev or the code :
//!
//! ```text
//! cargo run -p xdso-usb --example probe
//! ```
//!
//! it reads settings, grabs a waveform off both channels and times the whole
//! lot. no gui, no rendering, just the transport.

use std::time::Instant;

use xdso_dsp::{Waveform, measure};
use xdso_proto::{Channel, MeasureKind, units::eng};
use xdso_usb::Scope;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // pass --screen to also grab the scopes own display. its slow, hence opt in
    let want_screen = std::env::args().any(|a| a == "--screen");
    let t0 = Instant::now();
    let mut scope = Scope::open()?;
    println!("opened in {:?}", t0.elapsed());

    let t = Instant::now();
    let s = scope.settings()?;
    println!("\nsettings read in {:?}", t.elapsed());

    let h = s.horizontal();
    let trig = s.trigger();
    println!(
        "timebase  {}/div   {}",
        h.seconds_div.map_or("?".into(), |v| eng(v, "s", 3)),
        if trig.running { "RUN" } else { "STOP" }
    );
    println!(
        "trigger   {} {} {}  level {}  counter {}",
        trig.kind.map_or("?", |k| k.label()),
        trig.source.map_or("?", |k| k.label()),
        trig.slope.map_or("?", |k| k.label()),
        trig.level,
        eng(trig.frequency_hz, "Hz", 5),
    );

    for ch in Channel::ALL {
        let c = s.channel(ch);
        println!(
            "{}       {}  {}/div  {}x  pos {}",
            ch.label(),
            if c.enabled { "on " } else { "off" },
            c.volts_div.map_or("?".into(), |v| eng(v, "V", 3)),
            c.probe.map_or("?".to_string(), |p| p.to_string()),
            c.position,
        );
    }

    let on: Vec<MeasureKind> = s
        .measurements()
        .iter()
        .filter_map(|m| m.kind)
        .filter(|k| !k.is_off())
        .collect();
    println!("measure   {:?}", on.iter().map(|k| k.label()).collect::<Vec<_>>());

    println!();
    for ch in Channel::ALL {
        if !s.channel(ch).enabled {
            println!("{}       off, skipping", ch.label());
            continue;
        }
        let t = Instant::now();
        let w = scope.samples(ch)?;
        let dt = t.elapsed();
        if w.is_empty() {
            println!("{}       no samples in {dt:?} (acquisition stopped?)", ch.label());
            continue;
        }
        let lo = w.iter().copied().min().unwrap_or(0);
        let hi = w.iter().copied().max().unwrap_or(0);
        println!(
            "{}       {} samples in {dt:?}, counts {lo}..{hi}, first 8 {:?}",
            ch.label(),
            w.len(),
            &w[..8.min(w.len())]
        );
    }

    if want_screen {
        let t = Instant::now();
        let shot = scope.screenshot()?;
        println!(
            "\nscreenshot  {}x{} in {:?}, {} bytes of rgb",
            shot.width,
            shot.height,
            t.elapsed(),
            shot.rgb.len()
        );
        // quick sanity check that its a picture and not 768 KB of nothing
        let lit = shot.rgb.iter().filter(|&&b| b != 0).count();
        println!("            {lit} non black subpixels");
    }

    // the two channel measurements, which only mean anything with something
    // on both inputs
    let both: Vec<Option<Vec<f32>>> = Channel::ALL
        .iter()
        .map(|&ch| {
            let c = s.channel(ch);
            let vdiv = c.volts_div?;
            let counts = scope.samples(ch).ok()?;
            (counts.len() == xdso_proto::SAMPLES).then(|| {
                counts
                    .iter()
                    .map(|&b| {
                        (b as f32 - xdso_proto::CENTRE_COUNT - c.position as f32)
                            / xdso_proto::COUNTS_PER_DIV
                            * vdiv as f32
                    })
                    .collect()
            })
        })
        .collect();

    if let (Some(a), Some(b)) = (&both[0], &both[1]) {
        let dt = s.sample_interval().unwrap_or(0.0);
        let wa = Waveform { volts: a, dt };
        let wb = Waveform { volts: b, dt };
        println!("\ntwo channel, ch1 against ch2 :");
        for kind in [
            MeasureKind::Delay12Rise,
            MeasureKind::Delay12Fall,
            MeasureKind::Frf,
            MeasureKind::Ffr,
            MeasureKind::Lrr,
        ] {
            let shown = measure(kind, &wa, Some(&wb)).map_or("--".into(), |m| m.format());
            println!("  {:<14} {shown}", kind.label());
        }
        println!("  (same signal on both inputs means the delays should read about zero)");
    } else {
        println!("\ntwo channel : need a real signal on both inputs, skipping");
    }

    // how fast can we actually go, really
    let t = Instant::now();
    let n = 10;
    for _ in 0..n {
        scope.settings()?;
    }
    println!("\n{n} settings reads: {:?} each", t.elapsed() / n);

    Ok(())
}
