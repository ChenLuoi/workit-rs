use super::{WORKSPACE_FILES, install_missing, skill_file};
use crate::workspace::{Workspace, child, is_link};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

fn files(name: &str) -> impl Iterator<Item = (&'static str, &'static [u8])> {
    WORKSPACE_FILES.iter().filter_map(move |&(path, bytes)| {
        let (id, file) = skill_file(path)?;
        (id == name).then_some((file, bytes))
    })
}

fn matches_bundle(actual: &[u8], expected: &[u8]) -> bool {
    if actual == expected {
        return true;
    }
    if expected.contains(&0) {
        return false;
    }
    match (std::str::from_utf8(actual), std::str::from_utf8(expected)) {
        (Ok(a), Ok(b)) => a.replace("\r\n", "\n") == b.replace("\r\n", "\n"),
        _ => false,
    }
}

fn skill_dir(ws: &Workspace, name: &str) -> Result<PathBuf> {
    Ok(child(&child(&ws.0, ".agents")?, "skills")?.join(name))
}

pub fn skill_differences(ws: &Workspace, name: &str) -> Result<Vec<String>> {
    let dir = skill_dir(ws, name)?;
    let mut expected = BTreeMap::new();
    for (file, bytes) in files(name) {
        let path = Path::new(file);
        for parent in path.ancestors().skip(1) {
            expected.insert(parent, None);
        }
        expected.insert(path, Some(bytes));
    }
    let mut pending = vec![PathBuf::new()];
    let mut differences = Vec::new();
    while let Some(relative) = pending.pop() {
        let path = if relative.as_os_str().is_empty() {
            dir.clone()
        } else {
            dir.join(&relative)
        };
        let meta = match path.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("inspect {}", path.display())),
        };
        let kind = match expected.remove(relative.as_path()) {
            None => "unexpected",
            Some(None) if !is_link(&meta) && meta.is_dir() => {
                for entry in fs::read_dir(&path)? {
                    pending.push(relative.join(entry?.file_name()));
                }
                continue;
            }
            Some(Some(bytes)) if !is_link(&meta) && meta.is_file() => {
                if matches_bundle(&fs::read(&path)?, bytes) {
                    continue;
                }
                "modified"
            }
            _ => "wrong file type",
        };
        differences.push(format!(
            "{kind} {}",
            Path::new(name).join(relative).display()
        ));
    }
    for (file, _) in expected.into_iter().filter(|(_, bytes)| bytes.is_some()) {
        differences.push(format!("missing {}", Path::new(name).join(file).display()));
    }
    differences.sort();
    Ok(differences)
}

pub fn sync_skills(ws: &Workspace, selected: Option<&str>) -> Result<()> {
    let _lock = ws.lock()?;
    let mut names = ws.config()?.skills;
    if let Some(name) = selected {
        ensure!(
            names.contains(name),
            "skill {name} is not registered in wk.yaml"
        );
        names.retain(|id| id == name);
    }
    if names.is_empty() {
        println!("No official skills registered.");
    }
    for name in names {
        if skill_differences(ws, &name)?.is_empty() {
            println!("{name} is up to date");
            continue;
        }
        let dir = skill_dir(ws, &name)?;
        fs::create_dir_all(dir.parent().unwrap())?;
        // Keep partially staged skills outside harness discovery and workspace Git.
        let stage = tempfile::Builder::new()
            .prefix(".wk-skills-")
            .tempdir_in(child(&ws.0, ".wk")?)?;
        let new = stage.path().join("new");
        fs::create_dir(&new)?;
        for (file, bytes) in files(&name) {
            install_missing(&new, file, bytes)?;
        }
        let old = stage.path().join("previous");
        let had_old = match fs::rename(&dir, &old) {
            Ok(()) => true,
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => return Err(e).with_context(|| format!("replace {name}")),
        };
        if let Err(error) = fs::rename(&new, &dir) {
            if had_old && let Err(restore) = fs::rename(&old, &dir) {
                let backup = stage.keep().join("previous");
                bail!(
                    "install {name}: {error}; restore failed ({restore}); saved at {}",
                    backup.display()
                );
            }
            return Err(error).with_context(|| format!("install {name}"));
        }
        stage
            .close()
            .with_context(|| format!("clean up previous contents of {name}"))?;
        println!("Restored {name}");
    }
    Ok(())
}
