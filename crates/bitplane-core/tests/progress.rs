//! What `create` reports while it runs (ADR-0010), against real git
//! repositories and real scripts.
//!
//! Members run in parallel, so events from different members interleave in an
//! order no test can fix. The assertions are per member: each one's steps, in
//! the order that member went through them.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bitplane_core::lock::{self, Lock};
use bitplane_core::testing::{repository_with_one_commit, scratch_dir};
use bitplane_core::{
    BranchIntent, Directories, Engine, EngineError, LocalEngine, MemberProgress, MemberRef,
    PlaneAddRequest, PlaneCreateRequest, PlaneRef, ProgressEvent, ProjectAddRequest, Step,
    StepState,
};

#[test]
fn every_member_reports_its_fetch_its_worktree_and_each_script() {
    let host = Host::new("progress-steps");
    host.project("api");
    host.project("web");
    host.declares(
        "api",
        concat!(
            "[scripts.install]\nshell = \"true\"\npost_worktree_create = true\n\n",
            "[scripts.build]\nshell = \"true\"\npost_worktree_create = true\n",
        ),
    );

    host.create(&["@api", "@web"]).expect("the plane was built");

    let api = host.steps_of("@api");
    assert_eq!(
        api.iter()
            .map(|(step, state)| (label(step), *state))
            .collect::<Vec<_>>(),
        vec![
            ("fetch".to_owned(), StepState::Started),
            ("fetch".to_owned(), StepState::Done),
            ("worktree".to_owned(), StepState::Started),
            ("worktree".to_owned(), StepState::Done),
            ("script install".to_owned(), StepState::Started),
            ("script install".to_owned(), StepState::Done),
            ("script build".to_owned(), StepState::Started),
            ("script build".to_owned(), StepState::Done),
        ]
    );
    assert_eq!(
        host.steps_of("@web")
            .iter()
            .map(|(step, state)| (label(step), *state))
            .collect::<Vec<_>>(),
        vec![
            ("fetch".to_owned(), StepState::Started),
            ("fetch".to_owned(), StepState::Done),
            ("worktree".to_owned(), StepState::Started),
            ("worktree".to_owned(), StepState::Done),
        ],
        "a member with no scripts reports no script step"
    );
}

#[test]
fn a_script_event_names_the_log_its_output_is_going_to() {
    let host = Host::new("progress-script-log");
    host.project("api");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"echo installing\"\npost_worktree_create = true\n",
    );

    let created = host.create(&["@api"]).expect("the plane was built");

    let log = &created.scripts[0].log;
    assert!(
        host.steps_of("@api").iter().any(|(step, _)| matches!(
            step,
            Step::Script { name, log: reported } if name == "install" && reported == log
        )),
        "no script event names {log:?}"
    );
    assert_eq!(fs::read_to_string(log).unwrap(), "installing\n");
}

#[test]
fn a_failing_script_reports_failed_and_the_plane_is_still_kept() {
    let host = Host::new("progress-script-failed");
    host.project("api");
    host.declares(
        "api",
        "[scripts.install]\nshell = \"exit 1\"\npost_worktree_create = true\n",
    );

    let error = host.create(&["@api"]).unwrap_err();

    assert!(
        matches!(error, EngineError::ScriptFailed { .. }),
        "{error:?}"
    );
    assert_eq!(
        host.steps_of("@api").last().map(|(_, state)| *state),
        Some(StepState::Failed)
    );
}

#[test]
fn a_worktree_that_cannot_be_added_reports_failed() {
    let host = Host::new("progress-worktree-failed");
    let alpha = host.repository("repos/alpha");
    // git cannot register a worktree in a repository whose admin directory
    // it may not write.
    let worktrees = alpha.join(".git/worktrees");
    fs::create_dir_all(&worktrees).unwrap();
    fs::set_permissions(&worktrees, fs::Permissions::from_mode(0o555)).unwrap();
    let alpha = alpha.display().to_string();

    let error = host.create(&[&alpha]).unwrap_err();

    assert!(
        matches!(error, EngineError::CreateAborted { .. }),
        "{error:?}"
    );
    assert_eq!(
        host.steps_of(&alpha)
            .iter()
            .map(|(step, state)| (label(step), *state))
            .collect::<Vec<_>>(),
        vec![
            ("worktree".to_owned(), StepState::Started),
            ("worktree".to_owned(), StepState::Failed),
        ],
        "there is no event for the unwind: the result carries it"
    );
}

