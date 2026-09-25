//! The reference `commit` command creates a validated plan and changes no Git state.
use koru::{
    ai::{
        AiRequest, AiResult, AiService, FinishReason, ServiceCapabilities, ServiceError,
        ServiceEvents, Usage,
    },
    error::{ErrorCode, Result},
    json::JsonValue,
    lua::{ApprovalProvider, LoadedCommand},
    permissions::{Decision, PreparedAction},
    runtime::{ExecutionContext, Limits},
    source::{SourceBundle, SourceLimits},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Instant,
};

static CURRENT_DIR: Mutex<()> = Mutex::new(());

struct CwdGuard(PathBuf);
impl CwdGuard {
    fn enter(path: &Path) -> Self {
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(path).unwrap();
        Self(previous)
    }
}
impl Drop for CwdGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).unwrap();
    }
}

struct Approval {
    decision: Decision,
    previews: Vec<String>,
}
impl ApprovalProvider for Approval {
    fn decide(&mut self, action: &PreparedAction, _deadline: Instant) -> Result<Decision> {
        self.previews.push(action.display_preview());
        Ok(self.decision)
    }
}

struct PlanService {
    prompt: Arc<Mutex<Option<String>>>,
    invalid_id: bool,
}
impl AiService for PlanService {
    fn capabilities(&self) -> ServiceCapabilities {
        ServiceCapabilities::tool_capable("fixture")
    }

    fn run(
        &mut self,
        request: AiRequest,
        _events: &mut dyn ServiceEvents,
    ) -> std::result::Result<AiResult, ServiceError> {
        assert!(
            request.tools.is_empty(),
            "commit planning must not expose tools"
        );
        assert_eq!(request.max_turns, 1);
        *self.prompt.lock().unwrap() = Some(request.prompt.clone());
        let ids = if self.invalid_id {
            vec!["chg_ffffffffffffffffffff".to_owned()]
        } else {
            request
                .prompt
                .lines()
                .filter_map(|line| line.strip_prefix("ID: "))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let ids = ids
            .iter()
            .map(|id| format!("\"{id}\""))
            .collect::<Vec<_>>()
            .join(",");
        Ok(AiResult {
            text: format!(
                "{{\"groups\":[{{\"changes\":[{ids}],\"message\":\"feat: group staged changes\",\"rationale\":\"related work\"}}]}}"
            ),
            finish_reason: FinishReason::Stop,
            model: "fixture-model".to_owned(),
            usage: Usage::default(),
            request_id: None,
        })
    }
}

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn repository() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
    git(root.path(), &["config", "user.name", "Koru Test"]);
    git(
        root.path(),
        &["config", "user.email", "koru-test@example.invalid"],
    );
    fs::write(root.path().join("a.txt"), "before a\n").unwrap();
    fs::write(root.path().join("b.txt"), "before b\n").unwrap();
    git(root.path(), &["add", "a.txt", "b.txt"]);
    git(root.path(), &["commit", "--quiet", "-m", "initial"]);
    fs::write(root.path().join("a.txt"), "staged a\n").unwrap();
    fs::write(root.path().join("b.txt"), "staged b\n").unwrap();
    git(root.path(), &["add", "a.txt", "b.txt"]);
    fs::write(root.path().join("a.txt"), "unstaged after staging\n").unwrap();
    root
}

fn run_commit(
    root: &Path,
    approval: &mut Approval,
    invalid_id: bool,
    prompt: Arc<Mutex<Option<String>>>,
) -> Result<JsonValue> {
    let _lock = CURRENT_DIR.lock().unwrap();
    let _cwd = CwdGuard::enter(root);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/commands");
    let bundle = SourceBundle::capture(&examples, "commit", SourceLimits::default()).unwrap();
    let context =
        ExecutionContext::new("commit", bundle.digest(), Limits::default(), Instant::now())
            .unwrap();
    let loaded = LoadedCommand::load(&bundle, &context).unwrap();
    loaded.run_with_approval(
        Box::new(PlanService { prompt, invalid_id }),
        &JsonValue::Object(Default::default()),
        approval,
    )
}

#[test]
fn approved_snapshot_returns_plan_without_mutating_git_state() {
    let root = repository();
    let index_path = root.path().join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let head_before = git(root.path(), &["rev-parse", "HEAD"]);
    let status_before = git(root.path(), &["status", "--porcelain=v1"]);
    let content_before = fs::read(root.path().join("a.txt")).unwrap();
    let prompt = Arc::new(Mutex::new(None));
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        previews: Vec::new(),
    };
    let result = run_commit(root.path(), &mut approval, false, prompt.clone()).unwrap();
    let JsonValue::Object(fields) = result else {
        panic!("commit should return a plan object")
    };
    assert_eq!(
        fields.get("status"),
        Some(&JsonValue::String("plan_only".into()))
    );
    assert_eq!(fields.get("change_count"), Some(&JsonValue::Integer(2)));
    assert_eq!(approval.previews.len(), 1);
    assert!(approval.previews[0].contains("--cached"));
    assert!(approval.previews[0].contains(root.path().to_str().unwrap()));
    let model_prompt = prompt.lock().unwrap().clone().unwrap();
    assert!(!model_prompt.contains("a.txt"));
    assert!(!model_prompt.contains("b.txt"));
    assert!(model_prompt.contains("staged a"));
    assert!(!model_prompt.contains("unstaged after staging"));
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(git(root.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git(root.path(), &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), content_before);
}

#[test]
fn denied_snapshot_does_not_call_the_model_or_change_git_state() {
    let root = repository();
    let index_path = root.path().join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let head_before = git(root.path(), &["rev-parse", "HEAD"]);
    let status_before = git(root.path(), &["status", "--porcelain=v1"]);
    let prompt = Arc::new(Mutex::new(None));
    let mut approval = Approval {
        decision: Decision::Deny,
        previews: Vec::new(),
    };
    let result = run_commit(root.path(), &mut approval, false, prompt.clone()).unwrap();
    let JsonValue::Object(fields) = result else {
        panic!("commit should report a blocked plan")
    };
    assert_eq!(
        fields.get("status"),
        Some(&JsonValue::String("unavailable".into()))
    );
    assert_eq!(approval.previews.len(), 1);
    assert!(prompt.lock().unwrap().is_none());
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(
        git(root.path(), &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(git(root.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read(root.path().join("a.txt")).unwrap(),
        b"unstaged after staging\n"
    );
}

#[test]
fn unknown_model_change_id_fails_validation_without_mutation() {
    let root = repository();
    let index_path = root.path().join(".git/index");
    let index_before = fs::read(&index_path).unwrap();
    let head_before = git(root.path(), &["rev-parse", "HEAD"]);
    let status_before = git(root.path(), &["status", "--porcelain=v1"]);
    let content_before = fs::read(root.path().join("a.txt")).unwrap();
    let prompt = Arc::new(Mutex::new(None));
    let mut approval = Approval {
        decision: Decision::ApproveOnce,
        previews: Vec::new(),
    };
    let error = run_commit(root.path(), &mut approval, true, prompt).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Validation);
    assert_eq!(fs::read(&index_path).unwrap(), index_before);
    assert_eq!(git(root.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        git(root.path(), &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), content_before);
}
