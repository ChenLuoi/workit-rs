use crate::{
    tasks::{Member, Phase, Task},
    workspace::{Workspace, child, task_name_ok},
};
use anyhow::{Context, Result, bail, ensure};
use chrono::{Local, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const LOCAL_TIME: &str = "%Y—%m—%d—%H—%M—%S";

fn local_time(ms: i64) -> Result<String> {
    Ok(Local
        .timestamp_millis_opt(ms)
        .single()
        .context("invalid task timestamp")?
        .format(LOCAL_TIME)
        .to_string())
}

// The destination and the confirmed findings survive interruption and timezone changes.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClosingHistory {
    started_at: String,
    closed_at: String,
    occurrence: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    damage: BTreeSet<String>,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Archive {
    task: String,
    preset: Option<String>,
    #[serde(default)]
    skills: BTreeSet<String>,
    branch: String,
    started_at: String,
    closed_at: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovered: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    damage: BTreeSet<String>,
    projects: BTreeMap<String, Member>,
}

impl Archive {
    fn for_task(name: &str, task: &Task, history: &ClosingHistory) -> Self {
        Self {
            task: name.to_owned(),
            branch: name.to_owned(),
            preset: task.preset.clone(),
            skills: task.skills.clone(),
            projects: task.projects.clone(),
            started_at: history.started_at.clone(),
            closed_at: history.closed_at.clone(),
            recovered: task.phase == Phase::Recovering,
            damage: history.damage.clone(),
        }
    }

    fn read(path: &Path) -> Result<Self> {
        serde_saphyr::from_str(&fs::read_to_string(path)?)
            .with_context(|| format!("read archive {}", path.display()))
    }

    fn path(&self, dir: &Path, occurrence: Option<u32>) -> Result<PathBuf> {
        // Pending close records are editable YAML. Validate before using a timestamp in a path.
        NaiveDateTime::parse_from_str(&self.started_at, LOCAL_TIME)
            .context("invalid archive start time")?;
        task_name_ok(&self.task)?;
        let suffix = occurrence.map_or_else(String::new, |n| format!("—{n}"));
        child(
            dir,
            &format!(
                "{}—{}{suffix}.yaml",
                self.started_at,
                self.task.replace('/', "%2F")
            ),
        )
    }

    fn choose_occurrence(&self, dir: &Path, resuming: bool) -> Result<Option<u32>> {
        for n in 1..=u32::MAX {
            let occurrence = (n > 1).then_some(n);
            let path = self.path(dir, occurrence)?;
            if !path.try_exists()? {
                return Ok(occurrence);
            }
            let existing = Self::read(&path)?;
            ensure!(
                existing.task == self.task && existing.started_at == self.started_at,
                "archive identity differs from filename: {}",
                path.display()
            );
            if resuming && existing == *self {
                return Ok(occurrence);
            }
        }
        bail!("too many archives for {} at {}", self.task, self.started_at)
    }
}

impl Workspace {
    fn history_dir(&self) -> Result<PathBuf> {
        child(&child(&self.0, ".wk")?, "history")
    }

    pub(crate) fn prepare_history(
        &self,
        name: &str,
        task: &mut Task,
        closed: u64,
        damage: Vec<String>,
        resuming: bool,
    ) -> Result<()> {
        if let Some(history) = &mut task.history {
            history.damage.extend(damage);
            return Ok(());
        }
        let dir = self.history_dir()?;
        let mut history = ClosingHistory {
            started_at: local_time(task.started_at_unix_ms.try_into()?)?,
            closed_at: local_time(closed.try_into()?)?,
            occurrence: None,
            damage: damage.into_iter().collect(),
        };
        let archive = Archive::for_task(name, task, &history);
        history.occurrence = archive.choose_occurrence(&dir, resuming)?;
        task.history = Some(history);
        Ok(())
    }

    pub(crate) fn archive_task(&self, name: &str, task: &Task) -> Result<()> {
        let history = task.history.as_ref().context("missing closing history")?;
        let archive = Archive::for_task(name, task, history);
        let dir = self.history_dir()?;
        fs::create_dir_all(&dir)?;
        let path = archive.path(&dir, history.occurrence)?;
        if path.try_exists()? {
            ensure!(
                Archive::read(&path)? == archive,
                "existing archive differs from the closing task: {}",
                path.display()
            );
        } else {
            let mut temp = tempfile::NamedTempFile::new_in(dir)?;
            temp.write_all(serde_saphyr::to_string(&archive)?.as_bytes())?;
            temp.as_file().sync_all()?;
            temp.persist_noclobber(path)?;
        }
        Ok(())
    }
}
