use super::{ChangedFile, Checkout, MemberStatus, ProjectStatus, Report};
use crate::tasks::Task;
use console::{Alignment, Term, measure_text_width, pad_str, style};

const HEADERS: [&str; 4] = ["Name", "Snapshot", "Task base", "Remote"];

impl ProjectStatus {
    fn columns(&self) -> [&str; 4] {
        [
            &self.project.name,
            self.commit.as_deref().map_or("—", |oid| &oid[..12]),
            &self.project.base,
            &self.project.url,
        ]
    }
}

fn project_row(cells: [&str; 4], widths: &[usize; 4]) -> String {
    let [name, branch, base, remote] = cells;
    format!(
        "  {}  {}  {}  {remote}",
        pad_str(name, widths[0], Alignment::Left, None),
        pad_str(branch, widths[1], Alignment::Left, None),
        pad_str(base, widths[2], Alignment::Left, None),
    )
}

impl Checkout {
    fn tracking(&self) -> Option<String> {
        self.upstream
            .as_ref()
            .map(|upstream| match (self.ahead, self.behind) {
                (Some(a), Some(b)) => format!("{upstream} ↑{a} ↓{b}"),
                _ => upstream.clone(),
            })
    }

    fn summary(&self) -> String {
        let mut changes = Vec::new();
        for (count, label) in [
            (
                self.conflicts,
                if self.conflicts == 1 {
                    "conflict"
                } else {
                    "conflicts"
                },
            ),
            (self.staged, "staged"),
            (self.modified, "modified"),
            (self.untracked, "untracked"),
        ] {
            if count > 0 {
                changes.push(format!("{count} {label}"));
            }
        }
        if changes.is_empty() {
            return style("✓ clean").green().to_string();
        }
        let text = style(changes.join(", "));
        if self.conflicts > 0 {
            text.red()
        } else {
            text.yellow()
        }
        .to_string()
    }
}

fn print_files(files: &[ChangedFile]) {
    let display_path = |path: &str| {
        if path.chars().any(char::is_control) {
            format!("{path:?}")
        } else {
            path.to_owned()
        }
    };
    for f in files {
        let path = display_path(&f.path);
        let path = match &f.original_path {
            Some(old) => format!("{} → {path}", display_path(old)),
            None => path,
        };
        println!("                 {:<4} {path}", f.status);
    }
}

fn print_task(name: &str, task: &Task<MemberStatus>) {
    println!("{}  {}  [{}]", style("Task").bold(), name, task.phase);
    println!("  tasks/{name}/");
    if let Some(preset) = &task.preset {
        println!("  Preset  {preset}");
    }
    if !task.skills.is_empty() {
        println!(
            "  Skills  {}",
            task.skills
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for (project, member) in &task.projects {
        println!("\n  {}", style(project).bold());
        if let Some(start) = &member.start {
            println!("    Start        {} @ {}", start.base, &start.commit[..12]);
        }
        if let Some(c) = &member.current {
            let head = c.commit.as_ref().map_or("(no commit)", |s| &s[..12]);
            println!("    Current      {} @ {head}", c.branch);
            if let Some(upstream) = c.tracking() {
                println!("    Upstream     {upstream}");
            }
            if let Some(files) = &member.committed_changes {
                let noun = if files.len() == 1 { "file" } else { "files" };
                println!(
                    "    Committed    {} {noun} changed since start",
                    files.len()
                );
                print_files(files);
            }
            println!("    Working tree {}", c.summary());
            if let Some(files) = &c.files {
                print_files(files);
            }
        }
        if let Some(error) = &member.error {
            println!("    {} {error}", style("✗").red());
        }
    }
}

impl Report {
    pub(super) fn print(&self, selected: Option<&str>) {
        if let Some(name) = selected {
            print_task(name, &self.tasks[name]);
            return;
        }
        let width = Term::stdout()
            .size_checked()
            .filter(|&(_, cols)| cols > 0)
            .map_or(usize::MAX, |(_, cols)| usize::from(cols));
        println!("{} ({})", style("Projects").bold(), self.projects.len());
        self.print_projects(width);
        println!("\n{} ({})", style("Active tasks").bold(), self.tasks.len());
        if self.tasks.is_empty() {
            println!("  No active tasks.");
        }
        for (index, (name, task)) in self.tasks.iter().enumerate() {
            if index > 0 {
                println!();
            }
            println!("  {}", style(name).bold());
            let name_width = task
                .projects
                .keys()
                .map(|s| measure_text_width(s))
                .max()
                .unwrap_or(0);
            for (i, (project, entry)) in task.projects.iter().enumerate() {
                let twig = if i + 1 == task.projects.len() {
                    "└─"
                } else {
                    "├─"
                };
                let name = pad_str(project, name_width, Alignment::Left, None);
                let prefix = format!("    {} {name}", style(twig).dim());
                let detail = match (&entry.error, &entry.current) {
                    (Some(error), _) => format!("{} {error}", style("✗").red()),
                    (_, Some(c)) => {
                        let mut text = format!("{}  {}", c.branch, c.summary());
                        if let Some(upstream) = c.tracking() {
                            text.push_str(&format!("  → {upstream}"));
                        }
                        text
                    }
                    (None, None) => unreachable!("a member has a checkout or an error"),
                };
                if measure_text_width(&prefix) + 2 + measure_text_width(&detail) > width {
                    println!("{}\n       {detail}", prefix.trim_end());
                } else {
                    println!("{prefix}  {detail}");
                }
            }
        }
    }

    fn print_projects(&self, width: usize) {
        if self.projects.is_empty() {
            println!("  No projects registered. Use wk add <url> to add one.");
            return;
        }
        let mut widths = HEADERS.map(measure_text_width);
        for project in self.projects.values() {
            for (w, cell) in widths.iter_mut().zip(project.columns()) {
                *w = (*w).max(measure_text_width(cell));
            }
        }
        let inline = widths.iter().sum::<usize>() + 8 <= width;
        if inline {
            println!("{}", style(project_row(HEADERS, &widths)).dim());
        }
        for (index, project) in self.projects.values().enumerate() {
            let cells = project.columns();
            if inline {
                println!("{}", project_row(cells, &widths));
            } else {
                if index > 0 {
                    println!();
                }
                println!("  {}", style(cells[0]).bold());
                for (label, value) in HEADERS[1..].iter().zip(&cells[1..]) {
                    println!("    {}  {value}", style(format!("{label:<9}")).dim());
                }
            }
            if let Some(error) = &project.error {
                println!("    {} {error}", style("✗").red());
            }
            if let Some(role) = &project.project.description {
                println!("    {role}");
            }
        }
    }
}
