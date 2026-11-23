mod git;
mod report;
mod tasks;
mod workspace;

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};
use std::{ffi::OsString, path::PathBuf, process::Command};
use workspace::Workspace;

#[derive(Parser)]
#[command(name = "wk", version, about)]
struct Cli {
    /// Workspace directory (default: WK_WORKSPACE, or discover from current directory).
    #[arg(long, short = 'w', global = true)]
    workspace: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Initialize a workspace in DIR (default: WK_WORKSPACE or current directory).
    Init { dir: Option<PathBuf> },
    /// Clone a project into repos/ and register it in wk.yaml.
    Add {
        url: String,
        #[arg(long)]
        name: Option<String>,
        /// Git ref used when creating a task (default: origin/HEAD).
        #[arg(long)]
        base: Option<String>,
    },
    /// Clone missing projects and fetch origin for existing projects.
    Sync { projects: Vec<String> },
    /// Create or resume a task and print launch commands.
    Start {
        task: String,
        #[arg(required = true)]
        projects: Vec<String>,
        #[arg(long)]
        no_fetch: bool,
    },
    /// Run an interactive program in the task directory.
    Run {
        task: String,
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<OsString>,
    },
    /// Print the absolute path to a task or one of its projects.
    Path {
        task: String,
        project: Option<String>,
    },
    /// Report project and task status from local Git data.
    Status {
        task: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        fetch: bool,
    },
    /// Diagnose workspace and Git inconsistencies.
    Doctor,
    /// Remove clean worktrees and archive the task, keeping its branches.
    Close { task: String },
}

fn execute() -> Result<i32> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let default_workspace = std::env::var_os("WK_WORKSPACE").map(PathBuf::from);
    if let Action::Init { dir } = cli.command {
        ensure!(
            dir.is_none() || cli.workspace.is_none(),
            "use either DIR or --workspace"
        );
        Workspace::init(&dir.or(cli.workspace).or(default_workspace).unwrap_or(cwd))?;
        return Ok(0);
    }
    let ws = if let Some(dir) = cli.workspace {
        Workspace::find(&dir)?
    } else if let Some(dir) = default_workspace {
        Workspace::at_root(&dir)?
    } else {
        Workspace::find(&cwd)?
    };
    match cli.command {
        Action::Init { .. } => unreachable!(),
        Action::Add { url, name, base } => ws.add(&url, name, base)?,
        Action::Sync { projects } => ws.sync(&projects)?,
        Action::Start {
            task,
            projects,
            no_fetch,
        } => ws.start(&task, &projects, no_fetch)?,
        Action::Path { task, project } => {
            println!("{}", ws.active_path(&task, project.as_deref())?.display())
        }
        Action::Run { task, command } => {
            let mut child = Command::new(&command[0]);
            if std::env::var_os("WK_WORKSPACE").is_some() {
                child.env("WK_WORKSPACE", &ws.0);
            }
            child
                .args(&command[1..])
                .current_dir(ws.active_path(&task, None)?);
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                return Err(child.exec()).with_context(|| format!("launch {:?}", command[0]));
            }
            #[cfg(not(unix))]
            return Ok(child
                .status()
                .context("launch task command")?
                .code()
                .unwrap_or(1));
        }
        Action::Status { task, json, fetch } => ws.status(task.as_deref(), json, fetch)?,
        Action::Doctor => ws.doctor()?,
        Action::Close { task } => ws.close(&task)?,
    }
    Ok(0)
}

fn main() {
    match execute() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("wk: {error:#}");
            std::process::exit(1);
        }
    }
}
