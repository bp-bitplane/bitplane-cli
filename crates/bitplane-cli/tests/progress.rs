//! `bp create --progress-fd`, end to end through the binary (ADR-0010).
//!
//! What the core tests cannot check: that the events reach a real descriptor
//! as JSON lines, that stdout and stderr are byte for byte what they are
//! without the flag, and that a bad descriptor is refused before anything is
//! made.

use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use bitplane_core::testing::{repository_with_one_commit, scratch_dir};
use bitplane_core::{MemberProgress, MemberRef, ProgressEvent, Step, StepState};

#[test]
fn every_member_reports_its_steps_as_json_lines_on_the_descriptor() {
    let host = Host::new("cli-progress-lines");
    host.project("api");
    host.project("web");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"echo installing\"\npost_worktree_create = true\n",
    );

    let (run, events) = host.with_progress(&[
        "create",
        "@api",
        "@web",
        "-b",
        "feat-login",
        "--id",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        steps_of(&events, "@api"),
        vec![
            ("fetch", StepState::Started),
            ("fetch", StepState::Done),
            ("worktree", StepState::Started),
            ("worktree", StepState::Done),
            ("script", StepState::Started),
            ("script", StepState::Done),
        ]
    );
    assert_eq!(
        steps_of(&events, "@web"),
        vec![
            ("fetch", StepState::Started),
            ("fetch", StepState::Done),
            ("worktree", StepState::Started),
            ("worktree", StepState::Done),
        ]
    );
}

#[test]
fn stdout_and_stderr_are_byte_identical_with_and_without_the_flag() {
    let host = Host::new("cli-progress-identical");
    host.project("api");
    host.declares("api", "[scripts.install]\nshell = \"echo installing; echo warned >&2\"\npost_worktree_create = true\n");

    for json in [false, true] {
        let mut arguments = vec!["create", "@api", "-b", "feat-login", "--id", "auth-work"];
        if json {
            arguments.push("--json");
        }

        let (with, events) = host.with_progress(&arguments);
        host.tear_down("auth-work");
        let without = host.bp(&arguments);
        host.tear_down("auth-work");

        assert!(!events.is_empty(), "the flag reported something");
        assert_eq!(with.status.code(), Some(0), "stderr: {}", stderr(&with));
        assert_eq!(with.status.code(), without.status.code());
        assert_eq!(
            volatile(&stdout(&with)),
            volatile(&stdout(&without)),
            "stdout, json: {json}"
        );
        assert_eq!(
            volatile(&stderr(&with)),
            volatile(&stderr(&without)),
            "stderr, json: {json}"
        );
    }
}

#[test]
fn a_reader_that_went_away_does_not_stop_the_run() {
    let host = Host::new("cli-progress-closed-reader");
    host.project("api");

    let (reader, writer) = pipe();
    drop(reader);
    let run = host.bp_with_fd_3(
        &[
            "create",
            "@api",
            "-b",
            "feat-login",
            "--id",
            "auth-work",
            "--progress-fd",
            "3",
        ],
        writer,
    );

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(host.planes().join("auth-work/plane.toml").is_file());
}

#[test]
fn a_descriptor_that_is_not_open_is_a_usage_failure_before_anything_is_made() {
    let host = Host::new("cli-progress-not-open");
    host.project("api");

    let run = host.bp(&[
        "create",
        "@api",
        "-b",
        "feat-login",
        "--id",
        "auth-work",
        "--progress-fd",
        "97",
        "--json",
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    let envelope = envelope(&run);
    assert_eq!(envelope["error"], "bad_progress_fd");
    assert_eq!(
        envelope["message"],
        "file descriptor 97 is not open for writing"
    );
    assert!(
        !host.planes().join("auth-work").exists(),
        "nothing was created"
    );
}

#[test]
fn a_descriptor_open_only_for_reading_is_refused() {
    let host = Host::new("cli-progress-read-only");
    host.project("api");

    let (reader, _writer) = pipe();
    let run = host.bp_with_fd_3(
        &[
            "create",
            "@api",
            "-b",
            "feat-login",
            "--id",
            "auth-work",
            "--progress-fd",
            "3",
            "--json",
        ],
        reader,
    );

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(envelope(&run)["error"], "bad_progress_fd");
    assert!(
        !host.planes().join("auth-work").exists(),
        "nothing was created"
    );
}

#[test]
fn bp_s_own_streams_are_refused() {
    let host = Host::new("cli-progress-own-streams");

    for fd in ["0", "1", "2"] {
        let run = host.bp(&[
            "create",
            "/nowhere",
            "-b",
            "feat-login",
            "--progress-fd",
            fd,
            "--json",
        ]);

        assert_eq!(run.status.code(), Some(2), "fd {fd}");
        assert_eq!(envelope(&run)["error"], "bad_progress_fd", "fd {fd}");
    }
}

#[test]
fn a_create_request_on_stdin_reports_progress_too() {
    let host = Host::new("cli-progress-request");
    host.project("api");

    let (reader, writer) = pipe();
    let mut child = host
        .command(&["request", "--json", "--progress-fd", "3"])
        .with_fd_3(writer)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bp");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"action":"plane_create","members":["@api"],"branch":"feat-login","id":"auth-work","here":null,"intent":"resolve","fetch":true,"run_scripts":true}"#)
        .unwrap();
    let run = child.wait_with_output().expect("wait for bp");

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        steps_of(&read_events(reader), "@api"),
        vec![
            ("fetch", StepState::Started),
            ("fetch", StepState::Done),
            ("worktree", StepState::Started),
            ("worktree", StepState::Done),
        ]
    );
}

#[test]
fn a_request_that_is_not_a_create_reports_nothing() {
    let host = Host::new("cli-progress-request-read");

    let (reader, writer) = pipe();
    let mut child = host
        .command(&["request", "--progress-fd", "3"])
        .with_fd_3(writer)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bp");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"action":"plane_list","health":"cheap"}"#)
        .unwrap();
    let run = child.wait_with_output().expect("wait for bp");

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(read_events(reader), vec![], "EOF, and nothing before it");
}

/// One member's steps, in the order that member reported them.
fn steps_of(events: &[ProgressEvent], member: &str) -> Vec<(&'static str, StepState)> {
    let wanted = MemberRef::parse(member).unwrap();

    events
        .iter()
        .filter_map(|event| match event {
            ProgressEvent::Member(MemberProgress {
                member,
                step,
                state,
            }) if *member == wanted => Some((
                match step {
                    Step::Fetch => "fetch",
                    Step::Worktree => "worktree",
                    Step::Script { .. } => "script",
                },
                *state,
            )),
            _ => None,
        })
        .collect()
}

/// Everything written to the descriptor, every line of it an event.
fn read_events(reader: OwnedFd) -> Vec<ProgressEvent> {
    let mut text = String::new();
    fs::File::from(reader)
        .read_to_string(&mut text)
        .expect("read the progress");

    text.lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|err| panic!("not an event ({err}): {line}"))
        })
        .collect()
}

