//! `bp request`, end to end through the binary.
//!
//! A request on stdin is the command line with the parser taken out, so every
//! test here runs the same intent both ways and asserts the two agree — on
//! stdout, on stderr and on the exit code.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};

#[test]
fn a_request_on_stdin_answers_exactly_as_its_command_line_does() {
    let host = Host::new("cli-request-same");
    let alpha = host.repository("alpha");
    host.plane("auth-work", &alpha);

    for (argv, request) in [
        (
            vec!["list", "--json"],
            r#"{"action":"plane_list","health":"cheap"}"#,
        ),
        (
            vec!["show", "-p", "auth-work", "--json"],
            r#"{"action":"plane_show","plane":{"plane":"id","id":"auth-work"},"health":"cheap"}"#,
        ),
        (
            vec!["status", "-p", "auth-work", "--json"],
            r#"{"action":"plane_status","plane":{"plane":"id","id":"auth-work"}}"#,
        ),
        (
            vec!["project", "list", "--json"],
            r#"{"action":"project_list"}"#,
        ),
    ] {
        let by_argv = host.bp(&argv);
        let by_stdin = host.request(request, &["--json"]);

        assert_eq!(
            by_argv.status.code(),
            Some(0),
            "{argv:?}: {}",
            stderr(&by_argv)
        );
        assert_same(&by_argv, &by_stdin, request);
    }
}

#[test]
fn a_create_on_stdin_makes_the_plane_its_command_line_would() {
    let host = Host::new("cli-request-create");
    let alpha = host.repository("alpha");

    let run = host.request(
        &format!(
            r#"{{"action":"plane_create","members":["{}"],"branch":"feat-login","id":"auth-work","here":null,"intent":"require_new","fetch":true,"run_scripts":true}}"#,
            alpha.display()
        ),
        &[],
    );

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {alpha}  feat-login  created (new branch)  repos/alpha\n",
            ),
            planes = host.planes().display(),
            alpha = alpha.display(),
        )
    );
    assert_eq!(stderr(&run), "");
}

#[test]
fn a_finding_exits_3_with_the_result_on_stdout() {
    let host = Host::new("cli-request-drift");
    let alpha = host.repository("alpha");
    let plane = host.plane("auth-work", &alpha);
    fs::remove_dir_all(plane.join("repos/alpha")).expect("delete the worktree behind bp's back");

    let request = r#"{"action":"plane_status","plane":{"plane":"id","id":"auth-work"}}"#;
    let by_argv = host.bp(&["status", "-p", "auth-work"]);
    let by_stdin = host.request(request, &[]);

    assert_eq!(
        by_stdin.status.code(),
        Some(3),
        "stderr: {}",
        stderr(&by_stdin)
    );
    assert!(
        !stdout(&by_stdin).is_empty(),
        "the result is still on stdout"
    );
    assert_same(&by_argv, &by_stdin, request);
}

#[test]
fn a_refusal_writes_the_envelope_on_stderr_with_the_usual_code() {
    let host = Host::new("cli-request-refused");
    let alpha = host.repository("alpha");
    let plane = host.plane("auth-work", &alpha);
    fs::write(plane.join("repos/alpha/README.md"), "changed\n").expect("leave work behind");

    let request = r#"{"action":"plane_destroy","plane":{"plane":"id","id":"auth-work"},"waive":[],"run_scripts":true}"#;
    let by_argv = host.bp(&["destroy", "-p", "auth-work", "--json"]);
    let by_stdin = host.request(request, &["--json"]);

    assert_eq!(by_stdin.status.code(), Some(1));
    assert_eq!(stdout(&by_stdin), "", "stdout carries results only");
    assert_eq!(envelope(&by_stdin)["error"], "refused");
    assert_same(&by_argv, &by_stdin, request);
}

#[test]
fn a_usage_failure_inside_a_well_formed_request_exits_2() {
    let host = Host::new("cli-request-usage");

    let request =
        r#"{"action":"plane_show","plane":{"plane":"id","id":"nowhere"},"health":"cheap"}"#;
    let by_argv = host.bp(&["show", "-p", "nowhere", "--json"]);
    let by_stdin = host.request(request, &["--json"]);

    assert_eq!(by_stdin.status.code(), Some(2));
    assert_same(&by_argv, &by_stdin, request);
}

