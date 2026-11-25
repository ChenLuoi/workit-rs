use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

const WK: &str = env!("CARGO_BIN_EXE_wk");

fn active_index(ws: &Path) -> serde_json::Value {
    serde_saphyr::from_str(&fs::read_to_string(ws.join(".wk/active.yaml")).unwrap()).unwrap()
}

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

fn source_repo(root: &Path, name: &str) -> std::path::PathBuf {
    let source = root.join(name);
    fs::create_dir(&source).unwrap();
    ok(&source, "git", &["init", "-b", "trunk"]);
    fs::write(source.join(".gitignore"), ".env\n").unwrap();
    ok(&source, "git", &["add", ".gitignore"]);
    ok(&source, "git", &["commit", "-m", "initial"]);
    source
}

fn fixture(root: &Path) -> std::path::PathBuf {
    let ws = root.join("my space");
    source_repo(root, "api");
    ok(root, WK, &["init", "my space", "--yes", "--url", "./api"]);
    let ops = source_repo(root, "ops");
    ok(&ws, WK, &["add", ops.to_str().unwrap()]);
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
    ok(&source, "git", &["branch", "topic"]);
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

    ok(&repo, "git", &["push", "origin", "fix"]);
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
    assert_eq!(fs::read_dir(ws.join(".wk/history")).unwrap().count(), 1);
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
    let scoped: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", "implement", "--json"])).unwrap();
    assert_eq!(scoped["projects"].as_object().unwrap().len(), 1);
    assert_eq!(
        scoped["projects"]["api"]["commit"],
        ok(&repo, "git", &["rev-parse", "HEAD"]).trim()
    );
    assert_eq!(
        scoped["tasks"]["implement"]["projects"]["api"]["modified"],
        1
    );
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
    let url = ok(&repo, "git", &["config", "--get", "remote.origin.url"]);
    let status: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", "--json"])).unwrap();
    assert_eq!(
        status["projects"]["api"],
        serde_json::json!({
            "name": "api", "url": url.trim(), "base": "origin/trunk", "commit": ok(&repo, "git", &["rev-parse", "HEAD"]).trim()
        })
    );
    let doctor = run(&ws, WK, &["doctor"]);
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("• Ignored files  1"));
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
    let status = ok(&ws, WK, &["status"]);
    let row = status
        .lines()
        .find(|line| line.trim_start().starts_with("api "))
        .unwrap();
    assert_eq!(
        row.strip_suffix(url.trim())
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>(),
        [
            "api",
            &ok(&repo, "git", &["rev-parse", "HEAD"])[..12],
            "origin/trunk"
        ]
    );
    assert!(!status.contains('\x1b'));
    let doctor = run(&ws, WK, &["doctor"]);
    assert!(!doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("reference HEAD"));
}

#[test]
fn clone_without_advertised_head_requires_an_explicit_base() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    ok(root, "git", &["init", "--bare", "-b", "trunk", "empty.git"]);
    ok(
        root,
        WK,
        &[
            "init",
            "from-empty",
            "--yes",
            "--url",
            "./empty.git",
            "--base",
            "origin/trunk",
        ],
    );
    assert!(root.join("from-empty/repos/empty/.git").is_dir());
    source_repo(root, "first");
    ok(root, WK, &["init", "space", "--yes", "--url", "./first"]);
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
    assert!(!run(&ws, WK, &["init"]).status.success());
    assert_eq!(fs::read_to_string(ws.join("AGENTS.md")).unwrap(), "keep");
    assert!(!ws.join("worktrees").exists());
    assert!(
        fs::read_to_string(ws.join("tasks/.gitignore"))
            .unwrap()
            .lines()
            .any(|rule| rule == "/*")
    );
    ok(&ws, WK, &["sync"]);
    let task = ws.join("tasks/login");
    let api = task.join("api");
    let ops = task.join("ops");
    let output = ok(&ws, WK, &["start", "login", "api", "ops"]);
    assert!(!output.contains(" run login "));
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
        ok(&ops, WK, &["path", "login", "api"]).trim(),
        api.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(
        !run(&ws, WK, &["run", "login", "--", "git", "status"])
            .status
            .success()
    );
    ok(&ws, WK, &["doctor"]);

    fs::write(ops.join("untracked"), "changes").unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&ok(&ops, WK, &["status", "login", "--json"])).unwrap();
    assert_eq!(state["tasks"]["login"]["projects"]["ops"]["untracked"], 1);
    let text = ok(&ops, WK, &["status", "login"]);
    assert!(text.contains("✓ clean") && text.contains("1 untracked"));
    assert!(!text.contains("0 staged"));
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
        assert!(!trees.contains("refs/heads/login"), "{trees}");
    }
    ok(
        &ws.join("repos/api"),
        "git",
        &["rev-parse", "--verify", "refs/heads/login"],
    );
    let overview = ok(&ws, WK, &["status"]);
    assert!(overview.contains("parallel") && !overview.contains("login"));
    ok(&ws, WK, &["doctor"]);
}

#[test]
fn status_compares_the_recorded_start_to_head_and_separates_pending_files() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let name = "feature/login";
    ok(&ws, WK, &["start", name, "api", "--no-fetch"]);
    let tree = ws.join("tasks").join(name).join("api");
    let start = ok(&tree, "git", &["rev-parse", "HEAD"]);
    ok(&tree, "git", &["mv", ".gitignore", "renamed file"]);
    ok(&tree, "git", &["commit", "-m", "rename"]);
    let head = ok(&tree, "git", &["rev-parse", "HEAD"]);
    ok(&tree, "git", &["mv", "renamed file", "final name"]);
    fs::write(tree.join("final name"), "edited after staging").unwrap();
    fs::write(tree.join("new file"), "untracked").unwrap();

    let origin = tmp.path().join("api");
    ok(
        &origin,
        "git",
        &["commit", "--allow-empty", "-m", "advance trunk"],
    );
    fs::write(ws.join("repos/ops/unrelated"), "skip this project").unwrap();
    let report: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", name, "--fetch", "--json"])).unwrap();
    let api = &report["tasks"][name]["projects"]["api"];
    assert_eq!(
        api["start"],
        serde_json::json!({"base":"origin/trunk", "commit":start.trim()})
    );
    assert_ne!(ok(&tree, "git", &["rev-parse", "origin/trunk"]), start);
    assert_eq!(api["commit"], head.trim());
    assert_eq!(api["branch"], name);
    assert_eq!(
        api["committed_changes"],
        serde_json::json!([
            {"status":"R100", "path":"renamed file", "original_path":".gitignore"}
        ])
    );
    assert_eq!(
        api["files"],
        serde_json::json!([
            {"status":"RM", "path":"final name", "original_path":"renamed file"},
            {"status":"??", "path":"new file"}
        ])
    );
    let detail = ok(&ws, WK, &["status", name]);
    for label in [
        "tasks/feature/login/",
        "origin/trunk",
        &start[..12],
        &head[..12],
        ".gitignore → renamed file",
        "1 staged",
        "1 modified",
        "??   new file",
    ] {
        assert!(detail.contains(label), "{detail}");
    }
    assert!(!detail.contains("ops"));
    let overview = ok(&ws, WK, &["status"]);
    assert!(overview.contains("ops") && overview.contains(name));
    assert!(!overview.contains("renamed file"));

    fs::rename(&tree, tmp.path().join("moved-tree")).unwrap();
    let broken: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", name, "--json"])).unwrap();
    let member = &broken["tasks"][name]["projects"]["api"];
    assert_eq!(member["start"], api["start"]);
    assert!(member["error"].is_string() && member.get("commit").is_none());
}

