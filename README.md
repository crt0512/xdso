# ![xdso](packaging/xdso-logo.png)
The Unix App your Hantek DSO5XXXP Series Osciloscope deserves

Only tested on hantek DSO5102P! (Should work on 5072P and 5202P too they talk the same protocol from what I was able to figure out)

this is the same idea as `dso_fast.py` one folder up, just not a pile of scripts any more.

## why cant i just mirror the scopes screen?

Well you can... but you probably dont want to because that gives you like 1fps because the Hantek DSO series use a 200Mhz CPU + alot of fragmenting over USB. So shipping a 800x480 16bpc 

xdso doesnt abuse the poor scopes CPU trying to ship raw pixels over usb (unless you want/need to). 
Why ship pixels over a dial up pipe, ReadSampleData gives you the exact same waveform the scope is drawing in 3200 bytes a channel, ReadSettings gives you its entire front panel state in 208, just draw the damn thing yourself. sameish picture, 60x fewer bytes.

measured on my 5102P :

| what                             |  bytes | command time | what the app usually gets |
|----------------------------------|-------:|-------------:|--------------------------:|
| Screenshit the whole framebuffer | 768000 |     ~1030 ms |                   1.0 fps |
| ReadSampleData of one channel    |   3200 |     45-80 ms |                  13.3 fps |
| ReadSampleData of two channels   |   6400 |   115-195 ms |                   6.9 fps |
| ReadSettings                     |    208 |        ~5 ms |                    40 fps |

the command times wander about a fair bit depending on what the scope is up to.

For example it gets noticeably slower per channel when both channels are switched on at the front panel, even if youre only asking for one of them, so yea uhm its poor one little arm core is infact crying very much (if you make it cry too much the watchdog dies and the scope reboops)

## running it

```
cargo run --release -p xdso-gui
cargo run --release -p xdso-gui -- --ch 1
```

the binary is called `xdso` and lands in `target/release/`. it has no runtime deps beyond what any desktop already has (libc, x11 or wayland, libGL, all dlopened) so you can just copy it somewhere and run it.

if it says permission denied on or loonix setup, youll prolly need the udev rule :

```
SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{idVendor}=="049f", ATTR{idProduct}=="505a", MODE="0666"
```

drop that in `/etc/udev/rules.d/99-dso5102p.rules` or something like that,
then run `sudo udevadm control --reload-rules && sudo udevadm trigger` and unplug then replug your scope.

## the three views

| view        | what youre looking at                                | rate    |
|-------------|------------------------------------------------------|---------|
| waveform    | samples pulled off the scope and drawn here          | ~13 fps |
| live screen | the scopes own framebuffer, so you can see its menus | 1 fps   |
| hacker mode | a root shell on the scopes own linux                 | n/a     |

`g` flips between the first two. live screen pauses the waveform polling while its up, because theres no point fighting over the one usb pipe, and hacker mode pauses it entirely so the shell gets the whole thing to itself.

the math trace draws too, in purple, whenever thats turned on to draw. theres no scale field for it pollable anywhere in the protocol so it can be drawn it but program cant tell you its amplitude directly (relative to other elements)

btw fft drawing doesnt work at all just dont ... i couldnt figure it tf out (not even with the help of llms)

## the three layouts

- **scope** arranges them like the real front panel, using my ascii drawings in [doc/ASCII.md](doc/ASCII.md). this is the default
- **remote** same thing but no drawing the screen, use when wanting to remote controll the scope without polling data from it
- **computer** big screen shitty button layout

knobs turn with the scroll wheel or a drag, use knobs for rough settings, buttons for light settings. the measurements sit between the display and the buttons.

the plus and minus keys autorepeat if you hold them down, in every layout. not as fast as youd like them to, mind : every repeat is a usb command and the scope wants its 20 ms of quiet between them like always.

## hacker mode

the scope runs a stripped down arm linux and RemoteShell runs commands on it as root. hacker mode is a terminal for that.
it fakes the feeling of using a real tty quiet a bit though deliberately. 

Thats because its just a bare RemoteShell : every command starts in `/`, only the last part of a `;` chain reports back, stderr vanishes, and `cd` leaves the reply buffer holding the *previous* commands output. so every line you type gets wrapped in `sh -c '...'` with a cwd we track on this side, which turns it back into a normal shell. cd, pipes, quoting, `$VARS` and error messages all work.

the reply caps out somewhere between 4 and 16 KB and returns **nothing at all** if you go over, so pipe big output through `head`. the command line you send is limited too, and much more tightly : over about 100 characters it fails, and sometimes "fails" means it reports success while doing nothing. hacker mode refuses anything that long rather than letting it vanish.

