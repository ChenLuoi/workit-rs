mod git;
mod history;
mod preset;
mod prompt;
mod report;
mod setup;
mod task_skills;
mod tasks;
mod templates;
mod tool;
mod workspace;

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tool::Tool;
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
    /// Create a committable Git workspace in an empty directory and optionally add a project.
    Init {
        /// Target directory (default: WK_WORKSPACE or current directory).
        dir: Option<PathBuf>,
        /// Git URL or local repository path for the first project.
        #[arg(long)]
        url: Option<String>,
        /// Explicit task base for the first project (also works with an empty remote).
        #[arg(long)]
        base: Option<String>,
        /// Role of the first project in this workspace.
        #[arg(long)]
        description: Option<String>,
        /// Confirm the target without prompting. Include --url to add a project using defaults.
        #[arg(short, long)]
        yes: bool,
        /// Enable these tools (comma-separated). With --yes the default is none.
        #[arg(long, value_delimiter = ',')]
        tools: Option<Vec<Tool>>,
        /// Install these bundled workspace skills (comma-separated). With --yes the default is none.
        #[arg(long, value_delimiter = ',', value_parser = clap::builder::PossibleValuesParser::new(templates::skill_names()))]
        skills: Option<Vec<String>>,
    },
    /// Choose personal or shared workspace settings.
    Setup(setup::SetupOptions),
    /// Clone into a new repos/<directory> and register the project in wk.yaml.
    ///
    /// With only a URL, confirm the directory, choose a remote branch by prefix,
    /// and optionally describe the project. Enter accepts the defaults.
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
        /// Role of the project in this workspace.
        #[arg(long)]
        description: Option<String>,
    },
    /// Manage the official skills registered in wk.yaml.
    Skills {
        #[command(subcommand)]
        command: SkillAction,
    },
    /// Restore missing clones, fetch origin and refresh reference files at each configured base.
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
    Start(tasks::StartOptions),
    /// Manage reusable combinations of projects, starting refs and task skills.
    Preset {
        #[command(subcommand)]
        command: preset::PresetAction,
    },
    /// Launch an enabled tool in the task directory. Omit the tool to choose interactively.
    Run {
        /// Name of the active task to launch. Use wk status to list active tasks.
        task: Option<String>,
        /// Tool to open for this task.
        #[arg(long, conflicts_with_all = ["codex", "claude"])]
        agent: Option<Tool>,
        /// Shorthand for --agent codex.
        #[arg(long, conflicts_with = "claude")]
        codex: bool,
        /// Shorthand for --agent claude.
        #[arg(long)]
        claude: bool,
    },
    /// Print the absolute path to a task or one of its projects.
    Path {
        task: String,
        project: Option<String>,
    },
    /// Show all projects and active tasks, or inspect one active task in detail.
    Status {
        /// Show each member's recorded starting point, current checkout and changed files.
        task: Option<String>,
        /// Output the report as JSON.
        #[arg(long)]
        json: bool,
        /// Fetch origin for the projects in this report before reading their status.
        #[arg(long)]
        fetch: bool,
    },
    /// Show workspace information and diagnose projects, active tasks, official skills and directory layout.
    Doctor,
    /// Delete tasks/<task>/ including shared resources after checking all worktrees. Keep the Git branches.
    Close {
        task: String,
        /// Confirm recovery of a damaged task outside a terminal.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// Replace registered skill contents with the version bundled in this binary.
    Sync { skill: Option<String> },
}

fn execute() -> Result<i32> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    let default_workspace = std::env::var_os("WK_WORKSPACE").map(PathBuf::from);
    if let Action::Init {
        dir,
        url,
        base,
        description,
        yes,
        tools,
        skills,
    } = cli.command
    {
        ensure!(
            dir.is_none() || cli.workspace.is_none(),
            "use either DIR or --workspace"
        );
        Workspace::init(
            &dir.or(cli.workspace).or(default_workspace).unwrap_or(cwd),
            url,
            base,
            description,
            yes,
            tools,
            skills,
        )?;
        return Ok(0);
    }
    let ws = if let Some(dir) = cli.workspace {
        Workspace::find(&dir)
    } else if let Some(dir) = default_workspace {
        Workspace::at_root(&dir)
    } else {
        Workspace::find(&cwd)
    };
    if matches!(cli.command, Action::Doctor) {
        report::doctor::run(&cwd, ws)?;
        return Ok(0);
    }
    let ws = ws?;
    match cli.command {
        Action::Init { .. } | Action::Doctor => unreachable!(),
        Action::Setup(options) => ws.setup(options)?,
        Action::Add {
            url,
            directory,
            base,
            description,
        } => ws.add(&url, directory, base, description)?,
        Action::Skills {
            command: SkillAction::Sync { skill },
        } => templates::sync_skills(&ws, skill.as_deref())?,
        Action::Sync { projects } => ws.sync(&projects)?,
        Action::Remove { project } => ws.remove(&project)?,
        Action::Start(options) => ws.start(options)?,
        Action::Preset { command } => ws.preset(command)?,
        Action::Path { task, project } => {
            let record = ws.active_index()?.into_task(&task)?;
            println!(
                "{}",
                ws.active_path(&task, &record, project.as_deref())?
                    .display()
            )
        }
        Action::Run {
            task,
            agent,
            codex,
            claude,
        } => {
            return ws.run_tool(
                task.as_deref(),
                agent
                    .or(codex.then_some(Tool::Codex))
                    .or(claude.then_some(Tool::Claude)),
            );
        }
        Action::Status { task, json, fetch } => ws.status(task.as_deref(), json, fetch)?,
        Action::Close { task, force } => ws.close(&task, force)?,
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