#[test]
fn close_can_finish_interrupted_directory_cleanup() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let name = "maintenance/retry";
    ok(&ws, WK, &["start", name, "api", "--no-fetch"]);
    let root = ws.join("tasks").join(name);
    let record = ws.join(".wk/active.yaml");
    let mut index = active_index(&ws);
    let task = &mut index["tasks"][name];
    task["phase"] = "closing".into();
    task["closing_at_unix_ms"] = task["started_at_unix_ms"].clone();
    task["projects"]["api"]["final_commit"] = task["projects"]["api"]["commit"].clone();
    let closing = serde_saphyr::to_string(&index).unwrap();
    fs::write(&record, &closing).unwrap();
    let out = run(&ws, WK, &["run"]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("wk start"));
    assert!(!ok(&ws, WK, &["status"]).contains(name));
    ok(
        &ws.join("repos/api"),
        "git",
        &["worktree", "remove", root.join("api").to_str().unwrap()],
    );
    fs::write(root.join("brief.md"), "shared").unwrap();
    ok(&ws, WK, &["close", name]);
    assert!(!root.exists());
    assert!(!ws.join("tasks/maintenance").exists());
    // The directory may already be gone when the closing record is retried.
    fs::write(&record, closing).unwrap();
    ok(&ws, WK, &["close", name]);
    let archived = run(&ws, WK, &["status", name, "--json"]);
    assert!(!archived.status.success() && archived.stdout.is_empty());
    assert!(String::from_utf8_lossy(&archived.stderr).contains("no open task"));
    assert_eq!(fs::read_dir(ws.join(".wk/history")).unwrap().count(), 1);
}

#[test]
fn interrupted_start_can_resume_from_recorded_base() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let ops = ws.join("repos/ops");
    ok(&ops, "git", &["branch", "maintenance"]);
    let name = "maintenance/fix";
    let dir = ws.join("tasks").join(name);
    let start = ["start", name, "api", "ops", "--no-fetch"];
    assert!(!run(&ws, WK, &start).status.success());
    assert!(dir.join("api/.git").is_file());
    assert!(!ok(&ws, WK, &["status"]).contains(name));
    assert!(!run(&ws, WK, &["doctor"]).status.success());

    ok(&ops, "git", &["branch", "-m", "maintenance", "other"]);
    let source = tmp.path().join("ops");
    ok(&source, "git", &["commit", "--allow-empty", "-m", "next"]);
    ok(&ws, WK, &["sync", "ops"]);
    ok(&ws, WK, &["start", name]);
    assert_eq!(
        ok(&dir.join("ops"), "git", &["rev-parse", "HEAD"]),
        ok(&source, "git", &["rev-parse", "HEAD^"])
    );
    let tree = dir.join("ops");
    ok(&tree, "git", &["switch", "-c", "manual"]);
    assert!(!run(&ws, WK, &["doctor"]).status.success());
    assert!(!run(&ws, WK, &["close", name]).status.success());
    assert!(dir.join("api").is_dir());
}

#[test]
fn task_branch_names_coexist_with_tags_and_remote_refs() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let repo = ws.join("repos/api");
    let start = |name: &str| ok(&ws, WK, &["start", name, "api", "--no-fetch"]);
    let collision = run(&ws, WK, &["start", "trunk", "api", "--no-fetch"]);
    assert!(!collision.status.success());
    assert!(String::from_utf8_lossy(&collision.stderr).contains("branch trunk already exists"));
    assert!(active_index(&ws)["tasks"]["trunk"].is_null());

    let base = ok(&repo, "git", &["rev-parse", "refs/remotes/origin/trunk"]);
    start("origin/trunk");
    let tree = ws.join("tasks/origin/trunk/api");
    ok(
        &tree,
        "git",
        &["commit", "--allow-empty", "-m", "task progress"],
    );
    ok(&repo, "git", &["tag", "release"]);
    start("release");
    let release = ws.join("tasks/release/api");
    assert_eq!(ok(&release, "git", &["rev-parse", "HEAD"]), base);
    assert_eq!(
        ok(&release, "git", &["branch", "--show-current"]).trim(),
        "release"
    );
    ok(&ws, WK, &["doctor"]);
    ok(&ws, WK, &["close", "release"]);
    ok(&ws, WK, &["close", "origin/trunk"]);
}

