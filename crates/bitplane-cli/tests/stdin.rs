//! `bp --stdin`: a request as JSON in place of a command line (ADR-0010).
//!
//! Each case runs the request and its argv equivalent against the same host
//! and asserts the two said the same thing, exit code included.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use bitplane_core::testing::{repository_with_an_origin, scratch_dir};

#[test]
fn a_list_on_stdin_answers_exactly_what_the_command_line_does() {
    let host = Host::new("stdin-list");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let piped = host.bp_stdin(&["--json"], r#"{"action":"plane_list","health":"cheap"}"#);
    let typed = host.bp(&["list", "--json"]);

    assert_eq!(piped.status.code(), Some(0), "stderr: {}", stderr(&piped));
    assert_eq!(stdout(&piped), stdout(&typed));
    assert_eq!(stderr(&piped), "");
}

#[test]
fn the_human_rendering_is_the_default_there_too() {
    let host = Host::new("stdin-list-human");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let piped = host.bp_stdin(&[], r#"{"action":"plane_list","health":"cheap"}"#);

    assert_eq!(stdout(&piped), stdout(&host.bp(&["list"])));
}

#[test]
fn a_finding_still_exits_three() {
    let host = Host::new("stdin-list-drift");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);
    fs::remove_dir_all(host.planes().join("auth-work/repos/alpha")).unwrap();

    let piped = host.bp_stdin(&["--json"], r#"{"action":"plane_list","health":"cheap"}"#);

    assert_eq!(piped.status.code(), Some(3), "stderr: {}", stderr(&piped));
    assert_eq!(stdout(&piped), stdout(&host.bp(&["list", "--json"])));
}

#[test]
fn a_create_on_stdin_builds_the_plane_the_command_line_would() {
    let host = Host::new("stdin-create");
    let alpha = host.repository("alpha");

    let request = serde_json::json!({
        "action": "plane_create",
        "members": [alpha],
        "branch": "feat-login",
        "id": "auth-work",
        "here": null,
        "intent": "resolve",
        "fetch": true,
        "run_scripts": true,
    });
    let piped = host.bp_stdin(&["--json"], &request.to_string());

    assert_eq!(piped.status.code(), Some(0), "stderr: {}", stderr(&piped));
    let response: serde_json::Value = serde_json::from_str(&stdout(&piped)).unwrap();
    assert_eq!(response["action"], "plane_create");
    assert_eq!(response["id"], "auth-work");
    assert_eq!(response["members"][0]["detail"]["branch"], "feat-login");
    assert!(host.planes().join("auth-work/repos/alpha").is_dir());
}

#[test]
fn a_refused_destroy_writes_the_same_envelope_and_exits_one() {
    let host = Host::new("stdin-destroy-refused");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);
    fs::write(host.planes().join("auth-work/repos/alpha/README.md"), "x\n").unwrap();

    let request = r#"{"action":"plane_destroy","plane":{"plane":"id","id":"auth-work"},"waive":[],"run_scripts":true}"#;
    let piped = host.bp_stdin(&["--json"], request);
    let typed = host.bp(&["destroy", "-p", "auth-work", "--json"]);

    assert_eq!(piped.status.code(), Some(1));
    assert_eq!(typed.status.code(), Some(1));
    assert_eq!(stdout(&piped), "", "stdout carries results only");
    assert_eq!(stderr(&piped), stderr(&typed));
    assert_eq!(envelope(&piped)["error"], "refused");
    assert!(
        host.planes().join("auth-work").exists(),
        "nothing was touched"
    );
}

#[test]
fn input_that_is_not_a_request_is_a_usage_failure() {
    let host = Host::new("stdin-malformed");

    for input in [
        "",
        "not json",
        r#"{"action":"plane_explode"}"#,
        r#"{"health":"cheap"}"#,
    ] {
        let piped = host.bp_stdin(&["--json"], input);

        assert_eq!(piped.status.code(), Some(2), "input {input:?}");
        assert_eq!(stdout(&piped), "");
        assert_eq!(envelope(&piped)["error"], "malformed_request");
        assert_eq!(envelope(&piped)["code"], 2);
    }
}

#[test]
fn stdin_and_a_command_cannot_both_be_given() {
    let host = Host::new("stdin-and-command");

    let piped = host.bp_stdin(&["list", "--json"], r#"{"action":"project_list"}"#);

    assert_eq!(piped.status.code(), Some(2));
    assert_eq!(envelope(&piped)["error"], "bad_usage");
}

#[test]
fn the_contract_version_is_one_line_of_json_on_stdout() {
    let run = Command::new(env!("CARGO_BIN_EXE_bp"))
        .args(["--version", "--json"])
        .output()
        .expect("run bp");

    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stderr(&run), "");
    let version: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(version["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(version["contract"], bitplane_core::CONTRACT_VERSION);
}

// ---------------------------------------------------------------- fixtures --

fn envelope(run: &Output) -> serde_json::Value {
    serde_json::from_str(stderr(run).trim()).expect("one line of envelope on stderr")
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
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
        repository_with_an_origin(&path, &self.root.join("forge").join(name));
        path.canonicalize().expect("the repository was just made")
    }

    fn plane(&self, members: &[&Path]) {
        let mut arguments: Vec<String> = vec!["create".to_owned()];
        arguments.extend(members.iter().map(|member| member.display().to_string()));
        arguments.extend(["-b", "feat-login", "--id", "auth-work"].map(str::to_owned));

        let run = self.bp(&arguments.iter().map(String::as_str).collect::<Vec<&str>>());
        assert!(
            run.status.success(),
            "the fixture plane was not built: {}",
            stderr(&run)
        );
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    /// `bp --stdin`, with `input` written to it and closed.
    fn bp_stdin(&self, arguments: &[&str], input: &str) -> Output {
        let mut child = self
            .command(&[&["--stdin"], arguments].concat())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run bp");
        child
            .stdin
            .take()
            .expect("stdin is piped")
            .write_all(input.as_bytes())
            .expect("write the request");
        child.wait_with_output().expect("bp finishes")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.root.join("projects"));
        sealed_off(&mut command, &self.root);
        command
    }
}
