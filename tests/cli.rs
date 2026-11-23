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
fn clone_persists_defaults_and_refuses_to_adopt_or_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ws = fixture(root);
    let manifest = ws.join("wk.yaml");
    let source = root.join("api");
    let url = source.to_str().unwrap();
    let manifest_projects = || {
        let manifest: serde_json::Value =
            serde_saphyr::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
        manifest["projects"].clone()
    };
    let projects = manifest_projects();
    assert_eq!(projects["api"]["name"], "api");
    assert_eq!(projects["api"]["base"], "origin/trunk");
    ok(
        &ws,
        WK,
        &["clone", url, "preview", "--base", "origin/topic"],
    );
    let projects = manifest_projects();
    assert_eq!(
        projects["preview"],
        serde_json::json!({
            "name": "preview", "url": projects["api"]["url"], "base": "origin/topic"
        })
    );
    assert!(ws.join("repos/preview/.git").is_dir());

    let before = fs::read(&manifest).unwrap();
    fs::create_dir(ws.join("repos/empty")).unwrap();
    ok(&ws, "git", &["clone", url, "repos/manual"]);
    let occupied = ws.join("repos/file");
    fs::write(&occupied, "keep").unwrap();
    for dest in ["api", "empty", "manual", "file"] {
        assert!(
            !run(&ws, WK, &["add", url, dest]).status.success(),
            "{dest}"
        );
    }
    fs::rename(ws.join("repos/api"), root.join("previous-clone")).unwrap();
    assert!(
        !run(&ws, WK, &["add", url, "--base", "origin/changed"])
            .status
            .success()
    );
    assert!(!ws.join("repos/api").exists());
    assert!(ws.join("repos/manual/.git").is_dir());
    assert_eq!(fs::read_to_string(&occupied).unwrap(), "keep");
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert!(
        !run(&ws, WK, &["clone", url, "--name", "other"])
            .status
            .success()
    );

    ok(&ws, WK, &["sync", "api"]);
    assert!(ws.join("repos/api/.git").is_dir());
}

#[test]
fn remove_guards_tasks_worktrees_and_unpushed_commits() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let manifest = ws.join("wk.yaml");
    let before = fs::read(&manifest).unwrap();
    let repo = ws.join("repos/api");
    ok(&ws, WK, &["start", "fix", "api", "--no-fetch"]);
    let result = run(&ws, WK, &["rm", "api"]);
    assert!(!result.status.success());
    assert!(String::from_utf8(result.stderr).unwrap().contains("fix"));
    let tree = ws.join("tasks/fix/api");
    ok(
        &tree,
        "git",
        &["commit", "--allow-empty", "-m", "local work"],
    );
    ok(&ws, WK, &["close", "fix"]);
    let result = run(&ws, WK, &["remove", "api"]);
    assert!(!result.status.success());
    assert!(String::from_utf8(result.stderr).unwrap().contains("commit"));
    assert_eq!(fs::read(&manifest).unwrap(), before);

    ok(&repo, "git", &["push", "origin", "wk/fix"]);
    let manual = tmp.path().join("manual");
    ok(
        &repo,
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            manual.to_str().unwrap(),
            "origin/trunk",
        ],
    );
    let result = run(&ws, WK, &["remove", "api"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("worktree")
    );
    assert!(manual.join(".git").is_file());
    ok(
        &repo,
        "git",
        &["worktree", "remove", manual.to_str().unwrap()],
    );
    ok(&ws, WK, &["rm", "api"]);
    assert!(!repo.exists());
    assert!(ws.join("repos/ops").exists());
    assert!(ws.join(".wk/tasks/fix.json").is_file());
    assert!(!ok(&ws, WK, &["status", "--json"]).contains("\"api\""));
    ok(&ws, WK, &["doctor"]);
    ok(&ws, WK, &["add", tmp.path().join("api").to_str().unwrap()]);
}