#[test]
fn grouped_tasks_keep_their_names_and_close_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let start = |name: &str| ok(&ws, WK, &["start", name, "api", "--no-fetch"]);
    let first = "feature/ui/login";
    let sibling = "feature/ui/logout";
    start(first);
    start(sibling);
    start("feature-ui-login");
    for name in [first, sibling, "feature-ui-login"] {
        let tree = ws.join("tasks").join(name).join("api");
        assert_eq!(ok(&tree, "git", &["branch", "--show-current"]).trim(), name);
        assert_eq!(
            ok(&ws, WK, &["path", name, "api"]).trim(),
            tree.canonicalize().unwrap().to_str().unwrap()
        );
    }
    assert_eq!(active_index(&ws)["tasks"][first]["phase"], "active");
    ok(&ws, WK, &["start", first]);
    ok(&ws, WK, &["doctor"]);
    for name in [
        "feature",
        "feature/ui",
        "feature/ui/login/child",
        "Feature/ui/other",
    ] {
        assert!(
            !run(&ws, WK, &["start", name, "ops", "--no-fetch"])
                .status
                .success(),
            "{name}"
        );
    }
    fs::write(ws.join("tasks/feature/ui/notes"), "keep").unwrap();
    let doctor = run(&ws, WK, &["doctor"]);
    assert!(!doctor.status.success());
    assert!(
        String::from_utf8_lossy(&doctor.stdout)
            .contains("unregistered path tasks/feature/ui/notes")
    );
    ok(&ws, WK, &["close", first]);
    assert!(!ws.join("tasks").join(first).exists());
    assert!(ws.join("tasks").join(sibling).join("api/.git").is_file());
    assert!(!run(&ws, WK, &["rm", "api"]).status.success());
    ok(&ws, WK, &["close", sibling]);
    assert_eq!(
        fs::read_to_string(ws.join("tasks/feature/ui/notes")).unwrap(),
        "keep"
    );
    fs::remove_file(ws.join("tasks/feature/ui/notes")).unwrap();
    start("feature/ui/next");
    ok(&ws, WK, &["close", "feature/ui/next"]);
    assert!(!ws.join("tasks/feature").exists());
    assert!(ws.join("tasks/feature-ui-login/api/.git").is_file());
    ok(&ws, WK, &["start", "feature", "ops", "--no-fetch"]);
    let report: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", "--json"])).unwrap();
    assert!(report["tasks"].get(first).is_none());
    assert_eq!(report["tasks"]["feature-ui-login"]["phase"], "active");
    ok(&ws, WK, &["doctor"]);
}

#[test]
fn invalid_task_paths_cannot_escape_the_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    for name in [
        "/absolute",
        "HEAD",
        "../outside",
        "group/../outside",
        "group//name",
        "group/",
        "group/.",
        "group\\name",
        "group%2Fname",
    ] {
        assert!(
            !run(&ws, WK, &["start", name, "api", "--no-fetch"])
                .status
                .success(),
            "{name}"
        );
    }
    #[cfg(unix)]
    {
        let external = tmp.path().join("external");
        fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, ws.join("tasks/link")).unwrap();
        assert!(
            !run(&ws, WK, &["start", "link/escape", "api", "--no-fetch"])
                .status
                .success()
        );
        assert!(!external.join("escape").exists());
    }
    assert_eq!(active_index(&ws)["tasks"], serde_json::json!({}));
    #[cfg(unix)]
    ok(&ws, WK, &["start", "group/nuL.txt", "api", "--no-fetch"]);
}