/// A script's duration and a log's timestamp are real time, so two runs differ
/// in exactly those and nothing else.
fn volatile(text: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for character in text.chars() {
        if character.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(character);
        }
    }
    out
}

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    // SAFETY: `pipe` writes two new descriptors into the array it is given.
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "make a pipe");
    // Close-on-exec, or every `bp` another test spawns meanwhile inherits the
    // write end and the reader waits for that one to exit too. `dup2` clears
    // the flag on the copy at 3, so the child this is meant for keeps it.
    for fd in fds {
        // SAFETY: setting a descriptor flag on a descriptor this test owns.
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    // SAFETY: both were just opened and nothing else owns them.
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

/// The JSON error envelope `bp` writes to stderr on failure under `--json`.
fn envelope(run: &Output) -> serde_json::Value {
    serde_json::from_str(&stderr(run))
        .unwrap_or_else(|err| panic!("stderr is not a JSON envelope ({err}): {}", stderr(run)))
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}

trait WithFd3 {
    /// Puts `fd` at descriptor 3 in the child, the way a shell's `3>` does.
    fn with_fd_3(&mut self, fd: OwnedFd) -> &mut Self;
}

impl WithFd3 for Command {
    fn with_fd_3(&mut self, fd: OwnedFd) -> &mut Self {
        // SAFETY: `dup2` is async-signal-safe, and the closure only calls it.
        // The descriptor moves into the closure, so it outlives the spawn.
        unsafe {
            self.pre_exec(move || match libc::dup2(fd.as_raw_fd(), 3) {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            })
        }
    }
}

/// A host of its own, so no test can see another's planes.
struct Host {
    root: PathBuf,
}

impl Host {
    fn new(label: &str) -> Host {
        Host {
            root: scratch_dir(label),
        }
    }

    /// An owned project, registered from a local repository standing in for a
    /// forge — a path is a git URL, so this is the code path a real one takes.
    fn project(&self, name: &str) {
        let forge = self.root.join("forges").join(name);
        repository_with_one_commit(&forge);

        let run = self.bp(&["project", "add", &forge.display().to_string()]);
        assert!(
            run.status.success(),
            "the fixture project was not registered: {}",
            stderr(&run)
        );
    }

    /// Appends a `[scripts]` table to the project's file, the way a user does.
    fn declares(&self, project: &str, scripts: &str) {
        let file = self
            .root
            .join("projects")
            .join(project)
            .join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");

        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    /// Takes the plane apart again, so the next run can make it afresh.
    fn tear_down(&self, id: &str) {
        let run = self.bp(&["destroy", "-p", id, "--no-scripts"]);
        assert!(
            run.status.success(),
            "the plane was not destroyed: {}",
            stderr(&run)
        );
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    /// The run, and every event it wrote to descriptor 3.
    fn with_progress(&self, arguments: &[&str]) -> (Output, Vec<ProgressEvent>) {
        let (reader, writer) = pipe();
        let run = self.bp_with_fd_3(&[arguments, &["--progress-fd", "3"]].concat(), writer);

        (run, read_events(reader))
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    /// `bp` with `fd` at descriptor 3. The parent's copy is closed once the
    /// child has its own, so the reader sees EOF when `bp` exits.
    fn bp_with_fd_3(&self, arguments: &[&str], fd: OwnedFd) -> Output {
        let mut command = self.command(arguments);
        command.with_fd_3(fd);
        let output = command.output().expect("run bp");
        drop(command);
        output
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.root.join("projects"))
            .env("BITPLANE_CONFIG", self.root.join("no-such-config.toml"))
            .env("XDG_CONFIG_HOME", self.root.join("xdg-config"))
            .env("XDG_DATA_HOME", self.root.join("xdg-data"))
            .env_remove("BITPLANE_PLANES_DIR")
            .env_remove("BITPLANE_PROJECTS_DIR");
        command
    }
}
