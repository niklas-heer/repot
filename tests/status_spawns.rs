//! Git process budget for cached status over clean checkouts.
//!
//! A `git` shim on `PATH` logs every invocation made by `repot status
//! --no-fetch`, so the budget is enforced through the public binary.

#![cfg(unix)]

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    const CHECKOUTS: usize = 4;
    /// Git processes allowed per clean checkout by `status --no-fetch`.
    const GIT_PER_CLEAN_CHECKOUT: usize = 4;
    /// Asked once per run, not per checkout: `git version`, for the stash count.
    const GIT_PER_RUN: usize = 1;

    fn real_git() -> PathBuf {
        std::env::split_paths(&std::env::var_os("PATH").expect("PATH"))
            .map(|dir| dir.join("git"))
            .find(|candidate| candidate.is_file())
            .expect("git on PATH")
    }

    fn isolated<'a>(command: &'a mut Command, home: &Path) -> &'a mut Command {
        command
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_STATE_HOME", home.join("state"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
    }

    fn git(home: &Path, cwd: &Path, args: &[&str]) {
        let output = isolated(&mut Command::new("git"), home)
            .current_dir(cwd)
            .args(args)
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Two worktrees on one branch: the worktree listing is only run when linked
    /// worktrees exist, so this case must still be reported.
    #[test]
    fn branch_checked_out_in_linked_worktree_is_still_reported() {
        let home = TempDir::new().expect("isolated home");
        let root = home.path();
        let remote = root.join("remote.git");
        let source = root.join("source");
        let checkout = root.join("projects/host/owner/repo");
        let linked = root.join("projects/host/owner/linked");
        git(
            root,
            root,
            &["init", "--bare", "--initial-branch=main", "remote.git"],
        );
        git(root, root, &["init", "--initial-branch=main", "source"]);
        fs::write(source.join("README"), "base").expect("file");
        git(root, &source, &["add", "--", "README"]);
        git(root, &source, &["commit", "-m", "initial"]);
        git(root, &source, &["remote", "add", "origin", "../remote.git"]);
        git(root, &source, &["push", "-u", "origin", "main"]);
        fs::create_dir_all(checkout.parent().expect("parent")).expect("tree");
        git(
            root,
            root,
            &[
                "clone",
                remote.to_str().expect("remote path"),
                checkout.to_str().expect("checkout path"),
            ],
        );
        let linked_str = linked.to_str().expect("linked path");
        git(
            root,
            &checkout,
            &["worktree", "add", "--quiet", "--detach", linked_str],
        );
        git(
            root,
            &linked,
            &["switch", "--quiet", "--ignore-other-worktrees", "main"],
        );

        let output = isolated(&mut Command::new(env!("CARGO_BIN_EXE_repot")), root)
            .current_dir(root)
            .env("GHQ_ROOT", root.join("projects"))
            .env_remove("REPOT_CD_FILE")
            .args(["status", "--no-fetch", "--json"])
            .output()
            .expect("repot runs");
        let reports: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let reports = reports.as_array().expect("report list");
        assert_eq!(reports.len(), 2);
        for report in reports {
            assert_eq!(report["action"], "review");
            assert_eq!(
                report["reason"],
                "current branch is checked out in another worktree"
            );
        }
    }

    #[test]
    fn cached_status_stays_within_git_budget_per_clean_checkout() {
        let home = TempDir::new().expect("isolated home");
        let root = home.path();
        let remote = root.join("remote.git");
        let source = root.join("source");
        git(
            root,
            root,
            &["init", "--bare", "--initial-branch=main", "remote.git"],
        );
        git(root, root, &["init", "--initial-branch=main", "source"]);
        fs::write(source.join("README"), "base").expect("file");
        git(root, &source, &["add", "--", "README"]);
        git(root, &source, &["commit", "-m", "initial"]);
        git(root, &source, &["remote", "add", "origin", "../remote.git"]);
        git(root, &source, &["push", "-u", "origin", "main"]);
        for index in 0..CHECKOUTS {
            let parent = root.join(format!("projects/host/owner/repo{index}"));
            fs::create_dir_all(parent.parent().expect("parent")).expect("tree");
            git(
                root,
                root,
                &[
                    "clone",
                    remote.to_str().expect("remote path"),
                    parent.to_str().expect("checkout path"),
                ],
            );
        }

        // The shim records one line per Git process, then runs the real Git.
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("bin");
        let log = root.join("git.log");
        let shim = bin.join("git");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
                log.display(),
                real_git().display()
            ),
        )
        .expect("shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("shim mode");
        let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
            &std::env::var_os("PATH").expect("PATH"),
        )))
        .expect("PATH");

        let output = isolated(&mut Command::new(env!("CARGO_BIN_EXE_repot")), root)
            .current_dir(root)
            .env("PATH", path)
            .env("GHQ_ROOT", root.join("projects"))
            .env_remove("REPOT_CD_FILE")
            .args(["status", "--no-fetch", "--json"])
            .output()
            .expect("repot runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reports: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let reports = reports.as_array().expect("report list");
        assert_eq!(reports.len(), CHECKOUTS);
        assert!(reports.iter().all(|report| report["state"] == "synced"));

        let spawns = fs::read_to_string(&log).expect("Git log").lines().count();
        assert!(
            spawns <= GIT_PER_CLEAN_CHECKOUT * CHECKOUTS + GIT_PER_RUN,
            "{spawns} Git processes for {CHECKOUTS} clean checkouts; budget is {GIT_PER_CLEAN_CHECKOUT} each plus {GIT_PER_RUN}"
        );
    }

    /// Git before 2.35 omits porcelain v2's `# stash` header; a stash must still
    /// keep an otherwise synced checkout out of "Up to date".
    #[test]
    fn stashes_are_counted_when_git_omits_the_stash_header() {
        let home = TempDir::new().expect("isolated home");
        let root = home.path();
        git(
            root,
            root,
            &["init", "--bare", "--initial-branch=main", "remote.git"],
        );
        let checkout = root.join("projects/host/owner/stashed");
        fs::create_dir_all(&checkout).expect("tree");
        git(root, &checkout, &["init", "--initial-branch=main"]);
        fs::write(checkout.join("README"), "base").expect("file");
        git(root, &checkout, &["add", "--", "README"]);
        git(root, &checkout, &["commit", "-m", "initial"]);
        git(
            root,
            &checkout,
            &["remote", "add", "origin", "../../../../remote.git"],
        );
        git(root, &checkout, &["push", "-u", "origin", "main"]);
        fs::write(checkout.join("README"), "draft").expect("change");
        git(root, &checkout, &["stash"]);

        // Behaves like Git 2.34: older version string, no stash header.
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("bin");
        let shim = bin.join("git");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\ncase \" $* \" in\n  *' version '*) echo 'git version 2.34.1'; exit 0 ;;\n  *' status '*) '{git}' \"$@\" | perl -0 -ne 'print unless /^# stash /'; exit 0 ;;\nesac\nexec '{git}' \"$@\"\n",
                git = real_git().display()
            ),
        )
        .expect("shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("shim mode");
        let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
            &std::env::var_os("PATH").expect("PATH"),
        )))
        .expect("PATH");

        let output = isolated(&mut Command::new(env!("CARGO_BIN_EXE_repot")), root)
            .current_dir(root)
            .env("PATH", path)
            .env("GHQ_ROOT", root.join("projects"))
            .env_remove("REPOT_CD_FILE")
            .args(["status", "--no-fetch", "--json"])
            .output()
            .expect("repot runs");
        let reports: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
        let report = &reports[0];
        assert_eq!(report["stashes"], 1, "{report}");
        assert_ne!(report["action"], "none", "{report}");
    }
}
