//! hacker mode. a fake tty on the scopes own linux.
//!
//! RemoteShell runs one command as root and hands back stdout. thats it.
//! theres no session, no cwd, no environment, no history, nothing carries over
//! between calls. so everything that makes it feel like a terminal is faked up
//! here on the host.
//!
//! ## what the scope actually does with your line
//!
//! worth knowing before you go debugging something weird. a bare line behaves
//! oddly :
//!
//! | you send | you get back |
//! |---|---|
//! | `pwd` | `/`, always. every command starts in `/` |
//! | `echo a; echo b` | `b`. only the last bit of a `;` chain reports back |
//! | `cd /bin; pwd` | the **previous commands** output. `cd` leaves the reply buffer stale |
//! | `ls /nope` | nothing. stderr goes in the bin |
//!
//! wrap it in `sh -c '...'` though and you get a normal shell with all the
//! trimmings, so thats what we do. every line you type becomes :
//!
//! ```text
//! sh -c 'cd <cwd> && { <your line> ; } 2>&1'
//! ```
//!
//! which is how cd, pipes, quoting, `$VARS` and error messages all end up
//! working properly. the cwd is ours, we just keep reminding the scope of it.

use egui::{Key, RichText, ScrollArea, TextEdit, Ui};

use xdso_usb::ShellReply;


use crate::theme;

/// the shell caps replies somewhere between 4 and 16 KB, and asking for 16 KB
/// of output gets you nothing at all. so we suggest piping through head rather
/// than letting someone `cat` a firmware image and wonder why its empty
const BIG_OUTPUT_HINT: usize = 3500;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    Prompt(String),
    Out(String),
    Note(String),
    Bad(String),
}

/// what we are waiting on the scope for
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    /// an ordinary command, just print whatever comes back
    Run,
    /// a cd. the reply is the new working directory, or an error
    Cd,
    /// a file transfer. the reply is a sentence about how it went
    Transfer,
}

/// what the ui wants the app to do about it
pub enum Action {
    None,
    /// send this at the scope verbatim
    Run(String),
    /// fetch a file off the scope
    Download { remote: String, local: String },
    /// push one onto it
    Upload { local: String, remote: String },
    /// user typed exit, drop out of hacker mode
    Leave,
}

pub struct Shell {
    /// our pretend working directory. the scope has no idea
    cwd: String,
    lines: Vec<Line>,
    input: String,
    /// where the file transfer boxes are pointing
    remote_path: String,
    local_path: String,
    history: Vec<String>,
    /// where we are while arrowing back through history
    hist: Option<usize>,
    pending: Option<Pending>,
    focus_next: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Shell {
            cwd: "/".into(),
            lines: vec![
                Line::Note("xdso hacker mode. busybox on the scope, running as root.".into()),
                Line::Note(
                    "not a real tty : every line is a separate command and the cwd is faked \
                     on this end. `exit` to leave, `clear` to wipe this."
                        .into(),
                ),
            ],
            input: String::new(),
            remote_path: "/sys.inf".into(),
            local_path: "./sys.inf".into(),
            history: Vec::new(),
            hist: None,
            pending: None,
            focus_next: true,
        }
    }
}

impl Shell {
    pub fn is_busy(&self) -> bool {
        self.pending.is_some()
    }

