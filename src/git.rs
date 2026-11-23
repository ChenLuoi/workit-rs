use anyhow::{Context, Result, bail, ensure};
use indicatif::ProgressBar;
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::Duration,
};

pub fn path_arg(path: &Path) -> Result<&str> {
    path.to_str().context("workspace paths must be valid UTF-8")
}

// Git for Windows prints drive/UNC paths with forward slashes; Rust's
// canonicalize returns a verbatim Win32 path even for a short local path.
pub fn git_path(path: &Path) -> Result<String> {
    let value = path_arg(path)?;
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return Ok(format!("//{}", unc.replace('\\', "/")));
        }
        Ok(value.trim_start_matches(r"\\?\").replace('\\', "/"))
    }
    #[cfg(not(windows))]
    Ok(value.into())
}

fn invoke(dir: &Path, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    // Run against the named checkout even when wk is invoked by a Git hook.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        cmd.env_remove(key);
    }
    cmd.current_dir(dir)
        .args(args)
        .stdin(Stdio::inherit())
        .output()
        .with_context(|| format!("run git {} in {}", args[0], dir.display()))
}

pub fn bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = invoke(dir, args)?;
    ensure!(
        out.status.success(),
        "git {} in {}: {}",
        args[0],
        dir.display(),
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(out.stdout)
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8(bytes(dir, args)?)?
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

pub fn git_with_progress(dir: &Path, args: &[&str], message: String) -> Result<String> {
    if !io::stderr().is_terminal() {
        return git(dir, args);
    }
    let spinner = ProgressBar::new_spinner().with_message(message);
    if !spinner.is_hidden() {
        spinner.tick();
        spinner.enable_steady_tick(Duration::from_millis(100));
    }
    let result = git(dir, args);
    spinner.finish_and_clear();
    result
}

pub fn fetch_origin(repo: &Path, name: &str) -> Result<()> {
    git_with_progress(
        repo,
        &["fetch", "origin"],
        format!("Fetching {name} from origin"),
    )?;
    Ok(())
}

/// Short branch names with the advertised default first. Scripts only need HEAD.
pub fn remote_branches(dir: &Path, url: &str, include_choices: bool) -> Result<Vec<String>> {
    let mut args = vec!["ls-remote", "--symref", "--", url, "HEAD"];
    if include_choices {
        args.push("refs/heads/*");
    }
    let refs = git_with_progress(dir, &args, "Reading remote branches".into())?;
    let default = refs
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")?
                .strip_suffix("\tHEAD")
        })
        .context("remote has no advertised default branch; specify --base <ref>")?;
    let mut branches = vec![default.to_owned()];
    if include_choices {
        branches.extend(refs.lines().filter_map(|line| {
            let (_, name) = line.split_once('\t')?;
            let branch = name.strip_prefix("refs/heads/")?;
            (branch != default).then(|| branch.to_owned())
        }));
        branches[1..].sort_unstable();
    }
    Ok(branches)
}

/// Snapshot direct origin branches and their fetched commits; omit symbolic origin/HEAD.
pub fn cached_origin_branches(repo: &Path) -> Result<BTreeMap<String, String>> {
    let refs = git(
        repo,
        &[
            "for-each-ref",
            "--format=%(symref)%09%(refname:lstrip=3)%09%(objectname)",
            "refs/remotes/origin/",
        ],
    )?;
    Ok(refs
        .lines()
        .filter_map(|row| {
            let (name, oid) = row.strip_prefix('\t')?.split_once('\t')?;
            Some((name.to_owned(), oid.to_owned()))
        })
        .collect())
}

pub fn branch_exists(repo: &Path, branch: &str) -> Result<bool> {
    let out = invoke(
        repo,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => bail!(
            "cannot check branch {branch}: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
    }
}

pub fn check_clone_removal(repo: &Path) -> Result<()> {
    let listing = bytes(repo, &["worktree", "list", "--porcelain", "-z"])?;
    let worktrees = listing
        .split(|b| *b == 0)
        .filter(|f| f.starts_with(b"worktree "))
        .count();
    ensure!(
        worktrees == 1,
        "{} has linked worktrees; inspect git worktree list before removal",
        repo.display()
    );
    // A clean status cannot account for private submodule repositories or files
    // deliberately hidden from the index's working-tree checks.
    ensure!(
        !repo.join(".git/modules").try_exists()?,
        "{} contains submodule repositories; preserve them before removal",
        repo.display()
    );
    let index = bytes(repo, &["ls-files", "-v", "--stage", "-z"])?;
    for entry in index.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
        let flag = entry[0];
        ensure!(
            flag != b'S' && !flag.is_ascii_lowercase(),
            "{} has skip-worktree or assume-unchanged files; restore normal index checks before removal",
            repo.display()
        );
        // A gitlink can hold an independent clone with no entry in .git/modules.
        if entry[1..].starts_with(b" 160000 ") {
            let (_, path) = std::str::from_utf8(entry)?
                .split_once('\t')
                .context("missing submodule path in the Git index")?;
            let submodule = repo.join(path);
            ensure!(
                !submodule.try_exists()? || submodule.read_dir()?.next().is_none(),
                "submodule {path} contains local data; preserve it before removal"
            );
        }
    }
    let commit = git(
        repo,
        &[
            "rev-list",
            "--max-count=1",
            "--all",
            "--reflog",
            "--not",
            "--remotes=origin",
        ],
    )?;
    ensure!(
        commit.is_empty(),
        "{} contains commit {commit} not reachable from a locally known origin branch; push or back up your work first",
        repo.display()
    );
    Ok(())
}

pub fn check_root(path: &Path) -> Result<()> {
    let top = PathBuf::from(git(path, &["rev-parse", "--show-toplevel"])?);
    ensure!(
        top.canonicalize()? == path,
        "{} is not a Git checkout root",
        path.display()
    );
    Ok(())
}

pub fn validate_tree(repo: &Path, tree: &Path, branch: &str) -> Result<()> {
    check_root(tree)?;
    let common = git(
        tree,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    ensure!(
        Path::new(&common).canonicalize()? == repo.join(".git"),
        "wrong repository at {}",
        tree.display()
    );
    let actual = git(tree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    ensure!(
        actual == branch,
        "{} is on {actual}, expected {branch}",
        tree.display()
    );
    Ok(())
}