#[test]
fn remove_preserves_populated_gitlinks_and_allows_empty_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ws = fixture(root);
    let source = root.join("api");
    let dependency = root.join("ops");
    let subpath = "vendor/shared library";
    ok(
        &source,
        "git",
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            dependency.to_str().unwrap(),
            subpath,
        ],
    );
    ok(&source, "git", &["commit", "-m", "register submodule"]);
    ok(&ws, WK, &["add", source.to_str().unwrap(), "parent"]);

    let repo = ws.join("repos/parent");
    let sub = repo.join(subpath);
    ok(
        &repo,
        "git",
        &["clone", dependency.to_str().unwrap(), subpath],
    );
    assert!(sub.join(".git").is_dir());
    assert!(!repo.join(".git/modules").exists());
    ok(&sub, "git", &["switch", "-c", "private"]);
    ok(
        &sub,
        "git",
        &["commit", "--allow-empty", "-m", "private commit"],
    );
    let private_commit = ok(&sub, "git", &["rev-parse", "HEAD"]);
    ok(&sub, "git", &["switch", "trunk"]);
    fs::write(sub.join(".env"), "local settings").unwrap();
    assert!(ok(&repo, "git", &["status", "--porcelain", "--ignored"]).is_empty());

    let manifest = ws.join("wk.yaml");
    let before = fs::read(&manifest).unwrap();
    let result = run(&ws, WK, &["rm", "parent"]);
    assert!(
        !result.status.success(),
        "wk rm deleted the populated submodule"
    );
    assert!(String::from_utf8(result.stderr).unwrap().contains(subpath));
    assert_eq!(fs::read(&manifest).unwrap(), before);
    assert_eq!(
        fs::read_to_string(sub.join(".env")).unwrap(),
        "local settings"
    );
    assert_eq!(ok(&sub, "git", &["rev-parse", "private"]), private_commit);

    fs::rename(&sub, root.join("saved submodule")).unwrap();
    fs::create_dir(&sub).unwrap();
    ok(&ws, WK, &["rm", "parent"]);
    assert!(!repo.exists());
}

#[test]
fn remove_missing_clone_only_unregisters_target() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let moved = tmp.path().join("moved");
    fs::rename(ws.join("repos/api"), &moved).unwrap();
    ok(&ws, WK, &["rm", "api"]);
    assert!(moved.exists());
    assert!(!run(&ws, WK, &["rm", "api"]).status.success());
    ok(&ws, WK, &["sync"]);
    assert!(!ws.join("repos/api").exists());
}

#[test]
fn project_files_belong_to_task_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let repo = ws.join("repos/api");
    assert_eq!(
        ok(&repo, "git", &["rev-parse", "--is-bare-repository"]).trim(),
        "false"
    );
    let reference = fs::read(repo.join(".gitignore")).unwrap();
    ok(&ws, WK, &["start", "implement", "api"]);
    let tree = ws.join("tasks/implement/api");
    fs::write(tree.join(".gitignore"), "task change").unwrap();
    assert_eq!(fs::read(repo.join(".gitignore")).unwrap(), reference);
    fs::write(tree.join(".gitignore"), &reference).unwrap();
    ok(&ws, WK, &["close", "implement"]);
    let manifest = fs::read(ws.join("wk.yaml")).unwrap();
    fs::write(repo.join(".env"), "keep").unwrap();
    for args in [
        &["start", "new", "api"][..],
        &["doctor"],
        &["sync", "api"],
        &["status", "--fetch"],
        &["rm", "api"],
    ] {
        assert!(!run(&ws, WK, args).status.success(), "{args:?}");
    }
    assert_eq!(fs::read_to_string(repo.join(".env")).unwrap(), "keep");
    assert!(ok(&ws, WK, &["status", "--json"]).contains("ignored files"));
    assert_eq!(fs::read(ws.join("wk.yaml")).unwrap(), manifest);
    fs::remove_file(repo.join(".env")).unwrap();
    ok(
        &repo,
        "git",
        &["update-index", "--assume-unchanged", ".gitignore"],
    );
    fs::write(repo.join(".gitignore"), "accidental edit").unwrap();
    let hidden = run(&ws, WK, &["rm", "api"]);
    assert!(!hidden.status.success());
    assert!(
        String::from_utf8(hidden.stderr)
            .unwrap()
            .contains("assume-unchanged")
    );
    ok(
        &repo,
        "git",
        &["update-index", "--no-assume-unchanged", ".gitignore"],
    );
    assert!(!run(&ws, WK, &["rm", "api"]).status.success());
    ok(&repo, "git", &["add", ".gitignore"]);
    assert!(!run(&ws, WK, &["start", "new", "api"]).status.success());
    ok(&repo, "git", &["commit", "-m", "accidental base commit"]);
    assert!(!run(&ws, WK, &["rm", "api"]).status.success());
    assert!(!run(&ws, WK, &["sync", "api"]).status.success());
    assert!(ok(&ws, WK, &["status"]).contains("reference HEAD"));
}

