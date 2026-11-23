use anyhow::{Context, Result, bail, ensure};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
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
