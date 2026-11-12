//! `--progress-fd`: what `bp create` reports on a third channel (ADR-0010).
//!
//! The fd is handed to the child as fd 3, the way a daemon or a shell's
//! `3>progress.jsonl` would, and read back once `bp` has exited.

use std::fs::{self, File};
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use bitplane_core::testing::{repository_with_one_commit, scratch_dir};
use serde_json::{Value, json};

#[test]
fn every_member_reports_its_fetch_its_worktree_and_each_script() {
    let host = Host::new("progress-rows");
    host.project("api");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"echo installing\"\npost_worktree_create = true\n",
    );
    let adhoc = host.repository("scratch");

    let (run, events) = host.create_reporting(&["@api", &adhoc.display().to_string()]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    let rows: Vec<(String, String, String)> = events.iter().map(row).collect();
    let adhoc = adhoc.display().to_string();
    for wanted in [
        ("@api", "fetch", "started"),
        ("@api", "fetch", "done"),
        ("@api", "worktree", "started"),
        ("@api", "worktree", "done"),
        ("@api", "script", "started"),
        ("@api", "script", "done"),
        (&adhoc, "worktree", "started"),
        (&adhoc, "worktree", "done"),
    ] {
        let wanted = (
            wanted.0.to_owned(),
            wanted.1.to_owned(),
            wanted.2.to_owned(),
        );
        assert!(
            rows.contains(&wanted),
            "{wanted:?} missing from {events:#?}"
        );
    }
    assert!(
        !rows
            .iter()
            .any(|(member, step, _)| member == &adhoc && step == "fetch"),
        "an ad-hoc member has nothing to fetch"
    );

    let script = events
        .iter()
        .find(|event| event["step"] == "script" && event["state"] == "done")
        .unwrap();
    assert_eq!(script["name"], "install");
    assert!(Path::new(script["log"].as_str().unwrap()).is_file());

    let started = |step: &str| position(&events, "@api", step, "started");
    assert!(started("fetch") < started("worktree"));
    assert!(started("worktree") < started("script"));
}

#[test]
fn stdout_and_stderr_are_byte_identical_with_and_without_the_flag() {
    let host = Host::new("progress-unchanged");
    host.project("api");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"echo installing\"\npost_worktree_create = true\n",
    );

    for json in [false, true] {
        let mut arguments = vec!["create", "@api", "-b", "feat-login", "--id", "auth-work"];
        if json {
            arguments.push("--json");
        }

        let (reported, _) = host.bp_reporting(&arguments);
        host.destroy();
        let plain = host.bp(&arguments);
        host.destroy();

        assert_eq!(reported.status.code(), Some(0), "{}", stderr(&reported));
        assert_eq!(
            strip_timings(&stdout(&reported)),
            strip_timings(&stdout(&plain))
        );
        assert_eq!(
            strip_timings(&stderr(&reported)),
            strip_timings(&stderr(&plain))
        );
    }
}

#[test]
fn a_lock_held_by_someone_else_is_reported_as_waiting() {
    let host = Host::new("progress-waiting");
    host.project("api");

    let sentinel = host.projects().join("api/.bitplane/lock");
    fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
    let held = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&sentinel)
        .unwrap();
    held.lock().unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(held);
    });

    let (run, events) = host.create_reporting(&["@api"]);
    release.join().unwrap();

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(stderr(&run).contains("waiting for"), "{}", stderr(&run));
    let waiting = events
        .iter()
        .find(|event| event["event"] == "waiting")
        .unwrap_or_else(|| panic!("no waiting event in {events:#?}"));
    assert!(
        waiting["object"]
            .as_str()
            .unwrap()
            .ends_with("api/.bitplane/lock")
    );
}

#[test]
fn an_fd_that_is_not_open_is_a_usage_failure_before_anything_is_created() {
    let host = Host::new("progress-closed-fd");
    host.project("api");

    let run = host.bp(&[
        "create",
        "@api",
        "-b",
        "feat-login",
        "--id",
        "auth-work",
        "--progress-fd",
        "9",
        "--json",
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "");
    assert_eq!(envelope(&run)["error"], "bad_progress_fd");
    assert!(!host.planes().join("auth-work").exists());
}

#[test]
fn stdout_and_stderr_cannot_be_the_progress_fd() {
    let host = Host::new("progress-reserved-fd");

    for fd in ["0", "1", "2"] {
        let run = host.bp(&["list", "--progress-fd", fd, "--json"]);

        assert_eq!(run.status.code(), Some(2), "fd {fd}");
        assert_eq!(envelope(&run)["error"], "bad_progress_fd");
    }
}

#[test]
fn a_reader_that_went_away_costs_only_the_progress() {
    let host = Host::new("progress-reader-gone");
    host.project("api");

    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let run = host.bp_with_fd3(
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
        writer.into(),
    );

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(stdout(&run).contains("auth-work"));
    assert!(host.planes().join("auth-work/forges/api").is_dir());
}

#[test]
fn a_request_on_stdin_reports_progress_too() {
    let host = Host::new("progress-stdin");
    host.project("api");

    let request = json!({
        "action": "plane_create",
        "members": ["@api"],
        "branch": "feat-login",
        "id": "auth-work",
        "here": null,
        "intent": "resolve",
        "fetch": true,
        "run_scripts": true,
    });
    let target = host.root.join("progress.jsonl");
    let mut command = host.command(&["--stdin", "--json", "--progress-fd", "3"]);
    let fd: OwnedFd = File::create(&target).unwrap().into();
    with_fd3(&mut command, &fd);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(fd);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.to_string().as_bytes())
        .unwrap();
    let run = child.wait_with_output().unwrap();

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    let events = lines(&target);
    assert!(position(&events, "@api", "worktree", "done") < usize::MAX);
}

