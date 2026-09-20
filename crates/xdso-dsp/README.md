# xdso-dsp

[back to crates](../README.md) | [home](../../README.md)

measurements, computed here rather than on the scope.

the scope can work all of these out itself, it just wont tell you the answers over usb ... great

everything in here works in volts and seconds, never adc counts. converting is the callers job.

## what it can do

| measurement                   | how                                                                     |
|-------------------------------|-------------------------------------------------------------------------|
| Mean, Pk-Pk, Minimum, Maximum | exactly what it says                                                    |
| Cyclic RMS, Period RMS        | root mean square over the whole capture                                 |
| Vtop, Vbase, Vamp, Vmid       | histogram top and base, see below                                       |
| Overshoot, Preshoot           | how far past the flat top or bottom the spike goes, as a % of amplitude |
| Frequency, Period             | median gap between mid level crossings                                  |
| +Pulse Width, -Pulse Width    | median gap from one crossing to the next opposite one                   |
| +Duty, -Duty                  | width over period                                                       |
| Rise Time, Fall Time          | 10% to 90%, both interpolated                                           |

and what it cant, which all return None and show as `--` :

- Delay1-2Rise, Delay1-2Fall, FRF, FFR, LRR. these need both channels at
  once and the api only takes one waveform. fixable, just not done - BWidth, FOVShoot, RPREShoot. idk what the scope means by these. TODO

## the two tricks worth knowing

**top and base are not max and min.** taking the max would be wrong the second theres any ringing on an edge, and theres always ringing on an edge. so split the trace at its midpoint, histogram each half into 32 bins, and take the centre of the busiest bin. thats where the trace actually spends its time, which is the flat bit. its also why Vamp and Pk-Pk can disagree and neither is wrong.

**crossings are interpolated.** at 3200 samples a whole sample of error on each edge is most of a percent on a frequency reading, so we linearly interpolate between the two samples either side of the level. the fractional part matters more than youd think.

everything that can be fooled by one glitchy edge takes a median rather than a mean.

## Notes

a flat line has no edges, so anything timing related gives up early and returns None instead of dividing by zero and printing `inf Hz`. the level measurements still work on a flat line obviously.

Period Mean returns the same number as Mean. the scope means "averaged over whole cycles only", but over 3200 samples of anything periodic thats the same answer to within noise, so it isnt worth the code. 