theres a get and a put for files. get is a real protocol command so its quick, put goes through the shell in base64 chunks and moves a few hundred bytes a second.

## saving things

- `save png` grabs the window and drops it next to wherever you ran the binary. 
- `save csv` puts up a file picker and writes the samples out as volts, one row per sample with a column per enabled channel.

## auto tuning

the command gap is the quiet time between commands and its the main thing that decides your frame rate. the app tunes it for you : a dropped command bumps it up a millisecond, a couple of seconds of quiet lets it creep back down. the button in the top bar shows what its settled on, and goes amber if the scope has been wanting more room than you allowed for a while, at which point clicking it accepts the number.

the floor is 18 ms rather than the 15 the scope will technically answer at if doing very little, a dropped command costs far more than the millisecond it saves.

## settings

the command gap is the one number that really matters. its the quiet time between commands, measured from the end of the last reply, and below about 15 ms the scope just stops answering about half the time. 20 ms is the default and roughly the sweet spot. the settings window lets you move it and tells you what it should buy you.

you can also change how often the front panel state gets reread. it barely ever changes so every 5th pass is plenty.

## keys

every front panel button on the scope is clickable and most have a keyboard shortcut, shown on the button itself. the host only keeps three for itself :

| key   | what                                                |
|-------|-----------------------------------------------------|
| `g`   | flip between the waveform and the scopes own screen |
| `p`   | save a png of the window                            |
| `esc` | quit, or leave hacker mode if youre in it           |

in hacker mode the keyboard belongs to the shell, otherwise typing `ls` would press the CH1 menu and then Single Seq.

## whats in here

| crate                                     | what it does                                                                         |
|-------------------------------------------|--------------------------------------------------------------------------------------|
| [xdso-proto](crates/xdso-proto/README.md) | the wire protocol, settings decode, key table, unit formatting. no i/o at all        |
| [xdso-dsp](crates/xdso-dsp/README.md)     | measurements. frequency, duty, rise time and friends, computed here not on the scope |
| [xdso-usb](crates/xdso-usb/README.md)     | usb transport and the poller thread                                                  |
| [xdso-gui](crates/xdso-gui/README.md)     | the app. egui front end                                                              |

split this way because the interesting bits (protocol layout, measurement maths) are all testable with no scope plugged in, and i would rather not have usb or a window anywhere near them. `cargo test` runs 72 tests and needs no hardware.

see [crates/README.md](crates/README.md) for a bit more on each, theres a [packaging](packaging/README.md) folder with the icon and a desktop entry aswell as instructions on how to install it as app on loonix.

## poking at a real scope without a gui

theres a probe tool that skips the gui entirely :

```
cargo run -p xdso-usb --example probe
cargo run -p xdso-usb --example probe -- --screen
```

it opens the scope, dumps the settings, pulls a waveform off each channel and times everything. run this first when something is broken, it tells you whether the problem is the scope, the cable, udev or the code.

and a shell, if you want hacker mode without the gui :

```
cargo run -p xdso-usb --example shell
echo 'uname -a' | cargo run -p xdso-usb --example shell
```

that one is raw, it doesnt do the `sh -c` wrapping the gui does, so have fun with the goofy semantics lol.

## disclaimer :
I believe in that if LLM's were used for a project it should be clearly stated where and for what! For this project this means :

- All code comments have been fully sanetize using Large Language Models, I write literal garbage comments, very little if at all, curse in Swiss German through out all of my code and insult peoples mothers while writing it.
- Obviously bugs and larger feature additions have had LLM's help along many times but the code was inspected and should mostly be decent. (except some of the FFT related stuff that i couldnt wrap my head around at 4am)
- Tests were fully written by an LLM as I was way too lazy to write them myself for this tiny side project, only inspected quicly

## not done yet

- BWidth, FOVShoot and RPREShoot, because idk what the scope actually means by those. they show `--`
- cursors cant be done at all : theres no cursor field anywhere in the settings blob, so the scope never tells us where they are
- FFT drwaing
- Multiple Scope Support (if you have more than one connected, cant test only have one so i didnt feel like schizophreniaing something for that)

## notes

the scope is a little arm linux box (busybox, `uname -a` says `Linux Hantek 3.2.35 ... armv5tejl`) running `dso.exe`, and the usb protocol is the only way in i found sofar. it enumerates as a cdc ether gadget so linux binds a network driver to it, which we kick off on startup (conflicting usb headers with something that used to have notworking with it). that means your `usb0` interface disappears while xdso is running. it comes back.