// ---------------------------------------------------------------- fixtures --

fn row(event: &Value) -> (String, String, String) {
    let text = |key: &str| event[key].as_str().unwrap_or_default().to_owned();
    (text("member"), text("step"), text("state"))
}

fn position(events: &[Value], member: &str, step: &str, state: &str) -> usize {
    events
        .iter()
        .position(|event| row(event) == (member.to_owned(), step.to_owned(), state.to_owned()))
        .unwrap_or_else(|| panic!("no {member} {step} {state} in {events:#?}"))
}

fn lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one JSON object"))
        .collect()
}

/// Durations and log timestamps differ between two runs by nature, so every
/// run of digits is one token; the words around them have to agree.
fn strip_timings(text: &str) -> String {
    let mut stripped = String::new();
    let mut in_digits = false;
    for character in text.chars() {
        match (character.is_ascii_digit(), in_digits) {
            (true, false) => stripped.push('#'),
            (true, true) => {}
            (false, _) => stripped.push(character),
        }
        in_digits = character.is_ascii_digit();
    }
    stripped
}

fn envelope(run: &Output) -> Value {
    serde_json::from_str(stderr(run).trim()).expect("one line of envelope on stderr")
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}

/// Makes `fd` the child's fd 3, as `3>…` would.
fn with_fd3(command: &mut Command, fd: &OwnedFd) {
    let raw = fd.as_raw_fd();
    // SAFETY: dup2 is async-signal-safe, and nothing else runs between fork
    // and exec.
    unsafe {
        command.pre_exec(move || {
            // dup2 onto itself keeps close-on-exec, so clear it by hand.
            let done = match raw {
                3 => libc::fcntl(3, libc::F_SETFD, 0),
                _ => libc::dup2(raw, 3),
            };
            match done {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            }
        });
    }
}

/// Cuts the run off from the developer's own environment.
fn sealed_off(command: &mut Command, root: &Path) {
    command
        .env("BITPLANE_CONFIG", root.join("no-such-config.toml"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR");
}

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
    /// forge, so `create` fetches it.
    fn project(&self, name: &str) {
        let forge = self.root.join("forges").join(name);
        repository_with_one_commit(&forge);
        let run = self.bp(&["project", "add", &forge.display().to_string()]);
        assert!(
            run.status.success(),
            "the project was not registered: {}",
            stderr(&run)
        );
    }

    fn declares(&self, project: &str, scripts: &str) {
        let file = self.projects().join(project).join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");
        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    fn projects(&self) -> PathBuf {
        self.root.join("projects")
    }

    fn destroy(&self) {
        let run = self.bp(&["destroy", "-p", "auth-work"]);
        assert!(run.status.success(), "{}", stderr(&run));
    }

    /// `bp create <members> -b feat-login --id auth-work`, reporting progress.
    fn create_reporting(&self, members: &[&str]) -> (Output, Vec<Value>) {
        let mut arguments = vec!["create"];
        arguments.extend(members);
        arguments.extend(["-b", "feat-login", "--id", "auth-work"]);
        self.bp_reporting(&arguments)
    }

    /// The command with `--progress-fd 3`, fd 3 being a file read back after.
    fn bp_reporting(&self, arguments: &[&str]) -> (Output, Vec<Value>) {
        let target = self.root.join("progress.jsonl");
        let fd: OwnedFd = File::create(&target).unwrap().into();
        let run = self.bp_with_fd3(&[arguments, &["--progress-fd", "3"]].concat(), fd);
        (run, lines(&target))
    }

    fn bp_with_fd3(&self, arguments: &[&str], fd: OwnedFd) -> Output {
        let mut command = self.command(arguments);
        with_fd3(&mut command, &fd);
        let run = command.output().expect("run bp");
        drop(fd);
        run
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.projects());
        sealed_off(&mut command, &self.root);
        command
    }
}
