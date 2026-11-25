use crate::{
    templates::Skill,
    workspace::{Workspace, child, is_link, name_ok, publish_directory},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

fn skill_at(dir: &Path, id: &str) -> Result<Skill> {
    let file = child(dir, "SKILL.md")?;
    ensure!(file.is_file(), "missing regular file {}", file.display());
    Skill::read(id, &fs::read(&file)?).with_context(|| format!("read {}", file.display()))
}

impl Workspace {
    pub fn task_skill(&self, id: &str) -> Result<Skill> {
        name_ok(id)?;
        let dir = child(&child(&self.0, "task-skills")?, id)?;
        skill_at(&dir, id)
    }

    pub fn choose_task_skills(&self, selected: &BTreeSet<String>) -> Result<BTreeSet<String>> {
        let root = child(&self.0, "task-skills")?;
        let mut choices = Vec::new();
        if root.try_exists()? {
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("invalid task skill directory name"))?;
                choices.push(self.task_skill(&name)?);
            }
        }
        if choices.is_empty() {
            return Ok(BTreeSet::new());
        }
        choices.sort_by(|a, b| a.name.cmp(&b.name));
        let defaults = choices
            .iter()
            .enumerate()
            .filter_map(|(i, s)| selected.contains(&s.name).then_some(i))
            .collect::<Vec<_>>();
        crate::templates::pick_skills("Select task skills", choices, &defaults)
    }

    pub fn pending_skills(&self, task: &str, started: u64) -> Result<PathBuf> {
        child(
            &child(&self.0, ".wk")?,
            &format!("task-skills-{started}-{}", task.replace('/', "%2F")),
        )
    }

    pub fn stage_skills(&self, selected: &BTreeSet<String>, pending: &Path) -> Result<()> {
        let temp = tempfile::tempdir_in(child(&self.0, ".wk")?)?;
        let root = child(&self.0, "task-skills")?;
        for id in selected {
            name_ok(id)?;
            let source = child(&root, id)?;
            let target = temp.path().join(id);
            copy_tree(&source, &target).with_context(|| format!("snapshot task skill {id}"))?;
            skill_at(&target, id)?;
        }
        publish_directory(temp.path(), pending)
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = source
        .symlink_metadata()
        .with_context(|| format!("inspect {}", source.display()))?;
    ensure!(!is_link(&metadata), "refusing link {}", source.display());
    if metadata.is_file() {
        fs::copy(source, target)?;
    } else {
        ensure!(
            metadata.is_dir(),
            "expected regular file or directory at {}",
            source.display()
        );
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

// Publish the entire set at once. After a crash exactly one of the pending
// snapshot and the installed directory is present; retry never reads the catalog.
pub fn install_snapshot(dir: &Path, pending: &Path, selected: &BTreeSet<String>) -> Result<()> {
    let agents = child(dir, ".agents")?;
    fs::create_dir_all(&agents)?;
    let target = child(&agents, "skills")?;
    if pending.try_exists()? {
        publish_directory(pending, &target)?;
    }
    ensure!(
        target.is_dir(),
        "missing recorded task skill snapshot for {}",
        dir.display()
    );
    for id in selected {
        ensure!(
            child(&target, id)?.is_dir(),
            "missing recorded task skill {id}"
        );
    }
    Ok(())
}

pub fn discard_pending(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