#[test]
fn scripted_start_requires_explicit_projects_for_new_tasks() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let missing = run(&ws, WK, &["start", "-D"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("specify a task name"));
    for args in [&["start", "demo"][..], &["start", "demo", "-D"]] {
        let result = run(&ws, WK, args);
        assert!(!result.status.success());
        assert!(
            String::from_utf8(result.stderr)
                .unwrap()
                .contains("specify projects")
        );
        assert!(active_index(&ws)["tasks"]["demo"].is_null());
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
fn doctor_summarizes_projects_and_active_tasks_and_expands_only_problems() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    ok(&ws, WK, &["start", "feature/login", "api", "--no-fetch"]);
    ok(&ws, WK, &["start", "archived", "ops", "--no-fetch"]);
    ok(&ws, WK, &["close", "archived"]);
    fs::write(ws.join("tasks/feature/login/api/work"), "in progress").unwrap();
    let healthy = ok(&ws, WK, &["doctor"]);
    for line in [
        "PASS  Git executable",
        "PASS  Workspace discovery",
        "PASS  wk.yaml schema",
        "PASS  Workspace format version  1",
        "PASS  Project inventory  2 registered",
        "PASS  Task inventory  1 active",
        "PASS  Registered paths in tasks/feature/",
        "0 failed, 0 skipped",
    ] {
        assert!(healthy.contains(line), "missing {line} in {healthy}");
    }
    assert!(healthy.contains("\nProjects\n  ✓ PASS  api\n  ✓ PASS  ops\n"));
    assert!(healthy.contains("\nActive tasks\n  ✓ PASS  feature/login\n\nDirectory layout"));
    for hidden in [
        "archived",
        "Worktree",
        "Origin matches",
        "Staged files",
        "Task phase",
        "Task branch",
    ] {
        assert!(
            !healthy.contains(hidden),
            "unexpected {hidden} in {healthy}"
        );
    }

    let repo = ws.join("repos/api");
    ok(&repo, "git", &["switch", "-c", "local"]);
    ok(
        &repo,
        "git",
        &["commit", "--allow-empty", "-m", "local commit"],
    );
    fs::write(repo.join(".gitignore"), ".env\n*.bak\n").unwrap();
    fs::write(repo.join(".env"), "keep").unwrap();
    fs::write(repo.join("new"), "keep").unwrap();
    fs::rename(ws.join("repos/ops"), tmp.path().join("saved")).unwrap();
    ok(
        &ws.join("tasks/feature/login/api"),
        "git",
        &["switch", "-c", "manual"],
    );
    let mut index = active_index(&ws);
    let mut incomplete = index["tasks"]["feature/login"].clone();
    incomplete["phase"] = "closing".into();
    index["tasks"]["interrupted"] = incomplete;
    index["tasks"]["broken"] = serde_json::json!({"unexpected": true});
    fs::write(
        ws.join(".wk/active.yaml"),
        serde_saphyr::to_string(&index).unwrap(),
    )
    .unwrap();
    let out = run(&ws, WK, &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let report = String::from_utf8(out.stdout).unwrap();
    for line in [
        "FAIL  api",
        "• Modified files  1",
        "• Untracked files  1",
        "• Ignored files  1",
        "• Detached snapshot",
        "• Snapshot commit",
        "FAIL  ops\n          • Clone directory",
        "FAIL  Task inventory  1 active",
        "• broken: Task record",
        "• interrupted: incomplete (closing)",
        "Active tasks\n  ✗ FAIL  feature/login\n          • api: Task branch",
        "PASS  Registered paths in repos/",
    ] {
        assert!(report.contains(line), "missing {line} in {report}");
    }
    for hidden in [
        "archived",
        "\nWorktree ",
        "Staged files  0",
        "Origin matches wk.yaml",
        "PASS  Task branch",
        "FAIL  interrupted",
    ] {
        assert!(!report.contains(hidden), "unexpected {hidden} in {report}");
    }
    let total = |tag: &str| {
        report
            .lines()
            .filter(|line| line.trim_start().starts_with(tag))
            .count()
    };
    assert!(report.ends_with(&format!(
        "Summary: {} passed, {} failed, {} skipped\n",
        total("✓ PASS"),
        total("✗ FAIL"),
        total("– SKIP")
    )));
    assert_eq!(fs::read_to_string(repo.join(".env")).unwrap(), "keep");
}

#[test]
fn doctor_continues_when_workspace_configuration_is_damaged() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let manifest = ws.join("wk.yaml");
    let mut config: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
    config["projects"]["api"]
        .as_object_mut()
        .unwrap()
        .remove("url");
    config["projects"]["broken"] = serde_json::json!(["invalid entry"]);
    let yaml = serde_saphyr::to_string(&config).unwrap().replacen(
        "projects:\n",
        "projects:\n  123:\n    name: 123\n    url: git@example.org:team/source.git\n    base: 123\n",
        1,
    );
    fs::write(&manifest, yaml).unwrap();
    let out = run(&ws, WK, &["doctor"]);
    assert!(!out.status.success());
    let report = String::from_utf8(out.stdout).unwrap();
    assert!(
        report.contains("FAIL  api\n          • Project fields")
            && !report.contains("SKIP  Git URL"),
        "{report}"
    );
    assert!(report.contains("FAIL  123\n          • Clone directory"));
    assert!(report.contains("FAIL  broken\n          • Project fields"));
    assert!(report.contains("PASS  ops"));

    fs::write(&manifest, "[").unwrap();
    let out = run(&ws, WK, &["doctor"]);
    assert!(!out.status.success());
    let report = String::from_utf8(out.stdout).unwrap();
    assert!(
        report.contains("FAIL  wk.yaml schema")
            && report.contains("SKIP  Workspace format version")
    );
    assert!(report.contains("PASS  Task inventory") && report.contains("PASS  repos/ directory"));
    assert!(report.contains("SKIP  Registered paths in repos/") && report.contains("Summary:"));
}

#[test]
fn shared_manifest_can_restore_clones() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let other = tmp.path().join("other");
    fs::create_dir(&other).unwrap();
    fs::copy(ws.join("wk.yaml"), other.join("wk.yaml")).unwrap();
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
fn workspace_commit_shares_only_the_definition_and_sync_rebuilds_local_state() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    ok(
        &ws,
        WK,
        &["start", "feature/demo", "api", "ops", "--no-fetch"],
    );
    let tree = ws.join("tasks/feature/demo/api");
    let start = ok(&tree, "git", &["rev-parse", "HEAD"]);
    ok(
        &tree,
        "git",
        &["commit", "--allow-empty", "-m", "task change"],
    );
    let final_head = ok(&tree, "git", &["rev-parse", "HEAD"]);
    fs::write(ws.join("tasks/feature/demo/notes.md"), "local brief").unwrap();
    ok(&ws, "git", &["add", "."]);
    assert_eq!(
        ok(&ws, "git", &["ls-files"]).lines().collect::<Vec<_>>(),
        [
            ".gitignore",
            "AGENTS.md",
            "README.md",
            "repos/.gitignore",
            "task-skills/.gitkeep",
            "tasks/.gitignore",
            "wk.yaml"
        ]
    );
    ok(&ws, "git", &["commit", "-m", "workspace"]);
    let other = tmp.path().join("other");
    ok(tmp.path(), "git", &["clone", ws.to_str().unwrap(), "other"]);
    assert!(!other.join(".wk").exists());
    ok(&other, WK, &["sync"]);
    assert_eq!(active_index(&other)["tasks"], serde_json::json!({}));
    assert!(other.join("repos/api/.gitignore").is_file());
    assert!(ok(&other, "git", &["status", "--porcelain"]).is_empty());
    let active = ws.join(".wk/active.yaml");
    let saved = fs::read(&active).unwrap();
    fs::remove_file(&active).unwrap();
    assert!(!run(&ws, WK, &["sync"]).status.success());
    assert!(!active.exists());
    fs::write(active, saved).unwrap();
    ok(&ws, WK, &["close", "feature/demo"]);
    let history = fs::read_dir(ws.join(".wk/history"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let archive: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(history).unwrap()).unwrap();
    assert_eq!(archive["branch"], "feature/demo");
    assert_eq!(archive["projects"]["api"]["commit"], start.trim());
    assert_eq!(
        archive["projects"]["api"]["final_commit"],
        final_head.trim()
    );
    assert!(ok(&ws, "git", &["status", "--porcelain"]).is_empty());
}

#[test]
fn reference_files_follow_the_configured_base_only_on_explicit_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let remote = source_repo(root, "upstream");
    ok(&remote, "git", &["switch", "-c", "release"]);
    fs::write(remote.join("version"), "one").unwrap();
    ok(&remote, "git", &["add", "version"]);
    ok(&remote, "git", &["commit", "-m", "release one"]);
    ok(&remote, "git", &["tag", "v1"]);
    ok(&remote, "git", &["switch", "trunk"]);
    ok(
        root,
        WK,
        &[
            "init",
            "space",
            "--yes",
            "--url",
            "./upstream",
            "--base",
            "origin/release",
        ],
    );
    let ws = root.join("space");
    let repo = ws.join("repos/upstream");
    ok(
        &ws,
        WK,
        &["add", remote.to_str().unwrap(), "pinned", "--base", "v1"],
    );
    assert!(
        ok(&repo, "git", &["branch", "--show-current"])
            .trim()
            .is_empty()
    );
    assert_eq!(fs::read_to_string(repo.join("version")).unwrap(), "one");
    ok(&remote, "git", &["switch", "release"]);
    fs::write(remote.join("version"), "two").unwrap();
    ok(&remote, "git", &["commit", "-am", "release two"]);
    ok(&ws, WK, &["status", "--fetch"]);
    ok(&ws, WK, &["start", "test", "upstream"]);
    assert_eq!(
        fs::read_to_string(ws.join("tasks/test/upstream/version")).unwrap(),
        "two"
    );
    assert_eq!(fs::read_to_string(repo.join("version")).unwrap(), "one");
    fs::write(repo.join("version"), "local").unwrap();
    assert!(!run(&ws, WK, &["sync"]).status.success());
    assert_eq!(fs::read_to_string(repo.join("version")).unwrap(), "local");
    ok(&repo, "git", &["restore", "version"]);
    ok(&ws, WK, &["sync"]);
    assert_eq!(fs::read_to_string(repo.join("version")).unwrap(), "two");
    assert_eq!(
        fs::read_to_string(ws.join("repos/pinned/version")).unwrap(),
        "one"
    );
    let missing = run(
        &ws,
        WK,
        &[
            "add",
            remote.to_str().unwrap(),
            "invalid",
            "--base",
            "origin/absent",
        ],
    );
    assert!(!missing.status.success() && !ws.join("repos/invalid").exists());
}