    /// feed back whatever the poller got for us
    pub fn reply(&mut self, r: &ShellReply) {
        let Some(pending) = self.pending.take() else { return };
        match (&pending, &r.output) {
            (_, Err(e)) => self.lines.push(Line::Bad(format!("[{e}]"))),
            (Pending::Cd, Ok(out)) => {
                let out = out.trim_end_matches(['\n', '\r']);
                // pwd printed a path means the cd worked. anything else is the
                // shell complaining, and an empty reply means it failed quietly
                if out.starts_with('/') {
                    self.cwd = out.to_string();
                } else if out.is_empty() {
                    self.lines.push(Line::Bad("no such directory".into()));
                } else {
                    self.lines.push(Line::Bad(out.to_string()));
                }
            }
            (Pending::Transfer, Ok(msg)) => self.lines.push(Line::Note(msg.clone())),
            (Pending::Run, Ok(out)) => {
                if !out.is_empty() {
                    self.lines.push(Line::Out(out.trim_end_matches('\n').to_string()));
                }
                if out.len() > BIG_OUTPUT_HINT {
                    self.lines.push(Line::Note(
                        "(thats close to the reply size cap. pipe through head if things \
                         start coming back empty)"
                            .into(),
                    ));
                }
            }
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) -> Action {
        let mut action = Action::None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("scope shell").color(theme::HACK).size(13.0).monospace());
            ui.label(
                RichText::new(format!("cwd {} (faked)", self.cwd))
                    .color(theme::TEXT_FAINT)
                    .size(11.0)
                    .monospace(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("leave").clicked() {
                    action = Action::Leave;
                }
                if ui.button("clear").clicked() {
                    self.lines.clear();
                }
            });
        });

        // a few things worth having one click away
        ui.horizontal_wrapped(|ui| {
            for (label, cmd) in [
                ("uname", "uname -a"),
                ("uptime", "cat /proc/uptime"),
                ("mem", "free"),
                ("ps", "ps | head -20"),
                ("mounts", "mount"),
                ("dmesg", "dmesg | tail -20"),
                ("ls /", "ls -la /"),
                ("version", "cat /sys.inf"),
            ] {
                if ui.small_button(label).clicked() && !self.is_busy() {
                    action = self.submit(cmd.to_string());
                }
            }
        });
        // file transfer. no file dialog, because every crate that gives you
        // one drags in a system dependency and this binary has none
        ui.horizontal(|ui| {
            ui.label(RichText::new("file").color(theme::TEXT_FAINT).size(11.0).monospace());
            ui.label(RichText::new("scope").color(theme::TEXT_FAINT).size(11.0));
            ui.add(
                TextEdit::singleline(&mut self.remote_path)
                    .desired_width(180.0)
                    .font(egui::TextStyle::Monospace),
            );
            ui.label(RichText::new("here").color(theme::TEXT_FAINT).size(11.0));
            ui.add(
                TextEdit::singleline(&mut self.local_path)
                    .desired_width(180.0)
                    .font(egui::TextStyle::Monospace),
            );
            let idle = !self.is_busy();
            if ui
                .add_enabled(idle, egui::Button::new("get"))
                .on_hover_text("read it off the scope. fast, its a real protocol command")
                .clicked()
            {
                self.lines.push(Line::Prompt(format!(
                    "{}$ get {} -> {}",
                    self.cwd, self.remote_path, self.local_path
                )));
                self.pending = Some(Pending::Transfer);
                action = Action::Download {
                    remote: self.remote_path.clone(),
                    local: self.local_path.clone(),
                };
            }
            if ui
                .add_enabled(idle, egui::Button::new("put"))
                .on_hover_text(
                    "push it onto the scope. slow, a few hundred bytes a second, because it                      goes through the shell in little base64 chunks",
                )
                .clicked()
            {
                self.lines.push(Line::Prompt(format!(
                    "{}$ put {} -> {}",
                    self.cwd, self.local_path, self.remote_path
                )));
                self.pending = Some(Pending::Transfer);
                action = Action::Upload {
                    local: self.local_path.clone(),
                    remote: self.remote_path.clone(),
                };
            }
        });

        ui.separator();

        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height(ui.available_height() - row * 2.2)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for line in &self.lines {
                    let (text, colour) = match line {
                        Line::Prompt(s) => (s.as_str(), theme::HACK),
                        Line::Out(s) => (s.as_str(), theme::TEXT),
                        Line::Note(s) => (s.as_str(), theme::TEXT_FAINT),
                        Line::Bad(s) => (s.as_str(), theme::BAD),
                    };
                    ui.label(RichText::new(text).monospace().size(12.0).color(colour));
                }
                if self.is_busy() {
                    ui.label(
                        RichText::new("...").monospace().size(12.0).color(theme::TEXT_FAINT),
                    );
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{}$", self.cwd)).monospace().size(12.0).color(theme::HACK),
            );
            let enabled = !self.is_busy();
            let r = ui.add_enabled(
                enabled,
                TextEdit::singleline(&mut self.input)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("type a command"),
            );
            if self.focus_next && enabled {
                r.request_focus();
                self.focus_next = false;
            }
            if r.has_focus() {
                self.arrow_through_history(ui);
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let line = std::mem::take(&mut self.input);
                action = self.submit(line);
                self.focus_next = true;
            }
        });

        action
    }

    /// up and down walk back through what you have already run
    fn arrow_through_history(&mut self, ui: &Ui) {
        if self.history.is_empty() {
            return;
        }
        let (up, down) =
            ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
        if up {
            let next = match self.hist {
                None => self.history.len() - 1,
                Some(i) => i.saturating_sub(1),
            };
            self.hist = Some(next);
            self.input = self.history[next].clone();
        } else if down {
            match self.hist {
                Some(i) if i + 1 < self.history.len() => {
                    self.hist = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                }
                Some(_) => {
                    self.hist = None;
                    self.input.clear();
                }
                None => {}
            }
        }
    }

    fn submit(&mut self, line: String) -> Action {
        let line = line.trim().to_string();
        self.hist = None;
        if line.is_empty() {
            return Action::None;
        }
        self.lines.push(Line::Prompt(format!("{}$ {}", self.cwd, line)));
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }

        // the handful of things we deal with ourselves
        match line.as_str() {
            "exit" | "quit" | "logout" => return Action::Leave,
            "clear" => {
                self.lines.clear();
                return Action::None;
            }
            "help" => {
                self.lines.push(Line::Note(
                    "its busybox, so `busybox --list` is the real answer. cd and pipes and \
                     quoting all work. `exit` leaves, `clear` wipes the scrollback."
                        .into(),
                ));
                return Action::None;
            }
            _ => {}
        }

        // cd is ours to track, everything else just runs
        if line == "cd" || line.starts_with("cd ") {
            let target = line[2..].trim();
            let target = if target.is_empty() { "/" } else { target };
            self.pending = Some(Pending::Cd);
            // target goes in unquoted on purpose, so `cd "my dir"` and `cd $HOME`
            // behave like theyd behave in any shell
            return Action::Run(wrap(&format!(
                "cd {} && cd {target} && pwd",
                single_quote(&self.cwd)
            )));
        }

        let cmd = wrap(&format!("cd {} && {{ {line} ; }} 2>&1", single_quote(&self.cwd)));
        if cmd.len() > xdso_usb::MAX_COMMAND {
            // the scope would just ignore it and report success, which looks
            // exactly like the command having run and printed nothing
            self.lines.push(Line::Bad(format!(
                "too long. that works out at {} characters once wrapped and the scope                  silently drops anything over {}. split it up, or put it in a script and                  run that",
                cmd.len(),
                xdso_usb::MAX_COMMAND
            )));
            return Action::None;
        }
        self.pending = Some(Pending::Run);
        Action::Run(cmd)
    }
}

