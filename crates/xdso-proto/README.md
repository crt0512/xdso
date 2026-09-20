# xdso-proto

[back to crates](../README.md) | [home](../../README.md)

the wire protocol and everything you can work out without plugging anything in. no usb, no gui, no threads. just bytes in, structs out.

## the packet format

every packet looks the same in both directions :

| byte | what                                                                         |
|------|------------------------------------------------------------------------------|
| 0    | `0x53`, always                                                               |
| 1..3 | length, little endian. counts the command byte and the checksum, not these 3 |
| 3    | command going out, command echo coming back                                  |
| 4..  | payload                                                                      |
| last | checksum, low byte of the sum of every byte before it                        |

whats at the front of the payload depends on the command, which is annoying but thats the protocol :

| command          | out    | reply  | payload starts with                                          |
|------------------|--------|--------|--------------------------------------------------------------|
| Echo             | `0x00` | `0x80` | whatever you sent, straight back                             |
| ReadSettings     | `0x01` | `0x81` | nothing, its straight into the 208 bytes                     |
| ReadSampleData   | `0x02` | `0x82` | `[flag, channel]`                                            |
| ReadFile         | `0x10` | `0x90` | a null byte then the name, then flag framed chunks back      |
| RemoteShell      | `0x11` | `0x91` | the command line. **different packet type**, see below       |
| LockControlPanel | `0x12` | `0x92` | `[1,1]` locks, `[1,0]` unlocks, `[0,0]` "starts acquisition" |
| KeyTrigger       | `0x13` | -      | we send `[code, 0x01]`, the ack is useless                   |
| Screenshot       | `0x20` | `0xa0` | `[flag]`                                                     |
| ReadSystemTime   | `0x21` | `0xa1` | year as a little endian u16, then month, day, hour, min, sec |

the flag byte is `0x01` for "theres data in this one", `0x00` for "nothing this time", anything else for "thats the lot". careful : on sample data a `0x00` means keep reading, but on a screenshot anything that isnt `0x01` ends the transfer. the two loops are genuinely different, dont share the check.

`0x12` with `[0,0]` is the one to watch. it looks like "start acquisition" and the python called it that, but all it does is set TRIG-STATE to 3. the scope then insists its running while the acquisition engine sits idle and every sample read comes back empty. press the Run/Stop key instead.

also worth knowing : TRIG-STATE reads **3** when running, not 1.

ReadSampleData takes a channel index, and **3 is the math trace**. nothing documents that. 0 and 1 are the two inputs, 2 comes back as a copy of ch1, and 3 goes empty the moment MATH-DISP goes to 0, which is how it got pinned down. theres no volts per division field for math anywhere in the file though, so you can draw it but not scale it.

and theres nothing at all in here about cursors, so where they are is simply not knowable over usb.

### RemoteShell is a different packet

it goes out with `0x43` in byte 0 instead of `0x53`. the python called that the debug flag and thats as good a name as any. send it a normal `0x53` packet and it ignores you. `frame::encode_debug` does it for you.

why running a command needs its own packet type when every other command manages fine with `0x53` is anyones guess.

the semantics of what you send are odd enough that theyre written up at the top of `crates/xdso-gui/src/shell.rs` rather than here. short version : wrap everything in `sh -c '...'` and it behaves.


## the .inf tables

`inf/protocol.inf` and `inf/keyprotocol.inf` are copied off the scope and baked into the binary. they are the source of truth, not documentation :

- **protocol.inf** lists the 208 byte settings struct field by field, in
  order, with sizes. ReadSettings just dumps that struct at you and you walk
  the file to find out what each byte was - **keyprotocol.inf** lists the front panel keys. the code you send with
  KeyTrigger is the line number, 0 based. thats it, thats the whole scheme

verified on hardware : index 41 is `HZ-TBADD-KEY` and steps the timebase up, 40 steps it down, 19 is `CT-RS-KEY` and toggles run/stop.

## whats in here

| module     | what                                                            |
|------------|-----------------------------------------------------------------|
| `frame`    | build a command, slice up a reply, checksums                    |
| `inf`      | parse the .inf tables, resolve field offsets                    |
| `settings` | the 208 bytes -> typed views with real enums                    |
| `enums`    | Coupling, TrigType, MeasureKind and friends                     |
| `keys`     | key codes, nice labels, keyboard shortcuts                      |
| `units`    | the 1-2-5 tables, and `eng()` for scope style number formatting |

## gotchas

**sample data is signed on the wire.** value 0 means the graticule centre, so you xor `0x80` to get the offset binary counts (128 = centre) everything else uses. this caught me out for ages. the evidence : with ch1 flat at position -62 the raw bytes read 196.65 while the scope draws the trace at count 67.68. exactly 128 apart, and `196.65 ^ 0x80 = 68.65`. same on ch2. the offset shows up as +128 or -128 depending which side of centre you are, and those are the same thing mod 256, which is what the xor is really saying.

**positions are signed 16 bit, the wire is unsigned.** a position of -62 arrives as 65474. `settings::signed16` deals with it.

**`eng()` rounds before it picks the prefix, not after.** a period of 999.96us rounded to 4 significant figures is 1000.0us, and if you pick the prefix first youd print `1000us` where the scope prints `1.000ms`. theres a test for exactly that, dont "simplify" it.

**unknown enum values come back as None, not a guess.** the scope will happily hand you a byte thats not in any table we know about, and the ui shows `?` rather than making something up.

## Notes

the MeasureKind table is lifted verbatim out of the scopes own `/OurLanguages/English.lan`, which lists the measurements in enum order starting at Off. so thats the firmwares table, not a guess. whether we can actually compute each one is a separate question, see [xdso-dsp](../xdso-dsp/README.md).

the smaller enums (coupling, trigger type and so on) are less solid. the first couple of each are confirmed against what the screen shows, the rest follow the on screen menu order and are a reasonable guess.
