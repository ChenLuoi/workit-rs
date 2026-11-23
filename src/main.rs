mod git;
mod prompt;
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
    /// Clone into a new repos/<directory> and register the project in wk.yaml.
    ///
    /// With only a URL, enter a directory and choose a remote branch by prefix.
    /// Press Enter twice to use the repository name and remote default branch.
    /// Scripts use these defaults automatically. Use --base for any other Git ref.
    #[command(visible_alias = "clone")]
    Add {
        /// Git URL or path to a local repository.
        url: String,
        /// Project name and directory under repos/ (default: repository name).
        directory: Option<String>,
        /// Git ref used when creating a task (default: origin/<remote default branch>).
        #[arg(long)]
        base: Option<String>,
    },
    /// Clone missing projects and fetch origin for existing projects.
    Sync { projects: Vec<String> },
    /// Remove a project and its local Git repository after checking for work to keep.
    ///
    /// Close its tasks and other worktrees first. Local changes and commits
    /// absent from the locally known origin branches prevent deletion.
    #[command(visible_alias = "rm")]
    Remove { project: String },
    /// Create or resume a task and print launch commands.
    ///
    /// Omit projects in a terminal to select them with Space and Enter.
    /// Existing tasks resume with their recorded projects and commits.
    Start {
        task: String,
        /// Projects to include (required for a new task outside a terminal).
        projects: Vec<String>,
        /// Use each project's configured base without asking.
        #[arg(short = 'D', long = "default")]
        use_defaults: bool,
        /// Use cached remote branches and commits without fetching.
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
    /// Delete tasks/<task>/ including shared resources after checking all worktrees. Keep the Git branches.
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
        Action::Add {
            url,
            directory,
            base,
        } => ws.add(&url, directory, base)?,
        Action::Sync { projects } => ws.sync(&projects)?,
        Action::Remove { project } => ws.remove(&project)?,
        Action::Start {
            task,
            projects,
            no_fetch,
            use_defaults,
        } => ws.start(&task, projects, no_fetch, use_defaults)?,
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
