# xdso-usb

[back to crates](../README.md) | [home](../../README.md)

the only crate that knows usb exists. hands you a `Scope` with one method per command, and a `Poller` that drives one on its own thread.

pure rust usb via [nusb](https://docs.rs/nusb), not libusb. so theres nothing to `apt install` to build it and nothing to ship next to the binary.

## the two important numbers

**leave 20 ms between commands.** below about 15 ms the scope just doesnt answer, roughly half the time. thats measured, not folklore.

**measure that gap from the end of the last reply, not from when you sent the last command.** i got this wrong first time round and it was horrible to track down. a settings read takes 5 ms so the gap was still satisfied either way and everything looked fine, but a waveform read takes 50 ms, which "uses up" the whole gap, so the very next command went out too early and the scope silently binned it. symptom : timeouts that only ever happen straight after a waveform read. the python got this right by accident because it slept unconditionally.

## the startup dance

freshly claimed, the scope eats the first command. sometimes the first three. so `Scope::open` does two things about it :

1. `clear_halt` on both endpoints. the cdc ether driver we just kicked off
   left its data toggle sat in the scope while ours starts at zero, and a
   mismatched toggle means the scope quietly drops our first packet 2. asks for settings in a loop until it gets settings back, then hands over a
   device thats known to work

the python never noticed any of this because its poll loop retried forever and the failures scrolled past.

## threading

the poller thread owns the `Scope` outright. every method takes `&mut self`, which is not just rust being fussy : the scope has exactly one command in flight at a time and gets confused if you interleave them. the borrow checker enforcing that for free is a lot nicer than the mutex the python needed.

the ui pushes `Command`s down a channel and reads the latest `Snapshot` back, and never blocks on usb :

| command        | what happens                                                         |
|----------------|----------------------------------------------------------------------|
| `Press(code)`  | front panel key, jumps the queue so clicks feel instant              |
| `Screenshot`   | ~1 s of no waveform updates, then a picture                          |
| `SetFeed`      | waveforms, the scopes own screen at 1 fps, settings only, or nothing |
| `ForceChannel` | poll one channel instead of both, nearly doubles the rate            |
| `Tune`         | change the command gap and settings interval on the fly              |
| `Shell(line)`  | run it on the scopes linux, answer comes back on its own channel     |
| `Stop`         | pack it in                                                           |

settings get polled every 5th pass rather than every pass. they barely ever change and polling them every frame buys a whole extra round trip for nothing. every 5th is still about 2 Hz, plenty to keep up with somebody twiddling a knob. thats adjustable with `Tune`.

## feeds

the poller is doing exactly one of four things : pulling waveforms, pulling screen grabs, pulling just the settings, or nothing at all. theres one usb pipe and a screenshot monopolises it for a whole second, so they cant overlap. switching away from waveforms clears the stored traces so the ui isnt holding a stale one underneath something else.

| feed       | who asks for it | why                                                                      |
|------------|-----------------|--------------------------------------------------------------------------|
| `Waveform` | the normal case | 3200 samples a channel, ~13 fps                                          |
| `Screen`   | live screen     | the framebuffer is about a megabyte, so it gets the pipe to itself       |
| `Settings` | remote mode     | no display over there, so dont spend two round trips a frame filling one |
| `Paused`   | hacker mode     | so a shell command doesnt queue behind a waveform read                   |

`Settings` is paced at `SETTINGS_TICK` rather than run flat out. the settings barely ever change and hammering them would put the button presses right back behind a read, which is the exact thing that feed exists to avoid.

math costs its own round trip, so its only polled when the scope says the math channel is on.

## auto tuning

the poller can find the command gap itself, and does by default. a dropped command bumps it up a millisecond, and a couple of seconds of quiet lets it creep back down one.

it remembers the gap that failed and stays a step above it for 30 seconds rather than walking straight back into it. without that it sits on the edge oscillating forever, which is worse than useless : a dropout costs a read timeout plus a drain plus a retry, far more than the millisecond it was trying to save. the floor defaults to 18 ms for the same reason. the scope will answer at 15, but only just, and the dropouts cost more than the speed is worth.

## giving up, and coming back

after 25 failures in a row the poller thread stops rather than retrying forever. thats several seconds of solid nothing, so a transient hiccup recovers long before it but an unplugged cable never will. the thread stopping is what lets the gui notice, keep the last frame on screen and start looking for the scope again, instead of sitting there claiming to be live while every command quietly times out. the looking happens on its own thread, because opening takes the best part of a second and doing that on the ui thread would stutter every couple of seconds for as long as the scope was away.

## the shell

`Scope::shell` runs one command as root and hands back stdout. thats the whole api, and it is **not** a session : no cwd, no env, no history, nothing carries over between calls. the odd semantics of a bare command line are written up at the top of `crates/xdso-gui/src/shell.rs` along with the `sh -c` wrapping that makes it behave.

replies come back in one packet and cap out somewhere between 4 and 16 KB. ask for 16 KB of output and you get **nothing at all**, so pipe big things through `head`.

### the command length limit, which is worse

the line you send is limited too, and the limit is small and badly behaved. measured here :

| length          | what happens              |
|-----------------|---------------------------|
| under ~80 chars | fine                      |
| 84 to 96        | works about half the time |
| over 100        | fails every time          |

and "fails" is not always an error. sometimes the read times out, and sometimes **the command comes back looking like it succeeded while having done nothing at all**. thats presumably it being truncated mid quote and the shell giving up quietly. it is a horrible failure mode and it is why file uploads only move 48 bytes per round trip.

`MAX_COMMAND` is 110, conservatively under where it starts being flaky, and `Scope::shell` refuses anything longer instead of letting it disappear.

one more trap while were here : a **double quoted** `sh -c "..."` wrapper silently does nothing if the script contains `>>`. single quoting with the usual `'\''` escape works everywhere. no idea why.

## files

`read_file` is a real protocol command (ReadFile, 0x10) so its quick, around 26 KB/s on small files, and binary safe. it gives up on genuinely big files though, 131 KB is fine and 4.4 MB is not.

`write_file` has no protocol command behind it, because there isnt one. it base64s the data and appends it to a temp file over the shell a chunk at a time, then decodes it into place with busyboxs own `base64 -d`. base64 is used precisely because its alphabet has no shell metacharacters in it, so nothing needs escaping. given the command limit above thats 48 bytes a round trip, so **a few hundred bytes a second**. fine for a config file or a script, hopeless for anything big. it checks the length at both ends, because a chunk going missing would otherwise leave you a plausible looking short file.

## the probe example

```
cargo run -p xdso-usb --example probe
cargo run -p xdso-usb --example probe -- --screen
```

no gui, no rendering, just the transport. opens the scope, dumps settings, pulls a waveform off each channel, times the lot. this is the thing to run when something is broken and you dont know whether to blame the scope, the cable, udev or the code.

theres a shell one too :

```
cargo run -p xdso-usb --example shell
echo 'uname -a' | cargo run -p xdso-usb --example shell
```

it sends your line raw, with none of the `sh -c` wrapping the gui does, so its the quickest way to watch the protocol misbehave for yourself.

## Notes

the scope enumerates as a cdc ether gadget, so linux binds a network driver to interface 0 and we have to detach it. your `usb0` interface vanishes while xdso is running. it comes back when the process exits, which is also why the first command after startup gets eaten every single time.

reads ask for 16 KB, which has to be a multiple of the 512 byte max packet size and bigger than the ~10 KB the scope actually sends, so one read gets one whole protocol packet. all 3200 samples turn up in a single 3207 byte packet, with a 9 byte header packet before it and a 7 byte "done" after.