#[cfg(windows)]
#[test]
fn windows_refuses_device_names_and_launches_a_cmd_tool_in_the_task_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let source = tmp.path().join("api");
    for name in ["con", "NuL.txt", "COM9", "lpt1.foo.bar", "AUX", "PRN"] {
        for args in [
            vec!["add", source.to_str().unwrap(), name],
            vec!["start", name, "api"],
        ] {
            let out = run(&ws, WK, &args);
            assert!(
                !out.status.success()
                    && String::from_utf8_lossy(&out.stderr).contains("Windows reserves")
            );
        }
    }
    assert!(
        !run(&ws, WK, &["start", "group/nul", "api"])
            .status
            .success()
    );
    ok(&ws, WK, &["start", "COM10", "api"]);
    ok(&ws, WK, &["config", "--tools", "vscode"]);
    let bin = tmp.path().join("launcher dir");
    fs::create_dir(&bin).unwrap();
    fs::write(bin.join("code.cmd"), "@echo off\r\nif not exist api\\.git exit /b 3\r\nif not \"%~1\"==\".\" exit /b 4\r\nexit /b 17\r\n").unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let out = Command::new(WK)
        .args(["run", "COM10", "--agent", "vscode"])
        .current_dir(&ws)
        .env_remove("WK_WORKSPACE")
        .env("PATH", path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(17), "{out:?}");
    ok(&ws, WK, &["close", "COM10"]);
}

#[test]
fn init_rejects_nonempty_targets_before_prompting_or_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let edited = "version: 1\nprojects:\n  client:\n    name: client\n    url: 'git@example.org:team/client.git'\n    base: origin/trunk\n";
    fs::write(dir.join("wk.yaml"), edited).unwrap();
    let result = run(dir, WK, &["init", "--yes", "--url", "unused"]);
    assert!(!result.status.success());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("not empty")
    );
    assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    let status = ok(dir, WK, &["status", "--json"]);
    let status: serde_json::Value = serde_json::from_str(&status).unwrap();
    let client = &status["projects"]["client"];
    assert_eq!(client["url"], "git@example.org:team/client.git");
    assert_eq!(client["base"], "origin/trunk");
    assert!(client["commit"].is_null());
    assert!(
        client["error"]
            .as_str()
            .unwrap()
            .contains("missing clone client")
    );
    assert_eq!(fs::read_to_string(dir.join("wk.yaml")).unwrap(), edited);
    let hidden = dir.join("hidden");
    fs::create_dir(&hidden).unwrap();
    fs::write(hidden.join(".keep"), "keep").unwrap();
    assert!(!run(&hidden, WK, &["init"]).status.success());
    assert_eq!(fs::read_dir(&hidden).unwrap().count(), 1);
    let ambiguous = run(
        dir,
        WK,
        &["init", "missing/../hidden", "--yes", "--url", "unused"],
    );
    assert!(
        String::from_utf8(ambiguous.stderr)
            .unwrap()
            .contains("not empty")
    );
    assert!(!dir.join("missing").exists());
    for args in [
        &["init", "new"][..],
        &["init", "new", "--url", "unused"],
        &["init", "new", "--yes", "--base", "origin/trunk"],
    ] {
        assert!(!run(dir, WK, args).status.success());
        assert!(!dir.join("new").exists());
    }
    let failure = run(dir, WK, &["init", "new", "--yes", "--url", "./missing"]);
    assert!(!failure.status.success());
    assert!(!dir.join("new").exists());
}

#[test]
fn start_and_run_explain_missing_prerequisites() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    ok(tmp.path(), WK, &["init", "workspace", "--yes"]);
    let hint = |args: &[&str], expected: &str| {
        let out = run(&ws, WK, args);
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let message = String::from_utf8(out.stderr).unwrap();
        assert!(message.contains(expected), "{args:?}: {message}");
    };
    for args in [
        &["start"][..],
        &["start", "demo"],
        &["start", "demo", "api"],
    ] {
        hint(args, "wk add <url>");
    }
    let repo = source_repo(tmp.path(), "api");
    ok(&ws, WK, &["add", repo.to_str().unwrap()]);
    for args in [&["run"][..], &["run", "demo", "--codex"]] {
        hint(args, "wk start");
    }
    ok(&ws, WK, &["start", "demo", "api", "-D", "--no-fetch"]);
    hint(&["run"], "wk run <task>");
    hint(&["run", "missing"], "no open task missing");
    ok(&ws, WK, &["close", "demo"]);
    hint(&["run", "demo"], "wk start");
}

#[test]
fn init_without_a_project_is_ready_for_a_later_add() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ws = root.join("empty workspace");
    let output = ok(root, WK, &["init", "empty workspace", "--yes"]);
    assert!(!output.contains("Registered"));
    let manifest: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(ws.join("wk.yaml")).unwrap()).unwrap();
    assert_eq!(manifest["projects"], serde_json::json!({}));
    assert_eq!(manifest["skills"], serde_json::json!([]));
    assert!(!ws.join(".agents").exists());
    let diagnostics = ok(&ws, WK, &["doctor"]);
    assert!(diagnostics.contains("PASS  Project inventory  0 registered"));
    assert!(diagnostics.contains("PASS  Task inventory  0 active"));
    assert!(!diagnostics.contains("Worktree") && !diagnostics.contains("Ignored files"));
    assert!(diagnostics.contains("0 failed,"));
    let repo = source_repo(root, "api");
    ok(&ws, WK, &["add", repo.to_str().unwrap()]);
    ok(&ws, WK, &["start", "first", "api", "--no-fetch"]);
    assert!(ws.join("tasks/first/api/.git").is_file());
}

