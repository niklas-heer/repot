//! Reversible ghq-style removal preserves every checkout byte and supports recovery.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    struct World {
        temp: tempfile::TempDir,
    }
    impl World {
        fn new() -> Self {
            Self {
                temp: tempfile::tempdir().expect("world"),
            }
        }
        fn path(&self, path: &str) -> PathBuf {
            self.temp.path().join(path)
        }
        fn command(&self, binary: &str) -> Command {
            let mut command = Command::new(binary);
            command
                .current_dir(self.temp.path())
                .env("HOME", self.temp.path())
                .env("GHQ_ROOT", self.path("ghq"))
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Tester")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Tester")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("REPOT_CD_FILE");
            command
        }
        fn git(&self, relative: &str, args: &[&str]) {
            let output = self
                .command("git")
                .current_dir(self.path(relative))
                .args(args)
                .output()
                .expect("Git");
            success(&output);
        }
        fn init(&self, relative: &str, bare: bool) {
            let path = self.path(relative);
            let mut args = vec!["init", "--quiet", "--initial-branch=main"];
            if bare {
                args.push("--bare");
            }
            args.push(text(&path));
            self.git("", &args);
        }
        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot")
        }
        fn archive(&self, query: &str) -> String {
            let output = self.repot(&["rm", query, "--json"]);
            success(&output);
            let report: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("archive JSON");
            report[0]["id"].as_str().expect("archive ID").to_owned()
        }
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 fixture")
    }
    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn walk(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).expect("snapshot directory") {
                let entry = entry.expect("snapshot entry");
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("relative")
                    .to_path_buf();
                if entry.file_type().expect("kind").is_dir() {
                    files.insert(relative, b"directory".to_vec());
                    walk(root, &entry.path(), files);
                } else {
                    files.insert(relative, fs::read(entry.path()).expect("bytes"));
                }
            }
        }
        let mut files = BTreeMap::new();
        walk(root, root, &mut files);
        files
    }

    #[test]
    fn archive_and_restore_preserve_dirty_index_stashes_untracked_and_ignored_files_exactly() {
        let world = World::new();
        let relative = "ghq/github.com/owner/project";
        world.init(relative, false);
        let source = world.path(relative);
        fs::write(source.join("tracked"), "initial\n").expect("file");
        world.git(relative, &["add", "."]);
        world.git(relative, &["commit", "-m", "initial"]);
        fs::write(source.join("tracked"), "stashed work\n").expect("stash edit");
        world.git(relative, &["stash", "push", "-m", "existing stash"]);
        fs::write(source.join("tracked"), "staged work\n").expect("staged");
        world.git(relative, &["add", "."]);
        fs::write(source.join("tracked"), b"unstaged\0\xff\n").expect("unstaged");
        fs::write(source.join("untracked"), "untracked\n").expect("untracked");
        fs::write(source.join(".git/info/exclude"), "ignored\n").expect("exclude");
        fs::write(source.join("ignored"), "ignored\n").expect("ignored");
        success(&world.repot(&["adopt", text(&source), "--register"]));
        let before = snapshot(&source);
        let id = world.archive("owner/project");
        assert!(!source.exists());
        let archived = world.path(&format!("ghq/.repot-trash/{id}/repo"));
        assert_eq!(snapshot(&archived), before);
        for args in [
            vec!["list", "--json"],
            vec!["status", "--no-fetch", "--json"],
            vec!["find", text(world.temp.path()), "--json"],
        ] {
            let output = world.repot(&args);
            success(&output);
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "[]",
                "archived work must stay outside discovery"
            );
        }
        let listed = world.repot(&["trash", "list", "--json"]);
        success(&listed);
        let entries: serde_json::Value =
            serde_json::from_slice(&listed.stdout).expect("trash JSON");
        assert_eq!(entries[0]["id"], id);
        let output = world.repot(&["trash", "restore", &id]);
        success(&output);
        assert_eq!(snapshot(&source), before);
        assert!(!archived.exists());
        assert_eq!(
            String::from_utf8_lossy(&world.repot(&["trash", "list", "--json"]).stdout).trim(),
            "[]"
        );
    }

    #[test]
    fn dry_runs_change_nothing_and_restore_never_overwrites_new_work() {
        let world = World::new();
        world.init("ghq/github.com/owner/project", false);
        let before = snapshot(world.temp.path());
        success(&world.repot(&["rm", "project", "--dry-run"]));
        assert_eq!(snapshot(world.temp.path()), before);
        let id = world.archive("project");
        let archived = snapshot(world.temp.path());
        success(&world.repot(&["trash", "restore", &id, "--dry-run"]));
        assert_eq!(snapshot(world.temp.path()), archived);
        fs::create_dir(world.path("ghq/github.com/owner/project")).expect("new source collision");
        fs::write(
            world.path("ghq/github.com/owner/project/new-work"),
            "keep\n",
        )
        .expect("new work");
        let collision = snapshot(world.temp.path());
        for args in [
            vec!["trash", "restore", &id],
            vec!["trash", "restore", &id, "--dry-run"],
        ] {
            assert!(!world.repot(&args).status.success());
            assert_eq!(snapshot(world.temp.path()), collision);
        }
    }

    #[test]
    fn bare_repository_archives_and_restores_without_changing_its_bytes() {
        let world = World::new();
        world.init("ghq/github.com/owner/project.git", true);
        let source = world.path("ghq/github.com/owner/project.git");
        let before = snapshot(&source);
        let output = world.repot(&["rm", "https://github.com/owner/project", "--bare", "--json"]);
        success(&output);
        let report: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("archive JSON");
        let id = report[0]["id"].as_str().expect("ID");
        assert!(!source.exists());
        success(&world.repot(&["trash", "restore", id]));
        assert_eq!(snapshot(&source), before);
    }

    #[test]
    fn ambiguous_queries_and_linked_worktrees_are_refused_without_creating_archives() {
        let world = World::new();
        world.init("ghq/github.com/one/project", false);
        world.init("ghq/github.com/two/project", false);
        let before = snapshot(world.temp.path());
        assert!(!world.repot(&["rm", "project"]).status.success());
        assert_eq!(snapshot(world.temp.path()), before);
        world.git(
            "ghq/github.com/one/project",
            &["commit", "--allow-empty", "-m", "initial"],
        );
        world.git(
            "ghq/github.com/one/project",
            &[
                "worktree",
                "add",
                "-b",
                "feature",
                text(&world.path("linked")),
            ],
        );
        let before = snapshot(world.temp.path());
        assert!(!world.repot(&["rm", "one/project"]).status.success());
        assert_eq!(snapshot(world.temp.path()), before);
        assert!(!world.path("ghq/.repot-trash").exists());
    }

    #[cfg(unix)]
    #[test]
    fn archive_metadata_symlinks_and_unsafe_ids_are_not_followed() {
        let world = World::new();
        world.init("ghq/github.com/owner/project", false);
        let id = world.archive("project");
        let metadata = world.path(&format!("ghq/.repot-trash/{id}/metadata.json"));
        let external = world.path("external-metadata.json");
        fs::rename(&metadata, &external).expect("move metadata fixture");
        std::os::unix::fs::symlink(&external, &metadata).expect("malicious metadata link");
        assert!(!world.repot(&["trash", "restore", &id]).status.success());
        assert!(!world.repot(&["trash", "list"]).status.success());
        assert!(
            world
                .path(&format!("ghq/.repot-trash/{id}/repo/.git"))
                .exists()
        );
        for id in ["../escape", ".", "..", "/tmp"] {
            assert!(!world.repot(&["trash", "restore", id]).status.success());
        }
        assert!(
            fs::symlink_metadata(metadata)
                .expect("link remains")
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn bare_repositories_with_external_metadata_are_not_archived() {
        let world = World::new();
        world.init("ghq/github.com/owner/project.git", true);
        let source = world.path("ghq/github.com/owner/project.git");
        let external = world.path("external-objects");
        fs::rename(source.join("objects"), &external).expect("external metadata fixture");
        std::os::unix::fs::symlink(&external, source.join("objects"))
            .expect("object directory link");
        assert!(
            !world
                .repot(&["rm", "project.git", "--bare"])
                .status
                .success()
        );
        assert!(source.join("HEAD").is_file());
        assert!(
            fs::symlink_metadata(source.join("objects"))
                .expect("link retained")
                .is_symlink()
        );
        assert!(!world.path("ghq/.repot-trash").exists());
    }

    #[test]
    fn bare_repository_with_an_operation_marker_is_not_archived() {
        let world = World::new();
        world.init("ghq/github.com/owner/project.git", true);
        fs::write(
            world.path("ghq/github.com/owner/project.git/MERGE_HEAD"),
            "operation marker\n",
        )
        .expect("operation fixture");
        let before = snapshot(world.temp.path());
        assert!(
            !world
                .repot(&["rm", "project.git", "--bare"])
                .status
                .success()
        );
        assert_eq!(snapshot(world.temp.path()), before);
    }

    #[cfg(unix)]
    #[test]
    fn bare_config_inspection_failure_does_not_masquerade_as_missing_configuration() {
        use std::os::unix::fs::PermissionsExt;

        let world = World::new();
        world.init("ghq/github.com/owner/project.git", true);
        let real = world
            .command("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("Git path");
        success(&real);
        fs::create_dir(world.path("bin")).expect("bin");
        let wrapper = world.path("bin/git");
        fs::write(&wrapper, "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = core.worktree ]; then exit 2; fi\ndone\nexec \"$REPOT_REAL_GIT\" \"$@\"\n").expect("Git failure fixture");
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("executable");
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(world.path("bin")).chain(std::env::split_paths(&inherited)),
        )
        .expect("PATH");
        let before = snapshot(world.temp.path());
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("PATH", path)
            .env(
                "REPOT_REAL_GIT",
                String::from_utf8(real.stdout).expect("Git path").trim(),
            )
            .args(["rm", "project.git", "--bare"])
            .output()
            .expect("archive with config failure");
        assert!(!output.status.success());
        assert_eq!(snapshot(world.temp.path()), before);
    }

    #[test]
    fn migrate_moves_and_registers_the_repository_with_ghq_flags() {
        let world = World::new();
        world.init("stray", false);
        world.git(
            "stray",
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owner/project",
            ],
        );
        let before = snapshot(world.temp.path());
        success(&world.repot(&["migrate", "-y", "--dry-run", "stray"]));
        assert_eq!(snapshot(world.temp.path()), before);
        success(&world.repot(&["migrate", "-y", "stray"]));
        assert!(!world.path("stray").exists());
        assert!(world.path("ghq/github.com/owner/project/.git").is_dir());
        let text =
            fs::read_to_string(world.path("config/repot/repos.toml")).expect("registered manifest");
        assert!(text.contains("https://github.com/owner/project"));
    }
}
