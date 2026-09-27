//! `repot doctor` against temporary trees, with a scripted `gh` standing in for
//! GitHub so renames, transfers, archives and deletions are deterministic.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use serde_json::Value;
    use tempfile::TempDir;

    struct World {
        home: TempDir,
    }

    impl World {
        fn new() -> Self {
            let world = Self {
                home: tempfile::tempdir().expect("temporary home"),
            };
            fs::create_dir_all(world.path("bin")).expect("fake tools");
            fs::create_dir_all(world.path("root")).expect("root");
            // GitHub as seen through gh: renames and transfers resolve to the
            // new name, unknown repositories answer 404.
            let gh = r#"#!/bin/sh
[ "$1" = "--version" ] && { echo "gh version 0.0.0-test"; exit 0; }
echo "$*" >> "$GH_LOG"
case "$2" in
  repos/owner/old) printf 'owner/new\tfalse\n' ;;
  repos/owner/new) printf 'owner/new\tfalse\n' ;;
  repos/owner/moved) printf 'elsewhere/moved\tfalse\n' ;;
  repos/owner/retired) printf 'owner/retired\ttrue\n' ;;
  repos/Owner/Casing) printf 'owner/casing\tfalse\n' ;;
  *) printf '{"message":"Not Found","status":"404"}'; exit 1 ;;