#[test]
fn init_installs_selected_templates_and_sync_preserves_workspace_edits() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let name = "wk-cli";
    let skill_file = format!(".agents/skills/{name}/SKILL.md");
    let bad = run(
        dir,
        WK,
        &["init", "invalid", "--yes", "--skills", "../unknown"],
    );
    assert!(!bad.status.success() && !dir.join("invalid").exists());
    ok(dir, WK, &["init", "workspace", "--yes", "--skills", name]);
    let ws = dir.join("workspace");
    let config: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(ws.join("wk.yaml")).unwrap()).unwrap();
    assert_eq!(config["skills"], serde_json::json!([name]));
    assert!(!ws.join("README.zh-CN.md").exists());
    assert!(!ws.join(".agents/skills/wk-workspace-workflow").exists());
    let skill = ws.join(&skill_file);
    let original = fs::read(&skill).unwrap();
    assert_eq!(
        original,
        include_bytes!("../templates/workspace/.agents/skills/wk-cli/SKILL.md")
    );
    ok(&ws, "git", &["add", "."]);
    ok(&ws, "git", &["ls-files", "--error-unmatch", &skill_file]);
    for path in ["README.md", "AGENTS.md", &skill_file] {
        fs::write(ws.join(path), "workspace edit").unwrap();
    }
    ok(&ws, WK, &["sync"]);
    for path in ["README.md", "AGENTS.md", &skill_file] {
        assert_eq!(fs::read_to_string(ws.join(path)).unwrap(), "workspace edit");
    }
    fs::remove_file(&skill).unwrap();
    ok(&ws, WK, &["sync"]);
    assert_eq!(fs::read(&skill).unwrap(), original);
    #[cfg(unix)]
    {
        fs::rename(ws.join(".agents"), dir.join("saved")).unwrap();
        std::os::unix::fs::symlink(dir, ws.join(".agents")).unwrap();
        assert!(!run(&ws, WK, &["sync"]).status.success());
        assert!(!run(&ws, WK, &["skills", "sync"]).status.success());
        assert!(!dir.join("skills").exists());
    }
}

#[test]
fn official_skills_are_checked_and_restored_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path();
    ok(
        ws,
        WK,
        &["init", "--yes", "--skills", "wk-cli,wk-workspace-workflow"],
    );
    let skills = ws.join(".agents/skills");
    let cli = skills.join("wk-cli/SKILL.md");
    let workflow = skills.join("wk-workspace-workflow/SKILL.md");
    let custom = skills.join("team");
    fs::create_dir(&custom).unwrap();
    fs::write(custom.join("SKILL.md"), "team").unwrap();
    fs::write(ws.join("AGENTS.md"), "workspace conventions").unwrap();
    let original = fs::read_to_string(&cli).unwrap();
    fs::write(&cli, original.replace("\r\n", "\n").replace('\n', "\r\n")).unwrap();
    assert!(ok(ws, WK, &["doctor"]).contains("PASS  Official skills"));
    fs::write(&cli, "local edit").unwrap();
    fs::write(skills.join("wk-cli/extra"), "obsolete").unwrap();
    fs::remove_file(&workflow).unwrap();
    let out = run(ws, WK, &["doctor"]);
    assert!(!out.status.success());
    let details = String::from_utf8(out.stdout).unwrap().replace('\\', "/");
    for finding in [
        "FAIL  Official skills",
        "modified wk-cli/SKILL.md",
        "unexpected wk-cli/extra",
        "missing wk-workspace-workflow/SKILL.md",
        "wk skills sync",
    ] {
        assert!(details.contains(finding), "{details}");
    }
    assert_eq!(fs::read_to_string(&cli).unwrap(), "local edit");
    assert!(!run(ws, WK, &["skills", "sync", "team"]).status.success());
    ok(ws, WK, &["skills", "sync", "wk-cli"]);
    assert_eq!(fs::read_to_string(&cli).unwrap(), original);
    assert!(!skills.join("wk-cli/extra").exists());
    assert!(!workflow.exists());
    #[cfg(unix)]
    {
        // A replaced skill or an extra nested link must never redirect writes or cleanup.
        fs::remove_dir(workflow.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&custom, workflow.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&custom, skills.join("wk-cli/linked")).unwrap();
    }
    ok(ws, WK, &["skills", "sync"]);
    ok(ws, WK, &["doctor"]);
    assert_eq!(fs::read_to_string(custom.join("SKILL.md")).unwrap(), "team");
    assert_eq!(
        fs::read_to_string(ws.join("AGENTS.md")).unwrap(),
        "workspace conventions"
    );
    assert_eq!(
        fs::read_to_string(workflow).unwrap(),
        include_str!("../templates/workspace/.agents/skills/wk-workspace-workflow/SKILL.md")
    );
}

#[test]
fn task_docs_snapshot_member_roles_and_bases_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let api = source_repo(root, "api");
    let client = source_repo(root, "client");
    ok(
        root,
        WK,
        &[
            "init",
            "space",
            "--yes",
            "--url",
            api.to_str().unwrap(),
            "--description",
            "Serves the client",
        ],
    );
    let ws = root.join("space");
    ok(&client, "git", &["branch", "release"]);
    ok(
        &ws,
        WK,
        &[
            "add",
            client.to_str().unwrap(),
            "--base",
            "origin/release",
            "--description",
            "Calls the API",
        ],
    );
    assert!(
        !run(&ws, WK, &["add", client.to_str().unwrap(), "ReadMe.md"])
            .status
            .success()
    );
    ok(
        &ws,
        WK,
        &["start", "feature/demo", "api", "client", "--no-fetch"],
    );
    let task = ws.join("tasks/feature/demo");
    let sha = ok(&client, "git", &["rev-parse", "release"]);
    for file in ["README.md", "AGENTS.md"] {
        let guide = fs::read_to_string(task.join(file)).unwrap();
        for expected in [
            "api/",
            "Serves the client",
            "client/",
            "Calls the API",
            "origin/trunk",
            "origin/release",
            sha.trim(),
        ] {
            assert!(guide.contains(expected), "{guide}");
        }
    }
    ok(
        &ws,
        WK,
        &[
            "config",
            "--project",
            "api",
            "--description",
            "Updated role",
        ],
    );
    ok(
        &ws,
        WK,
        &["config", "--project", "client", "--description", ""],
    );
    let overview = ok(&ws, WK, &["status"]);
    assert!(overview.contains("Updated role") && !overview.contains("Calls the API"));
    let mut index = active_index(&ws);
    index["tasks"]["feature/demo"]["phase"] = "creating".into();
    fs::write(
        ws.join(".wk/active.yaml"),
        serde_saphyr::to_string(&index).unwrap(),
    )
    .unwrap();
    fs::write(task.join("AGENTS.md"), "task conventions").unwrap();
    fs::remove_file(task.join("README.md")).unwrap();
    ok(&ws, WK, &["start", "feature/demo"]);
    assert_eq!(
        fs::read_to_string(task.join("AGENTS.md")).unwrap(),
        "task conventions"
    );
    let recovered = fs::read_to_string(task.join("README.md")).unwrap();
    assert!(recovered.contains("Calls the API") && recovered.contains("Serves the client"));
    ok(&ws, WK, &["close", "feature/demo"]);
    assert!(!task.exists());
}