/// hand the whole thing to a real shell, because a bare RemoteShell line has
/// the daft semantics described at the top of this file
fn wrap(script: &str) -> String {
    // the 2>&1 is inside so a failed cd complains out loud instead of silently
    format!("sh -c {}", single_quote(&format!("{{ {script} ; }} 2>&1")))
}

/// wrap in single quotes for the shell, escaping any single quotes by closing,
/// escaping and reopening. the usual `'\''` dance
fn single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_nasty_input() {
        assert_eq!(single_quote("/etc"), "'/etc'");
        assert_eq!(single_quote("it's"), r"'it'\''s'");
        // a command full of quotes must still come out as one shell word
        let w = wrap(r#"echo "a" 'b'"#);
        assert!(w.starts_with("sh -c '"));
        assert!(w.ends_with('\''));
        assert!(!w[7..w.len() - 1].contains('\'') || w.contains(r"'\''"));
    }

    #[test]
    fn cd_is_tracked_locally() {
        let mut sh = Shell::default();
        let Action::Run(cmd) = sh.submit("cd /bin".into()) else {
            panic!("cd should have produced a command");
        };
        // the cwd comes out shell escaped, so look for the escaped form
        assert!(cmd.contains(r"cd '\''/'\'' && cd /bin && pwd"), "{cmd}");
        assert!(sh.is_busy());

        sh.reply(&ShellReply { cmdline: cmd, output: Ok("/bin\n".into()) });
        assert_eq!(sh.cwd, "/bin");
        assert!(!sh.is_busy());

        // and the next command runs from there
        let Action::Run(cmd) = sh.submit("ls".into()) else { panic!() };
        assert!(cmd.contains(r"cd '\''/bin'\''"), "{cmd}");
    }

    #[test]
    fn a_failed_cd_leaves_the_cwd_alone() {
        let mut sh = Shell::default();
        let _ = sh.submit("cd /nope".into());
        sh.reply(&ShellReply {
            cmdline: String::new(),
            output: Ok("sh: cd: can't cd to /nope\n".into()),
        });
        assert_eq!(sh.cwd, "/");
        assert!(matches!(sh.lines.last(), Some(Line::Bad(_))));
    }

    #[test]
    fn builtins_dont_go_to_the_scope() {
        let mut sh = Shell::default();
        assert!(matches!(sh.submit("exit".into()), Action::Leave));
        assert!(matches!(sh.submit("clear".into()), Action::None));
        assert!(!sh.is_busy());
    }

    #[test]
    fn history_walks_back() {
        let mut sh = Shell::default();
        let _ = sh.submit("one".into());
        sh.pending = None;
        let _ = sh.submit("two".into());
        assert_eq!(sh.history, vec!["one", "two"]);
    }
}
