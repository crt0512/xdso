# packaging

[back to index](../README.md)

bits for making xdso look like a real installed app rather than a binary you
run out of a folder. none of it is needed to *use* xdso, the binary works
fine on its own.

| file                | what it is                                                                    |
|---------------------|-------------------------------------------------------------------------------|
| `xdso.png`          | my icon, 64x64. its baked into the binary , see `app_icon()` in the gui crate |
| `xdso-logo.png`     | my banner, 64x128. its baked into the binary too                              |
| `xdso.desktop`      | desktop entry, so it turns up in your launcher with the right icon            |
| `99-dso5102p.rules` | udev rule, so you can open the scope without being root                       |

## installing it by hand

```
install -Dm755 target/release/xdso            ~/.local/bin/xdso
install -Dm644 packaging/xdso.png             ~/.local/share/icons/hicolor/64x64/apps/xdso.png
install -Dm644 packaging/xdso.desktop         ~/.local/share/applications/xdso.desktop
update-desktop-database ~/.local/share/applications 2>/dev/null || true
```

and the udev rule, or itll start up and then tell you it cant open the scope :

```
sudo install -Dm644 packaging/99-dso5102p.rules /etc/udev/rules.d/99-dso5102p.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

then unplug and replug the scope, the rule only applies when it enumerates.

## Notes

the icon is in the binary as well as on disk. the one in the binary is what
sets the window icon while its running, the one on disk is what your launcher
shows before you start it. they want to stay the same picture, so if you swap
one swap the other.

theres a test that fails if `xdso.png` stops being 64x64 8 bit rgba, since
`app_icon()` would otherwise quietly give up and youd get no window icon with
nothing to tell you why.