#[test]
fn input_that_is_not_one_request_is_a_usage_failure() {
    let host = Host::new("cli-request-malformed");

    for input in [
        "",
        "not json",
        r#"{"action":"plane_explode"}"#,
        r#"{"action":"plane_show"}"#,
        r#"{"action":"plane_list","health":"cheap"} {"action":"project_list"}"#,
    ] {
        let run = host.request(input, &["--json"]);

        assert_eq!(run.status.code(), Some(2), "{input:?}: {}", stderr(&run));
        assert_eq!(stdout(&run), "", "stdout carries results only");
        let envelope = envelope(&run);
        assert_eq!(envelope["error"], "malformed_request", "{input:?}");
        assert_eq!(envelope["code"], 2);
        assert!(
            envelope["remedy"].is_string(),
            "a usage failure says what to do"
        );
    }
}

#[test]
fn a_malformed_request_is_rendered_for_a_human_by_default() {
    let host = Host::new("cli-request-human");

    let run = host.request(r#"{"action":"plane_show"}"#, &[]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(
        stderr(&run),
        concat!(
            "error[malformed_request]: stdin is not a request: missing field `plane`\n",
            "\n",
            "remedy: Send exactly one request, as JSON, on stdin.\n",
        )
    );
}

#[test]
fn a_malformed_request_is_refused_before_git_is_checked() {
    let host = Host::new("cli-request-no-git");

    let run = host
        .command(&["request", "--json"])
        .env("PATH", host.root.join("no-git-here"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map(|mut child| {
            child
                .stdin
                .take()
                .expect("piped")
                .write_all(b"nonsense")
                .expect("write stdin");
            child.wait_with_output().expect("wait for bp")
        })
        .expect("spawn bp");

    assert_eq!(envelope(&run)["error"], "malformed_request");
}

#[test]
fn the_version_is_one_line_of_json_naming_the_contract() {
    let run = Host::new("cli-request-version").bp(&["--version", "--json"]);

    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stderr(&run), "");
    assert_eq!(stdout(&run).lines().count(), 1, "got: {}", stdout(&run));

    let version: serde_json::Value = serde_json::from_str(&stdout(&run)).expect("JSON on stdout");
    assert_eq!(version["name"], "bp");
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(version["contract"], bitplane_core::CONTRACT_VERSION);
}

#[test]
fn the_version_without_json_is_unchanged() {
    let run = Host::new("cli-request-version-human").bp(&["--version"]);

    assert_eq!(stdout(&run), format!("bp {}\n", env!("CARGO_PKG_VERSION")));
}

/// Both runs, byte for byte, on every stream.
fn assert_same(by_argv: &Output, by_stdin: &Output, request: &str) {
    assert_eq!(
        by_stdin.status.code(),
        by_argv.status.code(),
        "exit code of {request}"
    );
    assert_eq!(stdout(by_stdin), stdout(by_argv), "stdout of {request}");
    assert_eq!(stderr(by_stdin), stderr(by_argv), "stderr of {request}");
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

    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_one_commit(&path);
        // A branch for a plane to land on that the checkout does not occupy.
        git(&path, &["branch", "spare"]);
        path.canonicalize().expect("the repository was just made")
    }

    /// A plane holding `repository` on `spare`, made through the command line.
    fn plane(&self, id: &str, repository: &Path) -> PathBuf {
        let run = self.bp(&[
            "create",
            &repository.display().to_string(),
            "-b",
            "spare",
            "--id",
            id,
        ]);
        assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
        self.planes().join(id)
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    /// `bp request` with `input` on stdin.
    fn request(&self, input: &str, arguments: &[&str]) -> Output {
        let mut child = self
            .command(&[&["request"], arguments].concat())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn bp");

        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(input.as_bytes())
            .expect("write the request");

        child.wait_with_output().expect("wait for bp")
    }

    /// `bp`, cut off from the developer's own config: resolution reads it
    /// whatever the flags say.
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
