//! Independent real-Git checks for clone recursion, concurrent installation and bare updates.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    struct World {
        temp: tempfile::TempDir,
    }
    impl World {
        fn new() -> Self {
            let world = Self {
                temp: tempfile::tempdir().expect("world"),
            };
            world.git(
                world.temp.path(),
                &["init", "--initial-branch=main", "seed"],
            );
            fs::write(world.path("seed/base"), "initial\n").expect("initial file");
            world.git(&world.path("seed"), &["add", "."]);
            world.git(&world.path("seed"), &["commit", "-m", "initial"]);
            world.git(
                world.temp.path(),
                &["config", "--global", "ghq.user", "team"],
            );
            world.git(
                world.temp.path(),
                &["config", "--global", "ghq.defaultHost", "example.test"],
            );
            for prefix in [
                "https://example.test/",
                "ssh://git@example.test/",
                "git://example.test/",
            ] {
                world.git(
                    world.temp.path(),
                    &[
                        "config",
                        "--global",
                        "--add",
                        &format!("url.file://{}/.insteadOf", world.path("remotes").display()),
                        prefix,
                    ],
                );
            }
            world
        }
        fn path(&self, relative: &str) -> PathBuf {
            self.temp.path().join(relative)
        }
        fn destination(&self, repository: &str) -> PathBuf {
            self.path("ghq/example.test").join(repository)
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
        fn git(&self, path: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(path)
                .args(args)
                .output()
                .expect("Git");
            success(&output);
            String::from_utf8(output.stdout)
                .expect("Git output")
                .trim()
                .to_owned()
        }
        fn remote(&self, repository: &str) -> PathBuf {
            let path = self.path("remotes").join(repository);
            fs::create_dir_all(path.parent().expect("remote parent")).expect("remote parent");
            self.git(
                self.temp.path(),
                &["clone", "--bare", text(&self.path("seed")), text(&path)],
            );
            path
        }
        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .args(["--vcs", "git"])
                .output()
                .expect("repot")
        }
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 path")
    }
    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn bare_updates_preserve_private_refs_despite_prune_configuration_and_reject_rewrites_atomically()
     {
        let world = World::new();
        let remote = world.remote("team/project");
        success(&world.repot(&["get", "project", "--bare"]));
        let bare = world.destination("team/project.git");
        let initial = world.git(&bare, &["rev-parse", "main"]);
        world.git(&bare, &["update-ref", "refs/heads/private", &initial]);
        world.git(&bare, &["update-ref", "refs/tags/private", &initial]);
        world.git(
            world.temp.path(),
            &["config", "--global", "fetch.prune", "true"],
        );
        fs::write(world.path("seed/remote-update"), "remote advance\n").expect("remote file");
        world.git(&world.path("seed"), &["add", "."]);
        world.git(&world.path("seed"), &["commit", "-m", "remote advance"]);
        world.git(&world.path("seed"), &["push", text(&remote), "main"]);
        let before = world.git(&bare, &["show-ref"]);
        success(&world.repot(&["get", "project", "--bare", "--update", "--dry-run"]));
        assert_eq!(world.git(&bare, &["show-ref"]), before);
        success(&world.repot(&["get", "project", "--bare", "--update"]));
        assert_eq!(
            world.git(&bare, &["rev-parse", "refs/heads/private"]),
            initial
        );
        assert_eq!(
            world.git(&bare, &["rev-parse", "refs/tags/private"]),
            initial
        );
        assert_eq!(
            world.git(&bare, &["rev-parse", "main"]),
            world.git(&remote, &["rev-parse", "main"])
        );
        world.git(&remote, &["update-ref", "refs/heads/main", &initial]);
        world.git(&remote, &["update-ref", "refs/heads/new-branch", &initial]);
        let before = world.git(&bare, &["show-ref"]);
        let output = world.repot(&["get", "project", "--bare", "--update"]);
        assert!(
            !output.status.success(),
            "a non-fast-forward must be refused"
        );
        assert_eq!(
            world.git(&bare, &["show-ref"]),
            before,
            "atomic fetch must preserve all refs if one update fails"
        );
    }

    #[test]
    fn recursive_clone_populates_submodules_and_no_recursive_leaves_them_uninitialized() {
        let world = World::new();
        let dependency = world.remote("deps/library");
        world.git(
            &world.path("seed"),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "https://example.test/deps/library",
                "deps/library",
            ],
        );
        world.git(&world.path("seed"), &["commit", "-am", "add dependency"]);
        world.remote("team/recursive");
        world.remote("team/plain");
        success(&world.repot(&["get", "recursive"]));
        let module = world.destination("team/recursive/deps/library");
        assert!(module.join(".git").is_file());
        assert_eq!(
            world.git(&module, &["rev-parse", "HEAD"]),
            world.git(&dependency, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            fs::read_to_string(module.join("base")).expect("submodule file"),
            "initial\n"
        );
        success(&world.repot(&["get", "plain", "--no-recursive"]));
        let empty = world.destination("team/plain/deps/library");
        assert!(!empty.join(".git").exists());
        assert!(!empty.join("base").exists());
    }

    #[test]
    fn concurrent_clones_install_one_complete_repository_and_leave_no_staging_directories() {
        let world = World::new();
        let remote = world.remote("team/project");
        let mut children = Vec::new();
        for _ in 0..6 {
            children.push(
                world
                    .command(env!("CARGO_BIN_EXE_repot"))
                    .args(["get", "project", "--json", "--vcs", "git"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("concurrent clone"),
            );
        }
        let mut installed = 0usize;
        for child in children {
            let output = child.wait_with_output().expect("clone result");
            let results: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("clone reports");
            if results[0]["action"] == "cloned" {
                installed = installed.saturating_add(1);
            }
            assert!(matches!(
                results[0]["action"].as_str(),
                Some("cloned" | "exists" | "failed")
            ));
        }
        assert_eq!(installed, 1);
        let checkout = world.destination("team/project");
        assert_eq!(
            world.git(&checkout, &["rev-parse", "HEAD"]),
            world.git(&remote, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            fs::read_to_string(checkout.join("base")).expect("complete clone"),
            "initial\n"
        );
        let entries: Vec<_> = fs::read_dir(checkout.parent().expect("parent"))
            .expect("clone parent")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(entries, [std::ffi::OsString::from("project")]);
    }

    #[test]
    fn git_protocol_ssh_conversion_and_branch_suffix_preserve_the_requested_transport_and_branch() {
        let world = World::new();
        world.git(&world.path("seed"), &["switch", "-c", "feature/nested"]);
        fs::write(world.path("seed/feature"), "feature branch\n").expect("feature");
        world.git(&world.path("seed"), &["add", "."]);
        world.git(&world.path("seed"), &["commit", "-m", "feature"]);
        world.git(&world.path("seed"), &["switch", "main"]);
        world.remote("team/project");
        success(&world.repot(&[
            "get",
            "git://example.test/team/project@feature/nested",
            "-p",
        ]));
        let checkout = world.destination("team/project");
        assert_eq!(
            world.git(&checkout, &["branch", "--show-current"]),
            "feature/nested"
        );
        assert_eq!(
            world.git(&checkout, &["config", "--get", "remote.origin.url"]),
            "ssh://git@example.test/team/project"
        );
    }

    #[test]
    fn scp_inputs_normalize_origin_urls_including_absolute_remote_paths() {
        let world = World::new();
        for (name, input) in [
            ("team/relative", "git@example.test:team/relative"),
            ("team/absolute", "git@example.test:/team/absolute"),
        ] {
            world.remote(name);
            success(&world.repot(&["get", input]));
            let checkout = world.destination(name);
            assert_eq!(
                world.git(&checkout, &["config", "--get", "remote.origin.url"]),
                format!("ssh://git@example.test/{name}")
            );
            assert!(checkout.join("base").is_file());
        }
    }

    #[test]
    fn invalid_git_boolean_configuration_refuses_before_creating_any_checkout() {
        let world = World::new();
        world.git(
            world.temp.path(),
            &["config", "--global", "ghq.completeUser", "maybe"],
        );
        let output = world.repot(&["get", "project", "--dry-run"]);
        assert!(!output.status.success());
        assert!(!world.destination("team/project").exists());
        assert!(!world.destination("project/project").exists());
    }

    #[test]
    fn boolean_owner_configuration_and_bare_lookup_across_roots_follow_git_semantics() {
        let world = World::new();
        world.git(
            world.temp.path(),
            &["config", "--global", "ghq.completeUser", "FALSE"],
        );
        let preview = world.repot(&["get", "project", "--dry-run"]);
        success(&preview);
        assert!(String::from_utf8_lossy(&preview.stdout).contains("example.test/project/project"));
        world.git(
            world.temp.path(),
            &["config", "--global", "ghq.completeUser", "true"],
        );
        let remote = world.remote("team/project");
        let secondary = world.path("secondary/example.test/team/project.git");
        fs::create_dir_all(secondary.parent().expect("parent")).expect("parent");
        world.git(
            world.temp.path(),
            &["clone", "--bare", text(&remote), text(&secondary)],
        );
        let primary = world.path("primary/example.test/team/project");
        fs::create_dir_all(primary.parent().expect("parent")).expect("parent");
        world.git(world.temp.path(), &["clone", text(&remote), text(&primary)]);
        world.git(
            world.temp.path(),
            &[
                "config",
                "--global",
                "--add",
                "ghq.root",
                text(&world.path("secondary")),
            ],
        );
        world.git(
            world.temp.path(),
            &[
                "config",
                "--global",
                "--add",
                "ghq.root",
                text(&world.path("primary")),
            ],
        );
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env_remove("GHQ_ROOT")
            .args(["get", "project", "--bare", "--json", "--vcs", "git"])
            .output()
            .expect("bare multi-root lookup");
        success(&output);
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(report[0]["action"], "exists");
        assert_eq!(
            PathBuf::from(report[0]["path"].as_str().expect("path")),
            secondary.canonicalize().expect("bare clone")
        );
        assert!(!world.path("primary/example.test/team/project.git").exists());
    }
}
