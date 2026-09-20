# crates

[back to index](../README.md)

four crates, stacked. each one only knows about the ones below it.

| crate | depends on | needs hardware to test |
|---|---|---|
| [xdso-proto](xdso-proto/README.md) | nothing | no |
| [xdso-dsp](xdso-dsp/README.md) | xdso-proto | no |
| [xdso-usb](xdso-usb/README.md) | xdso-proto, nusb | yes, for the example |
| [xdso-gui](xdso-gui/README.md) | all of them, eframe, png | yes |

the split isnt architecture astronautics, it buys two real things. one, the protocol layout and the measurement maths get unit tested with no scope plugged in, which is most of the code. two, if egui turns out to be the wrong call the only crate that gets thrown away is xdso-gui.

## Notes

`xdso-proto` embeds the scopes own `.inf` tables with `include_str!`, so the settings layout and the key numbering come from hantek rather than from somebody transcribing them into a rust file. if you have a different firmware and yours disagree, swap the files in `xdso-proto/inf/` and everything downstream follows.
