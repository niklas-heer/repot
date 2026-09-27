//! Real terminal tests for the embedded picker and caller-shell handoff.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("temporary home");
            let root = home.path().join("projects");
            fs::create_dir(&root).expect("projects");
            fs::create_dir(home.path().join("tmp")).expect("temporary handoffs");
            Self { home, root }
        }

        fn repo(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            let output = Command::new("git")
                .args(["init", "--quiet"])
                .arg(&path)
                .env("HOME", self.home.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.home.path().join("gitconfig"))
                .output()
                .expect("git init");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            path.canonicalize().expect("canonical repository")
        }

        fn terminal(&self, shell: &str, body: &str, wrapper: bool) -> Session {
            let mut script = if wrapper {
                let output = Command::new(env!("CARGO_BIN_EXE_repot"))
                    .args(["shell-init", shell])
                    .output()
                    .expect("shell init");
                assert!(output.status.success());
                String::from_utf8(output.stdout).expect("UTF-8 wrapper")
            } else {
                String::new()
            };
            script.push_str(body);
            let script_path = self.home.path().join("test-script");
            fs::write(&script_path, script).expect("test script");
            let mut command = CommandBuilder::new(shell);
            command.arg(&script_path);
            command.cwd(self.home.path());
            command.env("HOME", self.home.path());
            command.env("XDG_CONFIG_HOME", self.home.path().join("config"));
            command.env("XDG_STATE_HOME", self.home.path().join("state"));
            command.env("GHQ_ROOT", &self.root);
            command.env("GIT_CONFIG_NOSYSTEM", "1");
            command.env("GIT_CONFIG_GLOBAL", self.home.path().join("gitconfig"));
            command.env("TMPDIR", self.home.path().join("tmp"));
            command.env("TERM", "xterm-256color");
            command.env("NO_COLOR", "1");
            command.env_remove("REPOT_CD_FILE");
            command.env("REPOT_TEST_RESULT", self.home.path().join("result"));
            command.env("REPOT_TEST_STATUS", self.home.path().join("status"));
            let binary_directory = Path::new(env!("CARGO_BIN_EXE_repot"))
                .parent()
                .expect("binary directory");
            let path = env::join_paths(
                std::iter::once(binary_directory.to_path_buf())
                    .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
            )
            .expect("test PATH");
            command.env("PATH", path);
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: 24,
                    cols: 100,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("open PTY");
            let child = pair.slave.spawn_command(command).expect("spawn shell");
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().expect("PTY reader");
            let writer = pair.master.take_writer().expect("PTY writer");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                while let Ok(count) = reader.read(&mut buffer) {
                    if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            });
            Session {
                errors: self.home.path().join("errors"),
                child,
                master: pair.master,
                writer,
                receiver,
                output: Vec::new(),
                stream: Vec::new(),
                answered: 0,
            }
        }
    }

    struct Session {
        errors: PathBuf,
        child: Box<dyn Child + Send + Sync>,
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        receiver: Receiver<Vec<u8>>,
        output: Vec<u8>,
        stream: Vec<u8>,
        answered: usize,
    }

    impl Session {
        /// Answer cursor position queries like a terminal emulator would; the
        /// inline picker anchors itself where the cursor is.
        fn record(&mut self, bytes: Vec<u8>) {
            self.stream.extend(&bytes);
            self.output.extend(bytes);
            let queries = self
                .stream
                .windows(4)
                .filter(|window| *window == b"\x1b[6n")
                .count();
            while self.answered < queries {
                self.answered = self.answered.saturating_add(1);
                self.send(b"\x1b[1;1R");
            }
        }

        fn until(&mut self, needle: &[u8]) {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(15))
                .expect("deadline");
            while !self
                .output
                .windows(needle.len())
                .any(|window| window == needle)
            {
                let timeout = deadline.saturating_duration_since(Instant::now());
                match self.receiver.recv_timeout(timeout) {
                    Ok(bytes) => self.record(bytes),
                    Err(error) => panic!(
                        "terminal did not show {:?}: {error}; output: {}",
                        String::from_utf8_lossy(needle),
                        String::from_utf8_lossy(
                            &[
                                self.output.clone(),
                                fs::read(&self.errors).unwrap_or_default()
                            ]
                            .concat()
                        )
                    ),
                }
            }
        }

        fn send(&mut self, keys: &[u8]) {
            self.writer.write_all(keys).expect("send keys");
            self.writer.flush().expect("flush keys");
        }

        fn finish(&mut self) {
            self.until(b"REPOT_TEST_DONE");
            assert!(self.child.wait().expect("shell status").success());
            assert!(
                !self
                    .output
                    .windows(8)
                    .any(|window| window == b"\x1b[?1049h"),
                "the picker stays inline instead of taking over the screen"
            );
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if let Some(pid) = self
                .child
                .process_id()
                .and_then(|pid| i32::try_from(pid).ok())
            {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn terminal_settings(path: &Path) -> Vec<String> {
        let fields = fs::read_to_string(path)
            .expect("terminal settings")
            .trim()
            .split(':')
            .map(str::to_owned)
            .collect();
        #[cfg(target_os = "macos")]
        let fields = darwin_terminal_settings(fields);
        fields
    }

    #[cfg(target_os = "macos")]
    fn darwin_terminal_settings(mut fields: Vec<String>) -> Vec<String> {
        // XNU termios.h defines PENDIN (0x20000000) as state; tty.c sets it
        // when restoring ICANON and clears it while processing pending input.
        // GNU stty's display_recoverable prints iflag:oflag:cflag:lflag:cc...;
        // BSD stty uses gfmt1 with labelled fields. Preserve every other bit.
        let (index, prefix) = if fields.first().is_some_and(|field| field == "gfmt1") {
            (
                fields
                    .iter()
                    .position(|field| field.starts_with("lflag="))
                    .expect("BSD stty local flags"),
                "lflag=",
            )
        } else {
            assert_eq!(
                fields.len(),
                nix::libc::NCCS.saturating_add(4),
                "GNU stty field count"
            );
            assert!(
                fields
                    .iter()
                    .all(|field| u64::from_str_radix(field, 16).is_ok()),
                "GNU stty hexadecimal fields"
            );
            (3, "")
        };
        let field = fields.get_mut(index).expect("stty local flags");
        let flags = u64::from_str_radix(field.strip_prefix(prefix).expect("local flag prefix"), 16)
            .expect("terminal local flags");
        *field = format!("{prefix}{:x}", flags & !nix::libc::PENDIN);
        fields
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn bsd_and_gnu_stty_comparison_ignores_only_darwin_pending_input_state() {
        let normalize =
            |text: &str| darwin_terminal_settings(text.split(':').map(str::to_owned).collect());
        for plain in [
            "gfmt1:cflag=4b00:iflag=2b02:lflag=5cb:oflag=3",
            "2b02:3:4b00:5cb:4:ff:ff:7f:17:15:12:ff:3:1c:1a:19:11:13:16:f:1:0:14:ff",
        ] {
            assert_eq!(
                normalize(plain),
                normalize(&plain.replace("5cb", "200005cb"))
            );
            assert_ne!(
                normalize(plain),
                normalize(&plain.replace("5cb", "5c3")),
                "echo changes must still fail"
            );
            assert_ne!(
                normalize(plain),
                normalize(&plain.replace("2b02", "20002b02")),
                "input flags must still match exactly"
            );
        }
    }

    #[test]
    fn picker_filters_selects_with_redirected_output_and_restores_terminal() {
        let fixture = Fixture::new();
        fixture.repo("host/owner/alpha");
        let wanted = fixture.repo("host/owner/beta");
        let mut terminal = fixture.terminal("bash", "\nstty -g > before\nrepot jump > result 2> errors\nstty -g > after\nprintf REPOT_TEST_DONE\n", false);
        terminal.until(b"esc cancel");
        terminal.send(b"beta\r");
        terminal.finish();
        assert_eq!(
            fs::read(fixture.home.path().join("result")).expect("selection"),
            [wanted.as_os_str().as_encoded_bytes(), b"\n"].concat()
        );
        assert!(
            fs::read(fixture.home.path().join("errors"))
                .expect("stderr")
                .is_empty()
        );
        assert_eq!(
            terminal_settings(&fixture.home.path().join("before")),
            terminal_settings(&fixture.home.path().join("after"))
        );
    }

    #[test]
    fn escape_and_ctrl_c_cancel_without_handoff_and_restore_terminal() {
        for cancel in [b"\x1b".as_slice(), b"\x03".as_slice()] {
            let fixture = Fixture::new();
            fixture.repo("alpha");
            fixture.repo("beta");
            fs::write(fixture.home.path().join("handoff"), "untouched").expect("sentinel");
            let mut terminal = fixture.terminal("bash", "\nstty -g > before\nREPOT_CD_FILE=handoff repot jump > output 2> errors\nprintf '%s' \"$?\" > status\nstty -g > after\nprintf REPOT_TEST_DONE\n", false);
            terminal.until(b"esc cancel");
            terminal.send(cancel);
            terminal.finish();
            assert_eq!(
                fs::read(fixture.home.path().join("handoff")).expect("handoff"),
                b"untouched"
            );
            assert_eq!(
                fs::read(fixture.home.path().join("status")).expect("status"),
                b"1"
            );
            assert_eq!(
                terminal_settings(&fixture.home.path().join("before")),
                terminal_settings(&fixture.home.path().join("after"))
            );
        }
    }

    #[test]
    fn initial_query_can_be_cleared_and_terminal_resized() {
        let fixture = Fixture::new();
        fixture.repo("alpha");
        let wanted = fixture.repo("beta");
        let mut terminal = fixture.terminal(
            "bash",
            "\nrepot jump no-match > result\nprintf REPOT_TEST_DONE\n",
            false,
        );
        terminal.until(b"no matches");
        terminal
            .master
            .resize(PtySize {
                rows: 5,
                cols: 20,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("shrink PTY");
        terminal.until(b"esc cancel");
        terminal.output.clear();
        terminal
            .master
            .resize(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("grow PTY");
        terminal.until(b"esc cancel");
        terminal.send(b"\x15beta\r");
        terminal.finish();
        assert_eq!(
            fs::read(fixture.home.path().join("result")).expect("selection"),
            [wanted.as_os_str().as_encoded_bytes(), b"\n"].concat()
        );
    }

    #[test]
    fn repeated_resize_and_batched_input_preserve_every_query() {
        let fixture = Fixture::new();
        fixture.repo("alpha");
        let wanted = fixture.repo("beta");
        let mut terminal = fixture.terminal(
            "bash",
            "\nrepot jump no-match > result\nprintf REPOT_TEST_DONE\n",
            false,
        );
        terminal.until(b"no matches");
        for _ in 0..16 {
            terminal.output.clear();
            terminal
                .master
                .resize(PtySize {
                    rows: 5,
                    cols: 20,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("shrink PTY");
            terminal.until(b"esc cancel");
            terminal.output.clear();
            terminal
                .master
                .resize(PtySize {
                    rows: 30,
                    cols: 120,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("grow PTY");
            // Queue the whole edit alongside SIGWINCH, without waiting for the
            // resize event to drain or resending input to wake a stuck reader.
            terminal.send(b"\x15beta");
            // Only a query matching beta renders it as a result.
            terminal.until(b"beta");
            terminal.output.clear();
            terminal.send(b"\x15no-match");
            terminal.until(b"no matches");
        }
        terminal.send(b"\x15beta\r");
        terminal.finish();
        assert_eq!(
            fs::read(fixture.home.path().join("result")).expect("selection"),
            [wanted.as_os_str().as_encoded_bytes(), b"\n"].concat()
        );
    }

    #[test]
    fn external_interrupts_restore_terminal_without_selecting() {
        for signal in [
            nix::sys::signal::Signal::SIGINT,
            nix::sys::signal::Signal::SIGTERM,
        ] {
            let fixture = Fixture::new();
            fixture.repo("alpha");
            fixture.repo("beta");
            let mut terminal = fixture.terminal("bash", "\nstty -g > before\nsh -c 'stty -g > before; echo $$ > pid; exec repot jump > result 2> errors'\nstty -g > after\nprintf REPOT_TEST_DONE\n", false);
            terminal.until(b"esc cancel");
            let pid = fs::read_to_string(fixture.home.path().join("pid"))
                .expect("picker PID")
                .trim()
                .parse::<i32>()
                .expect("PID number");
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), signal)
                .expect("interrupt picker");
            terminal.finish();
            assert!(
                fs::read(fixture.home.path().join("result"))
                    .expect("stdout")
                    .is_empty()
            );
            assert_eq!(
                terminal_settings(&fixture.home.path().join("before")),
                terminal_settings(&fixture.home.path().join("after"))
            );
        }
    }

    #[test]
    fn ctrl_z_restores_terminal_then_resume_keeps_query_and_selection() {
        let fixture = Fixture::new();
        fixture.repo("alpha");
        let wanted = fixture.repo("beta");
        let mut terminal = fixture.terminal("bash", "\nstty -g > before\nsh -c 'stty -g > before; echo $$ > pid; exec repot jump > result 2> errors'\nstty -g > after\nprintf REPOT_TEST_DONE\n", false);
        terminal.until(b"esc cancel");
        terminal.send(b"beta\x1a");
        let pid = fs::read_to_string(fixture.home.path().join("pid")).expect("picker PID");
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(5))
            .expect("deadline");
        loop {
            let output = Command::new("ps")
                .args(["-o", "state=", "-p", pid.trim()])
                .output()
                .expect("inspect stopped child");
            if String::from_utf8_lossy(&output.stdout)
                .trim()
                .starts_with('T')
            {
                break;
            }
            assert!(Instant::now() < deadline, "picker did not suspend");
        }
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid.trim().parse().expect("PID number")),
            nix::sys::signal::Signal::SIGCONT,
        )
        .expect("resume picker");
        // Wait for a fresh frame so Enter cannot race restoration of raw mode.
        terminal.output.clear();
        terminal.until(b"esc cancel");
        terminal.send(b"\r");
        terminal.finish();
        assert_eq!(
            fs::read(fixture.home.path().join("result")).expect("selection"),
            [wanted.as_os_str().as_encoded_bytes(), b"\n"].concat()
        );
        assert_eq!(
            terminal_settings(&fixture.home.path().join("before")),
            terminal_settings(&fixture.home.path().join("after"))
        );
    }

    fn shell_picker(shell: &str) {
        if Command::new(shell).arg("--version").output().is_err() {
            eprintln!("skipping {shell}: executable unavailable");
            return;
        }
        let fixture = Fixture::new();
        fixture.repo("host/owner/alpha");
        let wanted = fixture.repo("host/owner/beta ' ; $(false)\n");
        let body = match shell {
            "nu" => {
                "\nrepot jump; $env.PWD | save --force $env.REPOT_TEST_RESULT; print REPOT_TEST_DONE\n"
            }
            _ => {
                "\nrepot jump; printf '%s' \"$PWD\" > \"$REPOT_TEST_RESULT\"; printf REPOT_TEST_DONE\n"
            }
        };
        let mut terminal = fixture.terminal(shell, body, true);
        terminal.until(b"esc cancel");
        terminal.send(b"beta\r");
        terminal.finish();
        assert_eq!(
            fs::read(fixture.home.path().join("result")).expect("shell directory"),
            wanted.as_os_str().as_encoded_bytes()
        );
        assert_eq!(
            fs::read_dir(fixture.home.path().join("tmp"))
                .expect("handoffs")
                .count(),
            0
        );
    }

    #[test]
    fn bash_picker_handoff_preserves_newline_paths() {
        shell_picker("bash");
    }

    #[test]
    fn zsh_picker_handoff_preserves_newline_paths() {
        shell_picker("zsh");
    }

    #[test]
    fn fish_picker_handoff_preserves_newline_paths() {
        shell_picker("fish");
    }

    #[test]
    fn nushell_picker_handoff_preserves_newline_paths() {
        shell_picker("nu");
    }

    #[test]
    fn terminal_status_table_respects_no_color_and_escapes_path_controls() {
        let fixture = Fixture::new();
        fixture.repo("host/owner/newline\nname");
        let mut terminal = fixture.terminal(
            "bash",
            "\nrepot status --no-fetch\nprintf REPOT_TEST_DONE\n",
            false,
        );
        terminal.until(b"REPOT_TEST_DONE");
        assert!(terminal.child.wait().expect("shell status").success());
        let output = String::from_utf8_lossy(&terminal.output);
        assert!(output.contains("Needs review"));
        assert!(output.contains("no remote"));
        assert!(output.contains("1 repository inspected"));
        assert!(output.contains("newline\\nname"));
        assert!(!output.contains('\u{1b}'));
    }
}