#[test]
fn clone_without_advertised_head_requires_an_explicit_base() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    ok(root, "git", &["init", "--bare", "-b", "trunk", "empty.git"]);
    ok(root, WK, &["init", "space"]);
    let before = fs::read(root.join("space/wk.yaml")).unwrap();
    let missing = run(root, WK, &["-w", "space", "clone", "./empty.git"]);
    assert!(!missing.status.success());
    assert!(
        String::from_utf8(missing.stderr)
            .unwrap()
            .contains("--base")
    );
    assert!(!root.join("space/repos/empty").exists());
    assert_eq!(fs::read(root.join("space/wk.yaml")).unwrap(), before);
    ok(
        root,
        WK,
        &[
            "-w",
            "space",
            "clone",
            "./empty.git",
            "--base",
            "origin/trunk",
        ],
    );
    assert!(root.join("space/repos/empty/.git").is_dir());
}

#[test]
fn two_project_lifecycle_and_safe_close() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    fs::write(ws.join("AGENTS.md"), "keep").unwrap();
    ok(&ws, WK, &["init"]);
    assert_eq!(fs::read_to_string(ws.join("AGENTS.md")).unwrap(), "keep");
    assert!(!ws.join("worktrees").exists());
    assert!(
        fs::read_to_string(ws.join(".gitignore"))
            .unwrap()
            .lines()
            .any(|rule| rule == "/tasks/")
    );
    ok(&ws, WK, &["sync"]);
    let task = ws.join("tasks/login");
    let api = task.join("api");
    let ops = task.join("ops");
    let output = ok(&ws, WK, &["start", "login", "api", "ops"]);
    assert!(output.contains(" run login -- codex") && output.contains(" run login -- claude"));
    ok(&ws, WK, &["start", "login", "ops", "api"]);
    assert_eq!(
        ok(&api, WK, &["path", "login"]).trim(),
        task.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(task.join("AGENTS.md").is_file());
    fs::create_dir(task.join("fixtures")).unwrap();
    fs::write(task.join("fixtures/shared.txt"), "task data").unwrap();
    fs::write(task.join("CLAUDE.md"), "@AGENTS.md").unwrap();
    assert_eq!(
        ok(
            &ws,
            WK,
            &[
                "run",
                "login",
                "--",
                "git",
                "hash-object",
                "fixtures/shared.txt"
            ]
        ),
        ok(&task, "git", &["hash-object", "fixtures/shared.txt"])
    );
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
    assert!(api.is_dir() && task.join("fixtures/shared.txt").is_file());
    fs::rename(ops.join("untracked"), ops.join(".env")).unwrap();
    assert!(!run(&ws, WK, &["close", "login"]).status.success());
    assert!(api.is_dir() && ops.join(".env").exists());
    fs::remove_file(ops.join(".env")).unwrap();

    ok(&ws, WK, &["start", "parallel", "api", "--no-fetch"]);
    let external = tmp.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("keep"), "external").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external, task.join("external-link")).unwrap();
    ok(&ws, WK, &["close", "login"]);
    assert!(!task.exists());
    assert!(ws.join("tasks/parallel/api/.git").is_file());
    assert!(external.join("keep").is_file());
    for p in ["api", "ops"] {
        let trees = ok(
            &ws.join("repos").join(p),
            "git",
            &["worktree", "list", "--porcelain"],
        );
        assert!(!trees.contains("refs/heads/wk/login"), "{trees}");
    }
    ok(
        &ws.join("repos/api"),
        "git",
        &["rev-parse", "--verify", "refs/heads/wk/login"],
    );
    assert!(ok(&ws, WK, &["status"]).contains("login [closed]"));
    ok(&ws, WK, &["doctor"]);
}

