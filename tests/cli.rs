use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const WK: &str = env!("CARGO_BIN_EXE_wk");

fn run(dir: &Path, bin: &str, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .current_dir(dir)
        .env_remove("WK_WORKSPACE")
        .env("GIT_CONFIG_GLOBAL", dir.join("unused.gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.org")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.org")
        .output()
        .unwrap()
}

fn ok(dir: &Path, bin: &str, args: &[&str]) -> String {
    let out = run(dir, bin, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn fixture(root: &Path) -> std::path::PathBuf {
    let ws = root.join("my space");
    ok(root, WK, &["init", "my space"]);
    for name in ["api", "ops"] {
        let source = root.join(name);
        fs::create_dir(&source).unwrap();
        ok(&source, "git", &["init", "-b", "trunk"]);
        fs::write(source.join(".gitignore"), ".env\n").unwrap();
        ok(&source, "git", &["add", ".gitignore"]);
        ok(&source, "git", &["commit", "-m", "initial"]);
        ok(&ws, WK, &["add", source.to_str().unwrap()]);
    }
    ws
}

#[test]
fn two_project_lifecycle_and_safe_close() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    fs::write(ws.join("AGENTS.md"), "keep").unwrap();
    ok(&ws, WK, &["init"]);
    assert_eq!(fs::read_to_string(ws.join("AGENTS.md")).unwrap(), "keep");
    ok(&ws, WK, &["sync"]);
    let task = ws.join("worktrees/login");
    let api = task.join("api");
    let ops = task.join("ops");
    let output = ok(&ws, WK, &["start", "login", "api", "ops"]);
    assert!(output.contains(" run login -- codex") && output.contains(" run login -- claude"));
    ok(&ws, WK, &["start", "login", "ops", "api"]);
    assert_eq!(
        ok(&ops, WK, &["path", "login", "api"]).trim(),
        api.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        ok(
            &ws,
            WK,
            &[
                "run",
                "login",
                "--",
                "git",
                "-C",
                "api",
                "branch",
                "--show-current"
            ]
        )
        .trim(),
        "wk/login"
    );
    assert_eq!(
        run(&ws, WK, &["run", "login", "--", "git", "not-a-git-command"])
            .status
            .code(),
        Some(1)
    );
    ok(&ws, WK, &["doctor"]);

    fs::write(ops.join("untracked"), "changes").unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&ok(&ops, WK, &["status", "login", "--json"])).unwrap();
    assert_eq!(state["tasks"]["login"]["projects"]["ops"]["untracked"], 1);
    assert!(!run(&ws, WK, &["close", "login"]).status.success());
    assert!(api.is_dir());
    fs::rename(ops.join("untracked"), ops.join(".env")).unwrap();
    assert!(!run(&ws, WK, &["close", "login"]).status.success());
    assert!(api.is_dir() && ops.join(".env").exists());
    fs::remove_file(ops.join(".env")).unwrap();

    fs::write(task.join("notes"), "keep").unwrap();
    ok(&ws, WK, &["close", "login"]);
    assert!(!api.exists() && !ops.exists() && task.join("notes").is_file());
    ok(
        &ws.join("repos/api"),
        "git",
        &["rev-parse", "--verify", "refs/heads/wk/login"],
    );
    assert!(ok(&ws, WK, &["status"]).contains("login [closed]"));
    ok(&ws, WK, &["doctor"]);
}

#[test]
fn interrupted_start_can_resume_from_recorded_base() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let ops = ws.join("repos/ops");
    ok(&ops, "git", &["branch", "wk"]);
    let start = ["start", "fix", "api", "ops", "--no-fetch"];
    assert!(!run(&ws, WK, &start).status.success());
    assert!(ws.join("worktrees/fix/api/.git").is_file());
    assert!(!run(&ws, WK, &["doctor"]).status.success());

    ok(&ops, "git", &["branch", "-m", "wk", "other"]);
    let source = tmp.path().join("ops");
    ok(&source, "git", &["commit", "--allow-empty", "-m", "next"]);
    ok(&ws, WK, &["sync", "ops"]);
    ok(&ws, WK, &start);
    assert_eq!(
        ok(&ws.join("worktrees/fix/ops"), "git", &["rev-parse", "HEAD"]),
        ok(&source, "git", &["rev-parse", "HEAD^"])
    );
    let tree = ws.join("worktrees/fix/ops");
    ok(&tree, "git", &["switch", "-c", "manual"]);
    assert!(!run(&ws, WK, &["doctor"]).status.success());
    assert!(!run(&ws, WK, &["close", "fix"]).status.success());
    assert!(ws.join("worktrees/fix/api").is_dir());
}

#[test]
fn shared_manifest_can_restore_clones() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let other = tmp.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::copy(ws.join("wk.yaml"), other.join("wk.yaml")).unwrap();
    ok(&other, WK, &["init"]);
    assert!(!run(&other, WK, &["doctor"]).status.success());
    ok(&other, WK, &["sync"]);
    ok(&other, WK, &["doctor"]);
    assert!(
        !run(&other, WK, &["start", "../outside", "api"])
            .status
            .success()
    );
    assert!(
        !run(&other, WK, &["start", "issue", "api", "unknown"])
            .status
            .success()
    );
    assert!(!other.join("worktrees/issue").exists());
}

#[test]
fn init_preserves_handwritten_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let edited = "# shared settings\nversion: 1\nprojects:\n  client:\n    url: 'git@example.org:team/client.git'\n";
    fs::write(dir.join("wk.yaml"), edited).unwrap();
    ok(dir, WK, &["init"]);
    let status = ok(dir, WK, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert!(status["projects"].get("client").is_some());
    assert_eq!(fs::read_to_string(dir.join("wk.yaml")).unwrap(), edited);
}

#[test]
fn default_workspace_is_isolated_and_overridable() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dev = root.join(".wk-dev");
    let wk = |args: &[&str]| {
        Command::new(WK)
            .current_dir(root)
            .env("WK_WORKSPACE", &dev)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(wk(&["init", "other"]).status.success());
    assert!(wk(&["-w", "other", "doctor"]).status.success());
    assert!(!dev.exists());

    // An uninitialized dev directory must never select an enclosing workspace.
    ok(root, WK, &["init"]);
    fs::create_dir(&dev).unwrap();
    assert!(!wk(&["doctor"]).status.success());
    assert!(wk(&["init"]).status.success());
    assert!(dev.join("wk.yaml").is_file());
    let out = wk(&["doctor"]);
    assert!(out.status.success());
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains(dev.canonicalize().unwrap().to_str().unwrap())
    );
}

#[test]
fn explicit_workspace_runs_outside_source_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path();
    let dir = "dev team's workspace";
    ok(outside, WK, &["-w", dir, "init"]);
    ok(outside, WK, &["doctor", "--workspace", dir]);
    assert!(outside.join(dir).join("wk.yaml").is_file());
    assert!(!outside.join("wk.yaml").exists());
    let source = fixture(outside);
    let repo = source.join("repos/api");
    ok(outside, WK, &["-w", dir, "add", repo.to_str().unwrap()]);
    let commands = ok(outside, WK, &["-w", dir, "start", "demo", "api"]);
    assert!(commands.contains("--workspace") && commands.contains(" run demo -- codex"));
    #[cfg(unix)]
    {
        let line = commands.lines().find(|s| s.ends_with(" -- codex")).unwrap();
        let command = line.replace(" -- codex", " -- git -C api branch --show-current");
        assert_eq!(ok(outside, "sh", &["-c", &command]).trim(), "wk/demo");
    }
}
