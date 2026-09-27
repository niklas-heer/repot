//! Isolated discovery and root-configuration workflows through the binary.

#[cfg(test)]
mod tests {

    use std::fmt::Write;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    struct World {
        temp: TempDir,
    }

    impl World {
        fn new() -> Self {
            Self {
                temp: tempfile::tempdir().expect("temporary world"),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.temp.path().join(name)
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.temp.path())
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env_remove("GHQ_ROOT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_CONFIG_COUNT");
            command
        }

        fn git(&self, args: &[&str]) {
            let output = self.command("git").args(args).output().expect("run git");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn init(&self, path: &Path) {
            self.git(&["init", "--quiet", path.to_str().expect("UTF-8 test path")]);
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("run repot")
        }

        fn paths(output: &Output) -> Vec<PathBuf> {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let json: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("JSON output");
            json.as_array()
                .expect("repository array")
                .iter()
                .map(|entry| PathBuf::from(entry["path"].as_str().expect("repository path")))
                .collect()
        }
    }

    #[test]
    fn private_staging_and_archives_never_become_active_repositories() {
        let world = World::new();
        let active = world.path("ghq/example.test/team/active");
        let outside = world.path("outside/active");
        world.init(&active);
        world.init(&outside);
        let mut manifest = String::new();
        for prefix in [
            ".repot-clone-",
            ".repot-new-",
            ".repot-create-",
            ".repot-restore-",
            ".repot-trash/",
        ] {
            for parent in ["ghq/example.test/team", "outside"] {
                let relative = format!("{parent}/{prefix}private/checkout");
                world.init(&world.path(&relative));
                writeln!(
                    manifest,
                    "[[repo]]\nurl = ''\npath = '~/{relative}'\nrestore = false"
                )
                .expect("manifest entry");
            }
        }
        fs::create_dir_all(world.path("config/repot")).expect("config");
        fs::write(world.path("config/repot/repos.toml"), manifest).expect("manifest");
        assert_eq!(
            World::paths(&world.repot(&["list", "--json"])),
            [active.canonicalize().expect("active")]
        );
        let status = world.repot(&["status", "--no-fetch", "--json"]);
        let rows: serde_json::Value = serde_json::from_slice(&status.stdout).expect("status JSON");
        assert_eq!(rows.as_array().expect("statuses").len(), 1);
        assert_eq!(
            rows[0]["path"].as_str(),
            active.canonicalize().expect("active").to_str()
        );
        assert_eq!(
            World::paths(&world.repot(&["find", ".", "--json"])),
            [outside.canonicalize().expect("outside")]
        );
        assert!(
            World::paths(&world.repot(&[
                "find",
                "outside/.repot-clone-private/checkout",
                "--json"
            ]))
            .is_empty()
        );
    }

    #[test]
    fn lists_default_root_and_registered_extra_once_in_sorted_order() {
        let world = World::new();
        let tree = world.path("ghq/github.com/owner/project");
        let extra = world.path("outside");
        world.init(&tree);
        world.init(&extra);
        fs::create_dir_all(world.path("config/repot")).expect("config directory");
        fs::write(world.path("config/repot/repos.toml"),
        "[[repo]]\nurl = 'https://github.com/owner/project'\n\n[[repo]]\nurl = 'https://github.com/owner/extra'\npath = '~/outside'\nrestore = false\n")
        .expect("write manifest");
        let paths = World::paths(&world.repot(&["list", "--json"]));
        let mut expected = vec![
            tree.canonicalize().expect("tree"),
            extra.canonicalize().expect("extra"),
        ];
        expected.sort();
        assert_eq!(paths, expected);
    }

