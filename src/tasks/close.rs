use super::*;

fn task_root_ref(dir: &Path, phase: Phase) -> Result<String> {
    if phase != Phase::Active && !child(dir, ".git")?.try_exists()? {
        return Ok(String::new());
    }
    check_task_git_dir(dir)?;
    git(
        dir,
        &[
            "--git-dir=.git",
            "for-each-ref",
            "--count=1",
            "--format=%(refname)",
        ],
    )
}

impl Workspace {
    pub fn close(&self, name: &str, force: bool) -> Result<()> {
        let _lock = self.lock()?;
        task_name_ok(name)?;
        let mut index = self.active_index()?;
        self.check_task_namespace(name, &index)?;
        let mut task = index
            .tasks
            .remove(name)
            .with_context(|| format!("no open task {name}"))?;
        let config = self.config()?;
        let dir = self.task_dir(name)?;
        let mut damage = Vec::new();
        match task_root_ref(&dir, task.phase) {
            Err(error) => damage.push(format!("{error:#}")),
            Ok(reference) => ensure!(
                reference.is_empty(),
                "task {name} contains {reference} in its root repository; preserve and remove the ref before closing"
            ),
        }
        let mut removals = Vec::new();
        let mut stale = Vec::new();
        for (p, member) in &mut task.projects {
            let tree = child(&dir, p)?;
            if !self.repo(p)?.try_exists()? {
                damage.push(format!("{p}: reference clone is missing"));
                continue;
            }
            let spec = config
                .projects
                .get(p)
                .with_context(|| format!("unregistered project {p}"))?;
            let repo = self.verify_clean_repo(p, spec)?;
            let listing = bytes(&repo, &["worktree", "list", "--porcelain", "-z"])?;
            let row = git_record(&listing, &tree)?;
            if !tree.exists() && row.is_none() && task.phase != Phase::Active {
                continue;
            }
            let Some(row) = row else {
                damage.push(format!("worktree is not registered: {}", tree.display()));
                continue;
            };
            ensure!(
                !row.split(|b| *b == 0)
                    .any(|f| f == b"locked" || f.starts_with(b"locked ")),
                "locked worktree: {}",
                tree.display()
            );
            if let Err(error) = validate_tree(&repo, &tree, name) {
                let branch = format!("branch refs/heads/{name}");
                ensure!(
                    row.split(|b| *b == 0).any(|f| f == branch.as_bytes()),
                    "unexpected Git branch at {}; preserve the worktree before closing",
                    tree.display()
                );
                damage.push(format!("{}: {error:#}", tree.display()));
                if let Some(head) = row
                    .split(|b| *b == 0)
                    .find_map(|f| f.strip_prefix(b"HEAD "))
                {
                    member.final_commit = Some(std::str::from_utf8(head)?.to_owned());
                    member.check_commit(p)?;
                }
                stale.push((repo, tree));
                continue;
            }
            check_clean(&tree)?;
            let head = git(&tree, &["rev-parse", "HEAD"])?;
            member.final_commit = Some(head.clone());
            removals.push((repo, tree, head));
        }
        let recovery = task.phase == Phase::Recovering || !damage.is_empty();
        let directory_present = dir.try_exists()?;
        if recovery {
            eprintln!("Warning: damaged task {name}");
            for reason in &damage {
                eprintln!("  {reason}");
            }
            eprintln!(
                "Cleanup will archive {name} and remove its worktree registrations under {}.",
                dir.display()
            );
            for (_, tree, _) in &removals {
                eprintln!("  Remove clean worktree {}", tree.display());
            }
            for (_, tree) in &stale {
                eprintln!("  Remove stale Git registration for {}", tree.display());
            }
            if directory_present {
                eprintln!(
                    "All remaining files under {} will be permanently deleted.",
                    dir.display()
                );
            }
            // A previous release confirmed a different action: moving the files to a backup.
            let confirmed = task.phase == Phase::Recovering && task.history.is_some();
            if !confirmed && prompt::interactive() {
                ensure!(
                    Confirm::new("Clean up this damaged task?")
                        .with_default(false)
                        .prompt()?,
                    "cleanup cancelled"
                );
            } else {
                ensure!(
                    confirmed || force,
                    "to confirm recovery outside a terminal, run wk close {name} --force"
                );
            }
        }
        let resuming = matches!(task.phase, Phase::Closing | Phase::Recovering);
        task.phase = if recovery {
            Phase::Recovering
        } else {
            Phase::Closing
        };
        let closed_at_unix_ms = *task.closing_at_unix_ms.get_or_insert(now_ms()?);
        self.prepare_history(name, &mut task, closed_at_unix_ms, damage, resuming)?;
        index.save_task(self, name, &task)?;
        for (repo, tree, head) in removals {
            validate_tree(&repo, &tree, name)?;
            check_clean(&tree)?;
            ensure!(
                git(&tree, &["rev-parse", "HEAD"])? == head,
                "worktree HEAD changed while closing; retry"
            );
            git(&repo, &["worktree", "remove", "--", &git_path(&tree)?])?;
        }
        // A retry can arrive after the directory was removed but before the record was saved.
        if dir.try_exists()? {
            ensure!(
                directory_present,
                "task directory appeared during cleanup; retry"
            );
            fs::remove_dir_all(&dir)
                .with_context(|| format!("remove task directory {}", dir.display()))?;
        }
        // Remove only this task's stale registrations. A global prune could affect siblings.
        for (repo, tree) in stale {
            ensure!(
                !tree.try_exists()?,
                "worktree reappeared at {}; retry",
                tree.display()
            );
            git(&repo, &["worktree", "remove", "--", &git_path(&tree)?])?;
        }
        if !task.skills.is_empty() {
            task_skills::discard_pending(&self.pending_skills(name, task.started_at_unix_ms)?)?;
        }
        let root = self.0.join("tasks");
        for group in dir.ancestors().skip(1).take_while(|p| *p != root) {
            match fs::remove_dir(group) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("remove empty group {}", group.display()));
                }
            }
        }
        self.archive_task(name, &task)?;
        index.tasks.remove(name);
        save_yaml(&self.active_file()?, &index)?;
        println!("Closed {name}. Task directory removed; project branches retained.");
        Ok(())
    }
}
