//! Git-focused ghq list and root compatibility through the public binary.

#[cfg(test)]
mod tests {
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
        fn path(&self, name: &str) -> PathBuf {
            self.temp.path().join(name)
        }
        fn command(&self, binary: &str) -> Command {
            let mut command = Command::new(binary);
            command
                .current_dir(self.temp.path())
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("GHQ_ROOT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_CONFIG_COUNT");
            command
        }
        fn git(&self, args: &[&str]) {
            let output = self.command("git").args(args).output().expect("Git");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fn init(&self, relative: &str, bare: bool) {
            let path = self.path(relative);
            let mut args = vec!["init", "--quiet", "--initial-branch=main"];
            if bare {
                args.push("--bare");
            }
            args.push(text(&path));
            self.git(&args);
        }
        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot")
        }
        fn lines(&self, args: &[&str]) -> Vec<String> {
            lines(self.repot(args))
        }
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 path")
    }
    fn lines(output: Output) -> Vec<String> {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("text output")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn relative_full_json_and_smartcase_queries_follow_ghq_rules() {
        let world = World::new();
        for repo in [
            "github.com/Team/Project",
            "github.com/owner/project-other",
            "gitlab.com/owner/Project",
        ] {
            world.init(&format!("ghq/{repo}"), false);
        }
        assert_eq!(
            world.lines(&["list"]),
            [
                "github.com/Team/Project",
                "github.com/owner/project-other",
                "gitlab.com/owner/Project"
            ]
        );
        assert_eq!(world.lines(&["list", "project"]).len(), 3);
        assert_eq!(world.lines(&["list", "Project"]).len(), 2);
        assert_eq!(world.lines(&["list", "github.com/project"]).len(), 2);
        assert!(
            world.lines(&["list", "github.com"]).is_empty(),
            "plain query searches the non-host path"
        );
        assert_eq!(
            world.lines(&["list", "-e", "Team/Project"]),
            ["github.com/Team/Project"]
        );
        assert!(world.lines(&["list", "-e", "team/project"]).is_empty());
        let full = world.lines(&["list", "-p", "-e", "Team/Project"]);
        assert_eq!(
            full,
            [world
                .path("ghq/github.com/Team/Project")
                .canonicalize()
                .expect("repo")
                .to_string_lossy()]
        );
        let json = world.repot(&["list", "--json", "-e", "Team/Project"]);
        assert!(json.status.success());
        let values: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON");
        assert_eq!(values[0]["path"].as_str(), full.first().map(String::as_str));
        assert_eq!(
            values[0].as_object().expect("entry").len(),
            1,
            "JSON schema remains path-only"
        );
    }

    #[test]
    fn url_queries_normalize_ssh_and_bare_flags_without_filtering_ordinary_lists() {
        let world = World::new();
        world.init("ghq/github.com/owner/project", false);
        world.init("ghq/github.com/owner/project.git", true);
        assert_eq!(world.lines(&["list", "--bare"]).len(), 2);
        assert_eq!(world.lines(&["list"]).len(), 2);
        assert_eq!(
            world.lines(&["list", "-e", "git@github.com:owner/project.git"]),
            ["github.com/owner/project"]
        );
        assert_eq!(
            world.lines(&["list", "--bare", "-e", "https://github.com/owner/project"]),
            ["github.com/owner/project.git"]
        );
        assert_eq!(world.lines(&["list", "--vcs", "github"]).len(), 2);
        let unsupported = world.repot(&["list", "--vcs", "mercurial"]);
        assert!(!unsupported.status.success());
        assert!(String::from_utf8_lossy(&unsupported.stderr).contains("Git"));
    }

    #[test]
    fn unique_uses_shortest_unambiguous_suffix_and_deduplicates_roots() {
        let world = World::new();
        world.git(&["config", "--global", "--add", "ghq.root", "~/secondary"]);
        world.git(&["config", "--global", "--add", "ghq.root", "~/primary"]);
        for repo in [
            "primary/github.com/alice/project",
            "primary/github.com/bob/project",
            "primary/github.com/alice/alone",
            "secondary/github.com/alice/project",
        ] {
            world.init(repo, false);
        }
        assert_eq!(world.lines(&["list"]).len(), 4);
        assert_eq!(
            world.lines(&["list", "--unique"]),
            ["alice/project", "alone", "bob/project"]
        );
        assert_eq!(
            world.lines(&["list", "--unique", "-p"]),
            ["alice/project", "alone", "bob/project"]
        );
        let json = world.repot(&["list", "--unique", "--json", "-e", "alice/project"]);
        assert!(json.status.success());
        let values: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON");
        assert_eq!(values.as_array().expect("entries").len(), 1);
        assert_eq!(
            PathBuf::from(values[0]["path"].as_str().expect("path")),
            world
                .path("primary/github.com/alice/project")
                .canonicalize()
                .expect("primary repo")
        );
    }

    #[test]
    fn root_reports_primary_first_and_includes_url_scoped_roots() {
        let world = World::new();
        world.git(&["config", "--global", "--add", "ghq.root", "~/secondary"]);
        world.git(&["config", "--global", "--add", "ghq.root", "~/primary"]);
        world.git(&[
            "config",
            "--global",
            "ghq.https://work.example/.root",
            "~/work",
        ]);
        assert_eq!(world.lines(&["root"]), [text(&world.path("primary"))]);
        assert_eq!(
            world.lines(&["root", "--all"]),
            [
                text(&world.path("primary")),
                text(&world.path("secondary")),
                text(&world.path("work"))
            ]
        );
        world.init("work/work.example/team/project", false);
        assert_eq!(world.lines(&["list"]), ["work.example/team/project"]);
    }

    #[test]
    fn environment_path_list_overrides_config_and_preserves_first_root_priority() {
        let world = World::new();
        world.git(&["config", "--global", "ghq.root", "~/ignored"]);
        world.git(&[
            "config",
            "--global",
            "ghq.https://work.example/.root",
            "~/also-ignored",
        ]);
        let configured = std::env::join_paths([
            world.path("first"),
            world.path("second"),
            world.path("first"),
        ])
        .expect("path list");
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("GHQ_ROOT", configured)
            .args(["root", "--all"])
            .output()
            .expect("root list");
        assert_eq!(
            lines(output),
            [text(&world.path("first")), text(&world.path("second"))]
        );
    }

    #[test]
    fn url_specific_destination_is_used_for_adoption_and_discovery() {
        let world = World::new();
        world.git(&["config", "--global", "ghq.root", "~/ordinary"]);
        world.git(&[
            "config",
            "--global",
            "ghq.https://gitlab.example/groups/.root",
            "~/work",
        ]);
        world.init("stray", false);
        world.git(&[
            "-C",
            text(&world.path("stray")),
            "remote",
            "add",
            "origin",
            "https://gitlab.example/groups/team/project.git",
        ]);
        let output = world.repot(&["adopt", "stray", "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            world
                .path("work/gitlab.example/groups/team/project/.git")
                .is_dir()
        );
        assert!(!world.path("ordinary").exists());
        assert_eq!(
            world.lines(&["list"]),
            ["gitlab.example/groups/team/project"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn configured_root_symlinks_are_canonicalized_and_deduplicated() {
        let world = World::new();
        world.init("physical/github.com/owner/project", false);
        std::os::unix::fs::symlink(world.path("physical"), world.path("alias")).expect("root link");
        world.git(&["config", "--global", "--add", "ghq.root", "~/physical"]);
        world.git(&["config", "--global", "--add", "ghq.root", "~/alias"]);
        assert_eq!(
            world.lines(&["root", "--all"]),
            [text(&world.path("physical").canonicalize().expect("root"))]
        );
        assert_eq!(world.lines(&["list"]), ["github.com/owner/project"]);
        fs::create_dir_all(world.path("physical/not-a-repo.git")).expect("bare-looking directory");
        assert_eq!(
            world.lines(&["list"]).len(),
            1,
            "bare detection requires actual Git metadata"
        );
    }
}