    #[test]
    fn manifest_supports_nested_namespaces_and_relative_extras() {
        let world = World::new();
        let repo = world.path("ghq/gitlab.example/team/subgroup/project");
        let extra = world.path("config/extra");
        world.init(&repo);
        world.init(&extra);
        fs::create_dir_all(world.path("config/repot")).expect("manifest parent");
        fs::write(
            world.path("config/repot/repos.toml"),
            "[[repo]]\nurl = 'ssh://git@gitlab.example:2222/team/subgroup/project.git'\n\n[[repo]]\nurl = 'git@gitlab.example:team/subgroup/project.git'\n\n[[repo]]\nurl = '/local/remote'\npath = '../extra'\nrestore = false\n",
        )
        .expect("manifest");
        let mut expected = vec![
            repo.canonicalize().expect("repo"),
            extra.canonicalize().expect("extra"),
        ];
        expected.sort();
        assert_eq!(World::paths(&world.repot(&["list", "--json"])), expected);
    }

    #[test]
    fn root_environment_overrides_multiple_git_roots() {
        let world = World::new();
        world.init(&world.path("first/one"));
        world.init(&world.path("second/two"));
        world.init(&world.path("override/three"));
        world.git(&["config", "--global", "--add", "ghq.root", "~/first"]);
        world.git(&["config", "--global", "--add", "ghq.root", "${HOME}/second"]);
        assert_eq!(World::paths(&world.repot(&["list", "--json"])).len(), 2);
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("GHQ_ROOT", world.path("override"))
            .args(["list", "--json"])
            .output()
            .expect("run repot");
        assert_eq!(
            World::paths(&output),
            vec![
                world
                    .path("override/three")
                    .canonicalize()
                    .expect("override")
            ]
        );
    }

    #[test]
    fn discovery_stops_at_repository_boundaries_and_finds_linked_worktrees() {
        let world = World::new();
        let repo = world.path("ghq/repo");
        world.init(&repo);
        world.init(&repo.join("nested"));
        world.git(&[
            "-C",
            repo.to_str().expect("path"),
            "-c",
            "user.name=Tester",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ]);
        let worktree = world.path("ghq/worktree");
        world.git(&[
            "-C",
            repo.to_str().expect("path"),
            "worktree",
            "add",
            "-b",
            "feature",
            worktree.to_str().expect("path"),
        ]);
        let paths = World::paths(&world.repot(&["list", "--json"]));
        assert_eq!(
            paths,
            vec![
                repo.canonicalize().expect("repo"),
                worktree.canonicalize().expect("worktree")
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_are_not_followed_even_when_cyclic() {
        let world = World::new();
        let root = world.path("ghq");
        world.init(&root.join("repo"));
        world.init(&world.path("outside"));
        std::os::unix::fs::symlink(&root, root.join("cycle")).expect("cycle");
        std::os::unix::fs::symlink(world.path("outside"), root.join("linked")).expect("link");
        assert_eq!(World::paths(&world.repot(&["list", "--json"])).len(), 1);
    }

    #[test]
    fn explicit_missing_or_invalid_manifests_fail_without_echoing_secrets() {
        let world = World::new();
        let missing = world.repot(&["--manifest", "absent.toml", "list"]);
        assert!(!missing.status.success());
        fs::write(world.path("bad.toml"), "secret = 'do-not-print-this-token'").expect("manifest");
        let bad = world.repot(&["--manifest", "bad.toml", "list"]);
        assert!(!bad.status.success());
        assert!(!String::from_utf8_lossy(&bad.stderr).contains("do-not-print-this-token"));
    }

    #[test]
    fn unsafe_remote_paths_and_credentials_are_rejected_without_disclosure() {
        let world = World::new();
        for url in [
            "https://token:secret@github.com/owner/repo",
            "https://github.com/../repo",
            "https://github.com/owner/%2e%2e",
            "-option",
            "file:///tmp/repo",
        ] {
            let text = format!("[[repo]]\nurl = '{url}'\n");
            fs::write(world.path("unsafe.toml"), text).expect("manifest");
            let output = world.repot(&["--manifest", "unsafe.toml", "list"]);
            assert!(!output.status.success(), "accepted {url}");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("token:secret"));
        }
    }
}