#[test]
fn close_can_finish_interrupted_directory_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    ok(&ws, WK, &["start", "retry", "api", "--no-fetch"]);
    let root = ws.join("tasks/retry");
    let record = ws.join(".wk/tasks/retry.json");
    let mut task: serde_json::Value = serde_json::from_slice(&fs::read(&record).unwrap()).unwrap();
    task["phase"] = "closing".into();
    fs::write(&record, serde_json::to_vec(&task).unwrap()).unwrap();
    ok(
        &ws.join("repos/api"),
        "git",
        &["worktree", "remove", root.join("api").to_str().unwrap()],
    );
    fs::write(root.join("brief.md"), "shared").unwrap();
    ok(&ws, WK, &["close", "retry"]);
    assert!(!root.exists());
    // The directory may already be gone when the closing record is retried.
    fs::write(&record, serde_json::to_vec(&task).unwrap()).unwrap();
    ok(&ws, WK, &["close", "retry"]);
    assert!(ok(&ws, WK, &["status", "retry"]).contains("[closed]"));
}

#[test]
fn interrupted_start_can_resume_from_recorded_base() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let ops = ws.join("repos/ops");
    ok(&ops, "git", &["branch", "wk"]);
    let start = ["start", "fix", "api", "ops", "--no-fetch"];
    assert!(!run(&ws, WK, &start).status.success());
    assert!(ws.join("tasks/fix/api/.git").is_file());
    assert!(!run(&ws, WK, &["doctor"]).status.success());

    ok(&ops, "git", &["branch", "-m", "wk", "other"]);
    let source = tmp.path().join("ops");
    ok(&source, "git", &["commit", "--allow-empty", "-m", "next"]);
    ok(&ws, WK, &["sync", "ops"]);
    ok(&ws, WK, &["start", "fix"]);
    assert_eq!(
        ok(&ws.join("tasks/fix/ops"), "git", &["rev-parse", "HEAD"]),
        ok(&source, "git", &["rev-parse", "HEAD^"])
    );
    let tree = ws.join("tasks/fix/ops");
    ok(&tree, "git", &["switch", "-c", "manual"]);
    assert!(!run(&ws, WK, &["doctor"]).status.success());
    assert!(!run(&ws, WK, &["close", "fix"]).status.success());
    assert!(ws.join("tasks/fix/api").is_dir());
}

#[test]
fn scripted_start_requires_explicit_projects_for_new_tasks() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    for args in [&["start", "demo"][..], &["start", "demo", "-D"]] {
        let result = run(&ws, WK, args);
        assert!(!result.status.success());
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("specify projects")
        );
        assert!(!ws.join(".wk/tasks/demo.json").exists());
    }
    ok(
        &ws,
        WK,
        &["start", "demo", "--default", "ops", "--no-fetch"],
    );
    assert!(ws.join("tasks/demo/ops/.git").is_file());
    assert!(!ws.join("tasks/demo/api").exists());
    assert!(!run(&ws, WK, &["start", "demo", "api"]).status.success());
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
    assert!(!other.join("tasks/issue").exists());
}

#[test]
fn init_preserves_handwritten_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let edited = "version: 1\nprojects:\n  client:\n    name: client\n    url: 'git@example.org:team/client.git'\n    base: origin/trunk\n";
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
