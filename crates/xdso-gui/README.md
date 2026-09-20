# xdso-gui

[back to crates](../README.md) | [home](../../README.md)

the actual app. egui front end, binary is called `xdso`.

```bash
cargo run --release -p xdso-gui
cargo run --release -p xdso-gui -- --ch 1

```

## why egui

mostly packaging. the whole app is one file you can copy to another machine. on linux it links libc and dlopens x11 or wayland and libGL, all of which any desktop already has, so theres no `-dev` package needed to build and nothing to ship alongside. `ldd` on the release binary lists libc, libm, libgcc and the loader. thats it.

## views

| view        | module                    | notes                                      |
|-------------|---------------------------|--------------------------------------------|
| waveform    | `plot.rs`                 | the normal one. samples drawn here         |
| live screen | `app.rs`, `draw_screen`   | the scopes framebuffer as a texture, 1 fps |
| remote      | `panel.rs`                | no display at all, just the buttons        |
| hacker mode | `shell.rs`                | takes over the whole window                |

## layouts

three of them, and **scope** is the default because this is a scope and it should look like one.

| bit        | scope                            | remote                        | computer                     |
|------------|----------------------------------|-------------------------------|------------------------------|
| top strip  | same all three ways              | same                          | same                         |
| far left   | nothing                          | F0 to F6                      | nothing                      |
| centre     | the display                      | the front panel, full width   | the display                  |
| beside it  | F0 to F6                         | n/a                           | F0 to F6                     |
| next right | measurements                     | measurements                  | measurements                 |
| far right  | the fake front panel, `panel.rs` | nothing                       | nothing                      |
| bottom     | nothing                          | nothing                       | every other key, `keypad.rs` |

### remote mode

no display at all. for when the scope is sat right in front of you and you just want its buttons over here, which is most of the time honestly.

the point isnt saving screen space, its that **the poller drops to `Feed::Settings`** to match : settings only, no traces. a waveform pass is two or three round trips of 3200 samples each and in remote mode nobody is looking at any of it, so every button press was queueing behind data destined for the bin. without them the panel feels about as quick as the real front panel does.

the sidebar still updates, so you can watch volts per div and the trigger change as you press things. `g` doesnt toggle the live screen here like it does elsewhere, theres nowhere to put it. it takes you back to scope mode with the screen on instead, since asking for the scopes screen pretty clearly means you want a display.

the F keys normally line up against the graticule, so with no graticule they get a strip of their own down the left, which is the side of the panel theyre on anyway. capped at `F_COLUMN_MAX` tall because a full height F column on a big window looks daft and you have to reach for it.

LEDs :

| lamp             | where its state comes from                                     |
|------------------|-----------------------------------------------------------------|
| Run/Stop         | `TRIG-STATE`                                                    |
| Single           | `TRIG-MODE` being Single                                        |
| CH1, CH2         | `VERT-CHn-DISP`                                                 |
| Autoset          | amber for 5 s after you press it, then dark. the scope never tells us when its finished so we guess |
| Math             | none. its just a menu, it doesnt light up on the real scope     |


## settings

`settings.rs` is a front end for `xdso_usb::Tuning`. the command gap is the one that matters, see the usb crates readme for why its measured from the end of the last reply.

## hacker mode files

theres a get and a put next to the quick commands, as two text boxes. they predate the csv picker and could use one too, its just not done.

get is fast, its a real protocol command. put is **slow**, a few hundred bytes a second, because the scope has no upload command and it has to go through the shell in little base64 chunks. see the usb crates readme for why the chunks are so little.

hacker mode also refuses to send a line longer than `xdso_usb::MAX_COMMAND` and says so, because the scope would otherwise accept it, do nothing, and report success.

## Notes

if the scope isnt there the window still opens and shows what went wrong plus a try again button, rather than printing to a terminal nobody is looking at and then just ghosting the user.

if the link drops *after* weve had frames, we dont do that. the last frame stays on screen, traces and measurements and all, with a reconnect button in the top bar. youre usually looking at a scope because you want to see the last thing it caught, and throwing that away the moment the cable wobbles would be rude.