esac
"#;
            let script = world.path("bin/gh");
            fs::write(&script, gh).expect("fake gh");
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
            world
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.home.path().join(relative)
        }

        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            let path = std::env::join_paths(std::iter::once(self.path("bin")).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .expect("PATH");
            command
                .current_dir(self.home.path())
                .env("PATH", path)
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("root"))
                .env("GH_LOG", self.path("gh.log"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) {
            let output = self
                .command("git")
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

        /// A checkout at `relative` under the root whose origin is `remote`.
        fn checkout(&self, relative: &str, remote: &str) -> PathBuf {
            let path = self.path("root").join(relative);
            fs::create_dir_all(&path).expect("checkout directory");
            self.git(&path, &["init", "--quiet", "--initial-branch=main"]);
            self.git(&path, &["commit", "--quiet", "--allow-empty", "-m", "init"]);
            self.git(&path, &["remote", "add", "origin", remote]);
            // A remote-tracking ref makes the commit count as pushed.
            self.git(&path, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
            path.canonicalize().expect("canonical checkout")
        }

        fn doctor(&self, args: &[&str]) -> (Output, Vec<Value>) {
            let output = self
                .command(env!("CARGO_BIN_EXE_repot"))
                .arg("doctor")
                .args(args)
                .output()
                .expect("repot runs");
            let findings = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!(
                    "JSON: {error}; stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            });
            (output, findings)
        }
    }

    fn kinds(findings: &[Value]) -> Vec<(String, String)> {
        findings
            .iter()
            .map(|finding| {
                (
                    finding["kind"].as_str().expect("kind").to_owned(),
                    Path::new(finding["path"].as_str().expect("path"))
                        .file_name()
                        .expect("name")
                        .to_string_lossy()
                        .into_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn healthy_trees_report_nothing_and_exit_zero() {
        let world = World::new();
        world.checkout("github.com/owner/new", "https://github.com/owner/new.git");
        world.checkout("github.com/Owner/Casing", "git@github.com:Owner/Casing.git");
        let (output, findings) = world.doctor(&["--json"]);
        assert_eq!(output.status.code(), Some(0));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn renames_duplicates_archives_and_misplaced_checkouts_come_with_fixes() {
        let world = World::new();
        let stale = world.checkout("github.com/owner/old", "https://github.com/owner/old.git");
        world.checkout("github.com/owner/new", "https://github.com/owner/new.git");
        world.checkout("github.com/owner/moved", "git@github.com:owner/moved.git");
        world.checkout(
            "github.com/owner/retired",
            "https://github.com/owner/retired.git",
        );
        world.checkout(
            "github.com/owner/deleted",
            "https://github.com/owner/deleted.git",
        );
        world.checkout(
            "somewhere/else/misfiled",
            "https://example.com/team/misfiled.git",
        );
        fs::write(stale.join("notes"), "unsaved").expect("local work");
        let before = fs::read_dir(world.path("root/github.com/owner"))
            .expect("tree")
            .count();

        let (output, findings) = world.doctor(&["--json"]);
        assert_eq!(output.status.code(), Some(3), "findings need attention");
        assert_eq!(
            kinds(&findings),
            [
                ("duplicate".into(), "old".into()),
                ("renamed".into(), "moved".into()),
                ("misplaced".into(), "misfiled".into()),
                ("missing".into(), "deleted".into()),
                ("archived".into(), "retired".into()),
            ]
        );
        let duplicate = &findings[0];
        assert!(
            duplicate["fix"]
                .as_str()
                .expect("fix")
                .starts_with("repot rm ")
        );
        assert_eq!(duplicate["local_work"], "1 changed file");
        let renamed = findings[1]["fix"].as_str().expect("fix");
        assert!(renamed.contains("remote set-url origin git@github.com:elsewhere/moved.git"));
        assert!(renamed.contains("repot adopt "));
        assert!(
            findings[2]["related"]
                .as_str()
                .expect("related")
                .ends_with("example.com/team/misfiled")
        );

        // Report only: nothing moved, removed or reconfigured.
        assert_eq!(
            fs::read_dir(world.path("root/github.com/owner"))
                .expect("tree")
                .count(),
            before
        );
        assert!(stale.join("notes").exists());
    }

    #[test]
    fn offline_never_asks_github_and_still_finds_local_problems() {
        let world = World::new();
        world.checkout("github.com/owner/old", "https://github.com/owner/old.git");
        world.checkout("github.com/owner/copy", "https://github.com/owner/old.git");
        let (output, findings) = world.doctor(&["--json", "--offline"]);
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(kinds(&findings), [("duplicate".into(), "copy".into())]);
        assert!(!world.path("gh.log").exists(), "gh was called offline");

        let text = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["doctor", "--offline"])
            .output()
            .expect("repot runs");
        let text = String::from_utf8_lossy(&text.stdout);
        assert!(text.contains("Duplicates"));
        assert!(text.contains("same repository as owner/old"));
        assert!(text.contains("repot trash restores it"));
    }

    #[test]
    fn fix_applies_safe_fixes_and_leaves_unique_work_alone() {
        let world = World::new();
        let stale = world.checkout("github.com/owner/old", "https://github.com/owner/old.git");
        let kept = world.checkout("github.com/owner/new", "https://github.com/owner/new.git");
        let moved = world.checkout("github.com/owner/moved", "git@github.com:owner/moved.git");
        let retired = world.checkout(
            "github.com/owner/retired",
            "https://github.com/owner/retired.git",
        );
        fs::write(retired.join("notes"), "only here").expect("unique work");

        let without_terminal = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["doctor", "--fix"])
            .output()
            .expect("repot runs");
        assert!(!without_terminal.status.success());
        assert!(String::from_utf8_lossy(&without_terminal.stderr).contains("--yes"));
        assert!(stale.exists(), "nothing changes without confirmation");

        let fixed = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["doctor", "--fix", "--yes"])
            .output()
            .expect("repot runs");
        let stderr = String::from_utf8_lossy(&fixed.stderr);
        assert_eq!(
            fixed.status.code(),
            Some(3),
            "one finding was left: {stderr}"
        );
        assert!(stderr.contains("skipped"), "{stderr}");
        // The stale duplicate is archived, not deleted.
        assert!(!stale.exists());
        let trash = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .args(["trash", "list", "--json"])
            .output()
            .expect("trash");
        assert!(String::from_utf8_lossy(&trash.stdout).contains("owner/old"));
        assert!(kept.exists());
        // The renamed checkout now tracks the new name and lives there.
        assert!(!moved.exists());
        let relocated = world.path("root/github.com/elsewhere/moved");
        let url = world
            .command("git")
            .current_dir(&relocated)
            .args(["remote", "get-url", "origin"])
            .output()
            .expect("git");
        assert_eq!(
            String::from_utf8_lossy(&url.stdout).trim(),
            "git@github.com:elsewhere/moved.git"
        );
        // Unique work is never touched unattended.
        assert!(retired.join("notes").exists());
    }
}
