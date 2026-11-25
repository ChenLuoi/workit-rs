use crate::{prompt, tasks::Phase, workspace::Workspace};
use anyhow::{Context, Result, ensure};
use clap::ValueEnum;
use console::style;
use inquire::{MultiSelect, Select};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, env, fmt, path::PathBuf, process::Command};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Codex,
    Claude,
    Agy,
    Vscode,
    Zed,
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_possible_value().unwrap().get_name())
    }
}

impl Tool {
    fn commands(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &["codex"],
            Self::Claude => &["claude"],
            Self::Agy => &["agy"],
            Self::Vscode => &["code"],
            Self::Zed if cfg!(target_os = "linux") => &["zed", "zeditor"],
            Self::Zed => &["zed"],
        }
    }

    pub fn executable(self) -> Option<PathBuf> {
        let path = env::var_os("PATH")?;
        for name in self.commands() {
            for dir in env::split_paths(&path) {
                #[cfg(windows)]
                let candidates =
                    ["exe", "cmd", "bat"].map(|ext| dir.join(name).with_extension(ext));
                #[cfg(not(windows))]
                let candidates = [dir.join(name)];
                for candidate in candidates {
                    if !candidate.is_file() {
                        continue;
                    }
                    #[cfg(unix)]
                    if rustix::fs::access(&candidate, rustix::fs::Access::EXEC_OK).is_err() {
                        continue;
                    }
                    return std::path::absolute(candidate).ok();
                }
            }
        }
        None
    }
}

pub fn available_tools(enabled: &BTreeSet<Tool>) -> Vec<Tool> {
    enabled
        .iter()
        .copied()
        .filter(|t| t.executable().is_some())
        .collect()
}

pub fn select_tools(saved: Option<&BTreeSet<Tool>>) -> Result<BTreeSet<Tool>> {
    ensure!(
        prompt::interactive(),
        "use wk config --tools <list> outside a terminal"
    );
    eprintln!(
        "Choose the tools to enable for this workspace. You can change them later with wk config."
    );
    let tools = Tool::value_variants();
    let mut choices = Vec::new();
    let mut defaults = Vec::new();
    for (i, &tool) in tools.iter().enumerate() {
        let installed = tool.executable().is_some();
        let checked = match saved {
            Some(selection) => selection.contains(&tool),
            None => installed && matches!(tool, Tool::Codex | Tool::Claude | Tool::Agy),
        };
        if checked {
            defaults.push(i);
        }
        let label = if !installed {
            style(format!("{tool} (not installed)"))
                .for_stderr()
                .dim()
                .to_string()
        } else if tool == Tool::Vscode {
            format!("{tool} ({})", tool.commands()[0])
        } else {
            tool.to_string()
        };
        choices.push(label);
    }
    let selected = MultiSelect::new("Enable tools", choices)
        .with_default(&defaults)
        .with_help_message("↑/↓ move, Space toggles, Enter saves. An empty selection is allowed.")
        .raw_prompt()?;
    Ok(selected
        .into_iter()
        .map(|option| tools[option.index])
        .collect())
}

impl Workspace {
    pub fn run_tool(&self, task: Option<&str>, requested: Option<Tool>) -> Result<i32> {
        let index = self.active_index()?;
        ensure!(
            index
                .tasks
                .values()
                .any(|record| record.phase == Phase::Active),
            "no active tasks. Start a task first with wk start."
        );
        let name = task.context(
            "specify an active task with wk run <task>. List active tasks with wk status.",
        )?;
        let record = index.into_task(name)?;
        let dir = self.active_path(name, &record, None)?;
        let enabled = self.config()?.tools;
        let tool = match requested {
            Some(tool) => tool,
            None => {
                ensure!(!enabled.is_empty(), "no tools enabled; run wk config");
                let available = available_tools(&enabled);
                ensure!(
                    !available.is_empty(),
                    "none of the enabled tools are on PATH; check their installation or run wk config"
                );
                ensure!(
                    prompt::interactive(),
                    "specify --agent <tool> outside a terminal"
                );
                Select::new("Launch tool", available).prompt()?
            }
        };
        // Read again after the picker in case another terminal changed the settings.
        let enabled = if requested.is_none() {
            self.config()?.tools
        } else {
            enabled
        };
        ensure!(
            enabled.contains(&tool),
            "{tool} is not enabled in this workspace; run wk config"
        );
        let exe = tool.executable().with_context(|| {
            format!(
                "{tool} is enabled but {} was not found on PATH",
                tool.commands().join(" or ")
            )
        })?;
        let mut child = Command::new(exe);
        child.current_dir(dir);
        if matches!(tool, Tool::Vscode | Tool::Zed) {
            child.arg(".");
        }
        if env::var_os("WK_WORKSPACE").is_some() {
            child.env("WK_WORKSPACE", &self.0);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            Err(child.exec()).with_context(|| format!("launch {tool}"))
        }
        #[cfg(not(unix))]
        Ok(child
            .status()
            .with_context(|| format!("launch {tool}"))?
            .code()
            .unwrap_or(1))
    }
}