#[test]
fn an_ad_hoc_member_has_nothing_to_fetch_and_reports_no_fetch() {
    let host = Host::new("progress-ad-hoc");
    let alpha = host.repository("repos/alpha");
    let alpha = alpha.display().to_string();

    host.create(&[&alpha]).expect("the plane was built");

    assert_eq!(
        host.steps_of(&alpha)
            .iter()
            .map(|(step, state)| (label(step), *state))
            .collect::<Vec<_>>(),
        vec![
            ("worktree".to_owned(), StepState::Started),
            ("worktree".to_owned(), StepState::Done),
        ]
    );
}

#[test]
fn a_contended_lock_reports_waiting_naming_the_object() {
    let host = Arc::new(Host::new("progress-waiting"));
    host.project("api");

    let sentinel = host
        .directories
        .projects()
        .join("api/.bitplane")
        .join(lock::SENTINEL_NAME);
    let held = Lock::acquire(&sentinel, lock::DEFAULT_TIMEOUT, |_| {}).unwrap();

    let running = {
        let host = Arc::clone(&host);
        std::thread::spawn(move || host.create(&["@api"]).map(|_| ()))
    };

    let deadline = Instant::now() + Duration::from_secs(10);
    while !host
        .events()
        .iter()
        .any(|event| matches!(event, ProgressEvent::Waiting { .. }))
    {
        assert!(Instant::now() < deadline, "create never reported the wait");
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(held);
    running
        .join()
        .unwrap()
        .expect("the plane was built once the lock was free");

    assert!(
        host.events()
            .contains(&ProgressEvent::Waiting { object: sentinel })
    );
}

#[test]
fn add_reports_nothing() {
    let host = Host::new("progress-add-silent");
    host.project("api");
    host.project("web");
    host.create(&["@api"]).expect("the plane was built");
    host.events.lock().unwrap().clear();

    host.engine()
        .plane_add(PlaneAddRequest {
            plane: PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
            members: vec!["@web".to_owned()],
            branch: Some("feat-login".to_owned()),
            intent: BranchIntent::Resolve,
            fetch: true,
            run_scripts: true,
        })
        .expect("the member was added");

    assert_eq!(host.events(), vec![], "create is the only emitter");
}

/// A step as the assertions spell it.
fn label(step: &Step) -> String {
    match step {
        Step::Fetch => "fetch".to_owned(),
        Step::Worktree => "worktree".to_owned(),
        Step::Script { name, .. } => format!("script {name}"),
    }
}

/// A host of its own, recording every event its engines report.
struct Host {
    root: PathBuf,
    directories: Directories,
    events: Arc<Mutex<Vec<ProgressEvent>>>,
}

impl Host {
    fn new(label: &str) -> Host {
        let root = scratch_dir(label);

        Host {
            directories: Directories::new(root.join("planes"), root.join("projects")),
            root,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn engine(&self) -> LocalEngine {
        let events = Arc::clone(&self.events);

        LocalEngine::new(self.directories.clone()).reporting_progress(move |event| {
            events
                .lock()
                .expect("no progress sink panics")
                .push(event.clone())
        })
    }

    /// An owned project, registered the way `bp project add` registers one.
    fn project(&self, name: &str) {
        let forge = self.repository(&format!("forges/{name}"));

        LocalEngine::new(self.directories.clone())
            .project_add(ProjectAddRequest {
                url: forge.display().to_string(),
                name: Some(name.to_owned()),
            })
            .expect("the fixture project was registered");
    }

    /// Appends a `[scripts]` table to a registered project's file.
    fn declares(&self, project: &str, scripts: &str) {
        let file = self
            .directories
            .projects()
            .join(project)
            .join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");

        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    fn repository(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        repository_with_one_commit(&path);
        path.canonicalize().expect("the repository was just made")
    }

    fn create(&self, members: &[&str]) -> Result<bitplane_core::PlaneCreated, EngineError> {
        self.engine().plane_create(PlaneCreateRequest {
            members: members.iter().map(|member| (*member).to_owned()).collect(),
            branch: Some("feat-login".to_owned()),
            id: Some("auth-work".to_owned()),
            here: None,
            intent: BranchIntent::Resolve,
            fetch: true,
            run_scripts: true,
        })
    }

    fn events(&self) -> Vec<ProgressEvent> {
        self.events.lock().unwrap().clone()
    }

    /// One member's steps, in the order that member reported them.
    fn steps_of(&self, member: &str) -> Vec<(Step, StepState)> {
        let wanted = MemberRef::parse(member).unwrap();

        self.events()
            .into_iter()
            .filter_map(|event| match event {
                ProgressEvent::Member(MemberProgress {
                    member,
                    step,
                    state,
                }) if member == wanted => Some((step, state)),
                _ => None,
            })
            .collect()
    }
}
