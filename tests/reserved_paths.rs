//! Internal staging names cannot become successful but invisible projects.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    struct World(tempfile::TempDir);

    impl World {
        fn new() -> Self {
            Self(tempfile::tempdir().expect("temporary home"))
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.0.path())
                .env("HOME", self.0.path())
                .env("XDG_CONFIG_HOME", self.0.path().join("config"))
                .env("GHQ_ROOT", self.0.path().join("projects"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.0.path().join("gitconfig"))
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn repot(&self, args: &[&str], dry_run: bool) -> Output {
            let mut command = self.command(env!("CARGO_BIN_EXE_repot"));
            command.args(args);
            if dry_run {
                command.arg("--dry-run");
            }
            command.output().expect("repot")
        }

        fn git(&self, path: &Path, args: &[&str]) {
            let output = self
                .command("git")
                .current_dir(path)
                .args(args)
                .output()
                .expect("Git");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn init(&self, name: &str) {
            self.git(self.0.path(), &["init", "--quiet", "--template=", name]);
        }
    }

    fn rejected(output: &Output) {
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.status.success(), "unexpected success: {report}");
        assert!(report.contains("reserved repot"), "{report}");
    }

    #[test]
    fn creation_and_import_reject_reserved_names_before_any_filesystem_changes() {
        let world = World::new();
        for name in [
            ".repot-clone-project",
            ".repot-new-project",
            ".repot-create-project",
            ".repot-restore-project",
            ".repot-trash",
        ] {
            let repository = format!("example.test/team/{name}");
            for dry_run in [true, false] {
                for args in [
                    vec!["new", name],
                    vec!["new", "project", "--namespace", name],
                    vec!["create", &repository],
                    vec!["get", &repository, "--vcs", "git"],
                ] {
                    rejected(&world.repot(&args, dry_run));
                }
            }
        }
        assert!(!world.0.path().join("projects").exists());
        assert!(!world.0.path().join("config").exists());
    }

    #[test]
    fn adoption_registration_and_publish_refuse_hidden_paths_without_moving_work() {
        let world = World::new();
        world.init("source");
        world.init(".repot-new-existing");
        let source = world.0.path().join("source");
        fs::write(source.join("dirty"), "keep these bytes").expect("dirty work");
        world.git(
            &source,
            &[
                "remote",
                "add",
                "origin",
                "https://example.test/team/.repot-clone-project",
            ],
        );
        for dry_run in [true, false] {
            rejected(&world.repot(&["adopt", "source"], dry_run));
            rejected(&world.repot(&["migrate", "source", "--yes"], dry_run));
            rejected(&world.repot(&["adopt", ".repot-new-existing", "--register"], dry_run));
            let mut publish = world.command(env!("CARGO_BIN_EXE_repot"));
            publish.current_dir(&source).args([
                "publish",
                "team/.repot-clone-project",
                "--visibility",
                "public",
            ]);
            if dry_run {
                publish.arg("--dry-run");
            }
            rejected(&publish.output().expect("publish"));
        }
        assert_eq!(
            fs::read_to_string(source.join("dirty")).expect("preserved"),
            "keep these bytes"
        );
        assert!(!world.0.path().join("projects").exists());
        assert!(!world.0.path().join("config").exists());
    }

    #[test]
    fn explicit_restore_paths_are_checked_in_dry_run_and_real_execution() {
        let world = World::new();
        fs::create_dir_all(world.0.path().join("config/repot")).expect("config");
        fs::write(
            world.0.path().join("config/repot/repos.toml"),
            "[[repo]]\nurl = '/missing-local-remote'\npath = '~/outside/.repot-restore-project'\n",
        )
        .expect("manifest");
        for dry_run in [true, false] {
            rejected(&world.repot(&["restore"], dry_run));
        }
        assert!(!world.0.path().join("outside").exists());
    }

    #[cfg(unix)]
    #[test]
    fn aliases_into_private_staging_cannot_bypass_destination_validation() {
        let world = World::new();
        let private = world.0.path().join(".repot-clone-existing");
        fs::create_dir(&private).expect("private staging");
        fs::create_dir(world.0.path().join("projects")).expect("root");
        std::os::unix::fs::symlink(&private, world.0.path().join("projects/example.test"))
            .expect("alias");
        for dry_run in [true, false] {
            rejected(&world.repot(&["create", "example.test/team/project"], dry_run));
            rejected(&world.repot(
                &["get", "example.test/team/project", "--vcs", "git"],
                dry_run,
            ));
        }
        assert_eq!(fs::read_dir(&private).expect("private staging").count(), 0);
    }
}