#[test]
fn default_workspace_is_isolated_and_overridable() {
    let tmp = tempfile::tempdir().unwrap();
    let source = source_repo(tmp.path(), "source");
    let url = source.to_str().unwrap();
    let outer = tmp.path().join("outer");
    fs::create_dir(&outer).unwrap();
    let root = outer.as_path();
    let dev = root.join(".wk-dev");
    let wk = |args: &[&str]| {
        Command::new(WK)
            .current_dir(root)
            .env("WK_WORKSPACE", &dev)
            .args(args)
            .output()
            .unwrap()
    };
    ok(root, WK, &["init", "--yes", "--url", url]);
    assert!(
        wk(&["init", "other", "--yes", "--url", url])
            .status
            .success()
    );
    assert!(wk(&["-w", "other", "doctor"]).status.success());
    assert!(!dev.exists());

    // An uninitialized dev directory must never select an enclosing workspace.
    fs::create_dir(&dev).unwrap();
    assert!(!wk(&["doctor"]).status.success());
    assert!(wk(&["init", "--yes", "--url", url]).status.success());
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
    let repo = source_repo(outside, "api");
    ok(
        outside,
        WK,
        &["-w", dir, "init", "--yes", "--url", repo.to_str().unwrap()],
    );
    ok(outside, WK, &["doctor", "--workspace", dir]);
    assert!(outside.join(dir).join("wk.yaml").is_file());
    assert!(!outside.join("wk.yaml").exists());
    let commands = ok(outside, WK, &["-w", dir, "start", "demo", "api"]);
    assert!(commands.contains("--workspace") && commands.contains(" config"));
}

#[test]
#[cfg(unix)]
fn run_launches_only_enabled_tools_from_the_task_root() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let repo = source_repo(root, "api");
    let config_dir = "team's workspace";
    let ws = root.join(config_dir);
    ok(
        root,
        WK,
        &[
            "init",
            config_dir,
            "--yes",
            "--url",
            repo.to_str().unwrap(),
            "--tools",
            "codex,agy",
        ],
    );
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    symlink(
        ok(root, "sh", &["-c", "command -v git"]).trim(),
        bin.join("git"),
    )
    .unwrap();
    let stub = |name: &str| {
        let file = bin.join(name);
        fs::write(
            &file,
            "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"${0##*/}\" \"$#\" \"$*\"\nexit 17\n",
        )
        .unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    };
    for name in ["codex", "claude", "code", "zed"] {
        stub(name);
    }
    let invoke = |args: &[&str]| {
        Command::new(WK)
            .current_dir(root)
            .env_remove("WK_WORKSPACE")
            .env("PATH", &bin)
            .args(["-w", config_dir])
            .args(args)
            .output()
            .unwrap()
    };
    let name = "feature/login";
    let start = invoke(&["start", name, "api", "--no-fetch"]);
    assert!(
        start.status.success(),
        "{}",
        String::from_utf8_lossy(&start.stderr)
    );
    let hints = String::from_utf8(start.stdout).unwrap();
    let hint = hints
        .lines()
        .find(|l| l.ends_with("run feature/login --agent codex"))
        .unwrap();
    assert!(!hints.contains("--agent agy") && !hints.contains("--agent claude"));
    let from_hint = Command::new("/bin/sh")
        .current_dir(root)
        .env_remove("WK_WORKSPACE")
        .env("PATH", &bin)
        .args(["-c", hint])
        .output()
        .unwrap();
    assert_eq!(from_hint.status.code(), Some(17));

    for (args, error) in [
        (vec!["run", name, "--claude"], "not enabled"),
        (vec!["run", name, "--agent", "agy"], "not found on PATH"),
        (vec!["run", name], "specify --agent"),
        (
            vec!["run", name, "--codex", "--agent", "claude"],
            "cannot be used with",
        ),
        (
            vec!["run", name, "--codex", "--claude"],
            "cannot be used with",
        ),
        (vec!["config", "--tools", "shell"], "invalid value"),
    ] {
        let out = invoke(&args);
        assert!(
            !out.status.success() && String::from_utf8_lossy(&out.stderr).contains(error),
            "{args:?} {out:?}"
        );
    }
    stub("agy");
    ok(
        &ws,
        WK,
        &["config", "--tools", "codex,claude,agy,vscode,zed"],
    );
    let expected_dir = ws.join("tasks").join(name).canonicalize().unwrap();
    for (selection, exe, arguments) in [
        (vec!["--codex"], "codex", "0\n"),
        (vec!["--claude"], "claude", "0\n"),
        (vec!["--agent", "agy"], "agy", "0\n"),
        (vec!["--agent", "vscode"], "code", "1\n."),
        (vec!["--agent", "zed"], "zed", "1\n."),
    ] {
        let mut args = vec!["run", name];
        args.extend(selection);
        let out = invoke(&args);
        assert_eq!(out.status.code(), Some(17), "{out:?}");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            format!("{}\n{exe}\n{arguments}\n", expected_dir.display())
        );
    }
    ok(&ws, WK, &["config", "--tools"]);
    assert!(String::from_utf8_lossy(&invoke(&["run", name]).stderr).contains("no tools enabled"));
    assert!(
        String::from_utf8_lossy(&invoke(&["run", name, "--codex"]).stderr).contains("not enabled")
    );
}

fn task_skill(ws: &Path, id: &str) -> std::path::PathBuf {
    let dir = ws.join("task-skills").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!(
            "---\nname: {id}\ndescription: Test {id} workflows\n---\nUse the bundled resources.\n"
        ),
    )
    .unwrap();
    dir
}

fn set_presets(ws: &Path, presets: serde_json::Value) {
    let path = ws.join("wk.yaml");
    let mut manifest: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    manifest["presets"] = presets;
    fs::write(path, serde_saphyr::to_string(&manifest).unwrap()).unwrap();
}

#[test]
fn preset_and_skill_snapshot_survive_an_interrupted_start() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let skill = task_skill(&ws, "contract");
    let bytes = [0, 255, 42];
    fs::create_dir(skill.join("assets")).unwrap();
    fs::write(skill.join("assets/spec.bin"), bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            skill.join("assets/spec.bin"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    let ops = ws.join("repos/ops");
    let api = ws.join("repos/api");
    ok(&api, "git", &["tag", "v1"]);
    ok(&ops, "git", &["branch", "fix"]);
    set_presets(
        &ws,
        serde_json::json!({"pair":{"description":"Paired change","projects":{"api":{"base":"v1"},"ops":{}},"skills":["contract"]}}),
    );
    assert!(ok(&ws, WK, &["preset", "list"]).contains("Paired change"));
    assert!(ok(&ws, WK, &["preset", "show", "pair"]).contains("v1"));
    assert!(ok(&ws, WK, &["doctor"]).contains("PASS  Task presets"));
    let args = ["start", "fix/pair", "--preset", "pair", "--no-fetch"];
    assert!(!run(&ws, WK, &args).status.success());
    let tree = ws.join("tasks/fix/pair");
    assert!(tree.join("api/.git").is_file());
    let idx = active_index(&ws);
    let rec = &idx["tasks"]["fix/pair"];
    assert_eq!(rec["phase"], "creating");
    assert_eq!(rec["projects"]["api"]["base"], "v1");
    assert_eq!(rec["projects"]["ops"]["base"], "origin/trunk");
    // Reproduce interruption between recording the snapshot and publishing it.
    let pending = ws.join(".wk").join(format!(
        "task-skills-{}-fix%2Fpair",
        rec["started_at_unix_ms"]
    ));
    fs::rename(tree.join(".agents/skills"), pending).unwrap();
    ok(&ws, WK, &["preset", "rm", "pair"]);
    fs::remove_dir_all(&skill).unwrap();
    ok(&ops, "git", &["branch", "-m", "fix", "other"]);
    ok(&ws, WK, &args);
    let asset = tree.join(".agents/skills/contract/assets/spec.bin");
    assert_eq!(fs::read(&asset).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(asset.metadata().unwrap().permissions().mode() & 0o111, 0);
    }
    let status: serde_json::Value =
        serde_json::from_str(&ok(&ws, WK, &["status", "fix/pair", "--json"])).unwrap();
    let task = &status["tasks"]["fix/pair"];
    assert_eq!(task["preset"], "pair");
    assert_eq!(task["skills"], serde_json::json!(["contract"]));
    assert!(
        fs::read_to_string(tree.join("AGENTS.md"))
            .unwrap()
            .contains(".agents/skills/contract/SKILL.md")
    );
    fs::write(&asset, b"task edit").unwrap();
    ok(&ws, WK, &["start", "fix/pair"]);
    assert_eq!(fs::read(&asset).unwrap(), b"task edit");
    ok(&ws, WK, &["close", "fix/pair"]);
    assert!(!tree.exists());
    let archive = fs::read_dir(ws.join(".wk/history"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let archived: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(archive).unwrap()).unwrap();
    assert_eq!(archived["skills"], task["skills"]);
    assert_eq!(archived["preset"], task["preset"]);
}

#[test]
fn task_skills_are_scoped_and_bad_selections_fail_before_creating_worktrees() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = fixture(tmp.path());
    let first = task_skill(&ws, "one");
    task_skill(&ws, "two");
    let team = ws.join(".agents/skills/admin");
    fs::create_dir_all(&team).unwrap();
    fs::write(team.join("SKILL.md"), "workspace only").unwrap();
    set_presets(
        &ws,
        serde_json::json!({
            "gone":{"projects":{"api":{}},"skills":["three"]},
            "typo":{"projects":{"absent":{}}},
            "ref":{"projects":{"api":{"base":"origin/missing"}}}
        }),
    );
    let report = run(&ws, WK, &["doctor"]);
    assert!(!report.status.success());
    let report = String::from_utf8_lossy(&report.stdout);
    for label in [
        "FAIL  Task presets",
        "gone",
        "three",
        "typo",
        "absent",
        "ref: api (origin/missing)",
    ] {
        assert!(report.contains(label), "{report}");
    }
    for (args, error) in [
        (vec!["--preset", "gone"], "three"),
        (vec!["--preset", "typo"], "absent"),
        (vec!["--preset", "ref"], "origin/missing"),
        (vec!["--preset", "unknown"], "unknown preset"),
        (vec!["api", "--skills", "../outside"], "invalid name"),
        (vec!["api", "--skills", "admin"], "admin"),
        (vec!["--preset", "gone", "api"], "cannot be used with"),
        (
            vec!["--preset", "gone", "--skills", "two"],
            "cannot be used with",
        ),
    ] {
        let mut start = vec!["start", "demo", "--no-fetch"];
        start.extend(args);
        let out = run(&ws, WK, &start);
        let message = String::from_utf8_lossy(&out.stderr);
        assert!(
            !out.status.success() && message.contains(error),
            "{start:?}: {message}"
        );
        assert!(active_index(&ws)["tasks"].as_object().unwrap().is_empty());
        assert!(!ws.join("tasks/demo").exists());
        assert!(ok(&ws.join("repos/api"), "git", &["branch", "--list", "demo"]).is_empty());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&team, first.join("external")).unwrap();
        assert!(
            !run(
                &ws,
                WK,
                &[
                    "start",
                    "demo",
                    "api",
                    "-D",
                    "--skills",
                    "one",
                    "--no-fetch"
                ]
            )
            .status
            .success()
        );
        assert!(!ws.join("tasks/demo").exists());
        fs::remove_file(first.join("external")).unwrap();
    }
    ok(
        &ws,
        WK,
        &[
            "start",
            "demo",
            "api",
            "-D",
            "--skills",
            "one,two",
            "--no-fetch",
        ],
    );
    let installed = ws.join("tasks/demo/.agents/skills");
    for id in ["one", "two"] {
        assert!(installed.join(id).join("SKILL.md").is_file());
    }
    assert!(!installed.join("admin").exists());
    ok(&ws, WK, &["close", "demo"]);
    assert!(first.join("SKILL.md").is_file());
}
