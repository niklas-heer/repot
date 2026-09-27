//! Publication uses real isolated Git repositories and simulated forge CLIs.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    const URL: &str = "https://forge.example/owner/project";

    struct Fixture {
        temp: TempDir,
        git: PathBuf,
        forge: &'static str,
    }

    impl Fixture {
        fn new(forge: &'static str) -> Self {
            let git = env::split_paths(&env::var_os("PATH").expect("PATH"))
                .map(|path| path.join("git"))
                .find(|path| path.is_file())
                .expect("installed Git");
            let fixture = Self {
                temp: tempfile::tempdir().expect("isolated project"),
                git,
                forge,
            };
            fs::create_dir_all(fixture.path("bin")).expect("shim directory");
            fixture.git(
                fixture.temp.path(),
                &["init", "--initial-branch=main", "scratch"],
            );
            fs::write(fixture.path("scratch/work"), "precious project\n")
                .expect("project contents");
            fixture.git(&fixture.path("scratch"), &["add", "."]);
            fixture.git(&fixture.path("scratch"), &["commit", "-m", "initial"]);
            let remote = format!(
                "url.file://{}.insteadOf",
                fixture.path("remote.git").display()
            );
            fixture.git(
                fixture.temp.path(),
                &["config", "--global", "--add", &remote, URL],
            );
            fixture.git(
                fixture.temp.path(),
                &[
                    "config",
                    "--global",
                    "--add",
                    &remote,
                    &format!("{URL}.git"),
                ],
            );
            fixture.git(
                fixture.temp.path(),
                &[
                    "config",
                    "--global",
                    "--add",
                    &remote,
                    "git@forge.example:owner/project.git",
                ],
            );
            fixture.metadata("private", URL);
            for program in ["gh", "glab"] {
                let path = fixture.path(&format!("bin/{program}"));
                fs::write(&path, FORGE_SHIM).expect("forge shim");
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                    .expect("executable forge");
            }
            let path = fixture.path("bin/git");
            fs::write(&path, GIT_SHIM).expect("Git shim");
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable Git");
            fixture
        }

        fn path(&self, path: &str) -> PathBuf {
            self.temp.path().join(path)
        }

        fn command(&self, program: &Path, cwd: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(cwd)
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("projects"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Publish test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Publish test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("REPOT_CD_FILE")
                .env_remove("GH_TOKEN")
                .env_remove("GITHUB_TOKEN")
                .env_remove("GITLAB_TOKEN");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command(&self.git, cwd)
                .args(args)
                .output()
                .expect("Git runs");
            assert!(
                output.status.success(),
                "Git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("Git UTF-8")
                .trim()
                .into()
        }

        fn publish(&self) -> Command {
            self.publish_visibility("private")
        }

        fn publish_visibility(&self, visibility: &str) -> Command {
            let path = env::join_paths(
                std::iter::once(self.path("bin"))
                    .chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
            )
            .expect("test PATH");
            let mut command = self.command(
                Path::new(env!("CARGO_BIN_EXE_repot")),
                &self.path("scratch"),
            );
            command
                .args([
                    "publish",
                    "owner/project",
                    "--forge",
                    self.forge,
                    "--host",
                    "forge.example",
                    "--visibility",
                    visibility,
                ])
                .env("PATH", path)
                .env("REPOT_REAL_GIT", &self.git)
                .env("REPOT_FORGE_LOG", self.path("forge.log"))
                .env("REPOT_GIT_LOG", self.path("git.log"))
                .env("REPOT_TEST_REMOTE", self.path("remote.git"))
                .env("REPOT_FORGE_META", self.path("metadata.json"));
            command
        }

        fn metadata(&self, visibility: &str, url: &str) {
            let value = if self.forge == "github" {
                serde_json::json!({"nameWithOwner":"owner/project", "url":url, "sshUrl":"git@forge.example:owner/project.git", "visibility":visibility.to_ascii_uppercase()})
            } else {
                serde_json::json!({"path_with_namespace":"owner/project", "http_url_to_repo":url, "ssh_url_to_repo":"git@forge.example:owner/project.git", "visibility":visibility})
            };
            fs::write(self.path("metadata.json"), value.to_string()).expect("forge metadata");
        }

        fn destination(&self) -> PathBuf {
            self.path("projects/forge.example/owner/project")
        }

        fn log(&self, name: &str) -> String {
            fs::read_to_string(self.path(name)).unwrap_or_default()
        }

        fn assert_failed_before_create(&self, output: &Output) {
            assert!(!output.status.success());
            assert!(self.path("scratch/work").is_file());
            assert!(!self.path("remote.git").exists());
            assert!(!self.destination().exists());
            assert!(!self.log("forge.log").contains("repo create"));
        }
    }

    const FORGE_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$REPOT_FORGE_LOG"
case "$1 $2" in
  'auth status') exit 0 ;;
  'config get') printf '%s\n' "${REPOT_PROTOCOL:-https}"; exit 0 ;;
  'repo view') test -d "$REPOT_TEST_REMOTE" || exit 1; cat "$REPOT_FORGE_META"; exit 0 ;;
  'repo create')
    test -z "$REPOT_FAIL_CREATE" || exit 1
    test ! -e "$REPOT_TEST_REMOTE" || exit 1
    "$REPOT_REAL_GIT" init --bare --initial-branch=main "$REPOT_TEST_REMOTE" >/dev/null || exit 1
    test -z "$REPOT_FAIL_AFTER_CREATE" || exit 1
    exit 0 ;;
esac
exit 99
"#;

    const GIT_SHIM: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$REPOT_GIT_LOG"
for argument do
  if [ "$argument" = push ]; then
    test -z "$REPOT_FAIL_PUSH" || exit 1
    if [ -n "$REPOT_ADVANCE_ON_PUSH" ]; then
      "$REPOT_REAL_GIT" -c commit.gpgSign=false commit --allow-empty -m 'concurrent local progress' >/dev/null || exit 1
    fi
  fi
  if [ "$argument" = ls-remote ] && [ -n "$REPOT_WRONG_TIP" ]; then
    printf '%s\trefs/heads/main\n' 0000000000000000000000000000000000000000
    exit 0
  fi
done
exec "$REPOT_REAL_GIT" "$@"
"#;

    fn successful_publication(forge: &'static str) {
        let fixture = Fixture::new(forge);
        let head = fixture.git(&fixture.path("scratch"), &["rev-parse", "HEAD"]);
        fixture.git(&fixture.path("scratch"), &["tag", "private-local-tag"]);
        let output = fixture
            .publish()
            .env("REPOT_CD_FILE", fixture.path("handoff"))
            .output()
            .expect("publish runs");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!fixture.path("scratch").exists());
        assert_eq!(
            fs::read_to_string(fixture.destination().join("work")).expect("moved work"),
            "precious project\n"
        );
        assert_eq!(
            fixture.git(
                &fixture.path("remote.git"),
                &["rev-parse", "refs/heads/main"]
            ),
            head
        );
        assert!(
            fixture
                .git(&fixture.path("remote.git"), &["tag", "--list"])
                .is_empty()
        );
        assert_eq!(
            fixture.git(
                &fixture.destination(),
                &["rev-parse", "--abbrev-ref", "@{upstream}"]
            ),
            "origin/main"
        );
        assert_eq!(
            fs::read_to_string(fixture.path("handoff")).expect("handoff"),
            fixture.destination().to_string_lossy()
        );
        let forge_calls = fixture.log("forge.log");
        assert!(forge_calls.contains("repo create https://forge.example/owner/project --private"));
        assert!(!forge_calls.contains("--push"));
        if forge == "gitlab" {
            assert!(forge_calls.contains("--skipGitInit --defaultBranch main"));
        }
        let push = fixture
            .log("git.log")
            .lines()
            .find(|line| line.contains(" push "))
            .expect("push command")
            .to_owned();
        assert!(push.contains(&format!("{head}:refs/heads/main")));
        assert!(!push.contains("--force"));
    }

    #[test]
    fn github_publishes_only_inspected_branch_then_moves_checkout() {
        successful_publication("github");
    }

    #[test]
    fn gitlab_publishes_without_implicit_init_or_push() {
        successful_publication("gitlab");
    }

    #[test]
    fn dry_run_checks_authentication_without_creating_pushing_or_moving() {
        let fixture = Fixture::new("github");
        let before = fs::read(fixture.path("scratch/.git/config")).expect("Git config");
        let output = fixture
            .publish()
            .arg("--dry-run")
            .output()
            .expect("dry-run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            fixture
                .log("forge.log")
                .contains("auth status --hostname forge.example")
        );
        assert!(!fixture.log("forge.log").contains("repo create"));
        assert!(!fixture.log("git.log").contains(" push "));
        assert!(!fixture.path("remote.git").exists());
        assert!(!fixture.path("projects").exists());
        assert_eq!(
            fs::read(fixture.path("scratch/.git/config")).expect("unchanged config"),
            before
        );
    }

    #[test]
    fn dirty_and_detached_checkouts_are_rejected_before_forge_creation() {
        let fixture = Fixture::new("github");
        fs::write(fixture.path("scratch/untracked"), "local changes").expect("local changes");
        fixture.assert_failed_before_create(&fixture.publish().output().expect("dirty publish"));
        fs::remove_file(fixture.path("scratch/untracked")).expect("remove test change");
        fixture.git(&fixture.path("scratch"), &["switch", "--detach"]);
        fixture.assert_failed_before_create(&fixture.publish().output().expect("detached publish"));
    }

    #[test]
    fn existing_destination_prevents_remote_creation() {
        let fixture = Fixture::new("github");
        fs::create_dir_all(fixture.destination()).expect("existing checkout destination");
        fs::write(fixture.destination().join("keep"), "existing").expect("existing file");
        let output = fixture.publish().output().expect("publish runs");
        assert!(!output.status.success());
        assert!(fixture.log("forge.log").is_empty());
        assert_eq!(
            fs::read_to_string(fixture.destination().join("keep")).expect("existing file"),
            "existing"
        );
    }

    #[test]
    fn failed_create_can_be_retried_without_local_changes() {
        let fixture = Fixture::new("github");
        let before = fs::read(fixture.path("scratch/.git/config")).expect("Git config");
        for _ in 0..2 {
            let output = fixture
                .publish()
                .env("REPOT_FAIL_CREATE", "1")
                .output()
                .expect("failed create");
            assert!(!output.status.success());
            assert!(fixture.path("scratch/work").is_file());
            assert!(!fixture.path("remote.git").exists());
            assert!(!fixture.destination().exists());
            assert_eq!(
                fs::read(fixture.path("scratch/.git/config")).expect("unchanged Git config"),
                before
            );
        }
    }

    #[test]
    fn failed_push_retains_checkout_and_matching_origin_for_resume() {
        let fixture = Fixture::new("github");
        let first = fixture
            .publish()
            .env("REPOT_FAIL_PUSH", "1")
            .output()
            .expect("failed push");
        assert!(!first.status.success());
        assert!(fixture.path("scratch/work").is_file());
        assert_eq!(
            fixture.git(&fixture.path("scratch"), &["config", "remote.origin.url"]),
            URL
        );
        let second = fixture.publish().arg("--resume").output().expect("resume");
        assert!(
            second.status.success(),
            "{}",
            String::from_utf8_lossy(&second.stderr)
        );
        assert!(fixture.destination().join("work").is_file());
        assert_eq!(
            fixture
                .log("forge.log")
                .lines()
                .filter(|line| line.contains("repo create"))
                .count(),
            1
        );
    }

    #[test]
    fn resume_refuses_changed_visibility_and_mismatched_origins() {
        let fixture = Fixture::new("github");
        let first = fixture
            .publish()
            .env("REPOT_FAIL_PUSH", "1")
            .output()
            .expect("failed push");
        assert!(!first.status.success());
        fixture.metadata("public", URL);
        let second = fixture
            .publish()
            .args(["--resume", "--dry-run"])
            .output()
            .expect("resume visibility check");
        assert!(!second.status.success());
        assert!(String::from_utf8_lossy(&second.stderr).contains("visibility"));
        fixture.git(
            &fixture.path("scratch"),
            &[
                "remote",
                "set-url",
                "origin",
                "https://other.example/owner/project",
            ],
        );
        let third = fixture
            .publish()
            .arg("--resume")
            .output()
            .expect("origin check");
        assert!(!third.status.success());
        assert!(String::from_utf8_lossy(&third.stderr).contains("origin"));
        assert!(!fixture.destination().exists());
    }

    #[test]
    fn mismatched_remote_tip_prevents_move_after_push() {
        let fixture = Fixture::new("github");
        let output = fixture
            .publish()
            .env("REPOT_WRONG_TIP", "1")
            .output()
            .expect("tip verification");
        assert!(!output.status.success());
        assert!(fixture.path("scratch/work").is_file());
        assert!(!fixture.destination().exists());
    }

    #[test]
    fn credential_bearing_forge_url_is_rejected_without_printing_credentials() {
        let fixture = Fixture::new("github");
        fixture.metadata(
            "private",
            "https://user:secret-marker@forge.example/owner/project",
        );
        let output = fixture.publish().output().expect("metadata validation");
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-marker"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-marker"));
        assert!(
            fixture
                .git(&fixture.path("scratch"), &["remote"])
                .is_empty()
        );
    }

    #[test]
    fn hidden_index_flags_block_publish_even_if_porcelain_looks_clean() {
        for flag in ["--assume-unchanged", "--skip-worktree"] {
            let fixture = Fixture::new("github");
            fixture.git(&fixture.path("scratch"), &["update-index", flag, "work"]);
            fs::write(fixture.path("scratch/work"), "hidden local work\n")
                .expect("hidden local work");
            assert!(
                fixture
                    .git(&fixture.path("scratch"), &["status", "--porcelain"])
                    .is_empty()
            );
            fixture.assert_failed_before_create(
                &fixture.publish().output().expect("hidden flag check"),
            );
        }
    }

    #[test]
    fn concurrent_commit_during_push_cannot_publish_uninspected_work_or_move_checkout() {
        let fixture = Fixture::new("github");
        let original = fixture.git(&fixture.path("scratch"), &["rev-parse", "HEAD"]);
        let output = fixture
            .publish()
            .env("REPOT_ADVANCE_ON_PUSH", "1")
            .output()
            .expect("concurrent publication");
        assert!(!output.status.success());
        assert!(!fixture.destination().exists());
        assert_eq!(
            fixture.git(
                &fixture.path("remote.git"),
                &["rev-parse", "refs/heads/main"]
            ),
            original
        );
        assert_ne!(
            fixture.git(&fixture.path("scratch"), &["rev-parse", "HEAD"]),
            original
        );
        let resumed = fixture
            .publish()
            .arg("--resume")
            .output()
            .expect("resume inspected progress");
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }

    #[test]
    fn server_side_creation_with_lost_response_remains_explicitly_recoverable() {
        let fixture = Fixture::new("github");
        let first = fixture
            .publish()
            .env("REPOT_FAIL_AFTER_CREATE", "1")
            .output()
            .expect("lost creation response");
        assert!(!first.status.success());
        assert!(fixture.path("remote.git").is_dir());
        assert!(
            fixture
                .git(&fixture.path("scratch"), &["remote"])
                .is_empty()
        );
        let retry = fixture.publish().output().expect("ordinary retry");
        assert!(!retry.status.success());
        fixture.git(&fixture.path("scratch"), &["remote", "add", "origin", URL]);
        let resumed = fixture
            .publish()
            .arg("--resume")
            .output()
            .expect("explicit recovery");
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
        assert_eq!(
            fixture
                .log("forge.log")
                .lines()
                .filter(|line| line.contains("repo create"))
                .count(),
            1
        );
    }

    #[test]
    fn resume_never_overwrites_non_fast_forward_remote_history() {
        let fixture = Fixture::new("github");
        let first = fixture
            .publish()
            .env("REPOT_FAIL_PUSH", "1")
            .output()
            .expect("first push failure");
        assert!(!first.status.success());
        fixture.git(fixture.temp.path(), &["clone", "remote.git", "competitor"]);
        fixture.git(
            &fixture.path("competitor"),
            &[
                "commit",
                "--allow-empty",
                "-m",
                "independent remote history",
            ],
        );
        fixture.git(&fixture.path("competitor"), &["push", "origin", "main"]);
        let remote_head = fixture.git(
            &fixture.path("remote.git"),
            &["rev-parse", "refs/heads/main"],
        );
        let resumed = fixture
            .publish()
            .arg("--resume")
            .output()
            .expect("non-fast-forward resume");
        assert!(!resumed.status.success());
        assert!(fixture.path("scratch/work").is_file());
        assert_eq!(
            fixture.git(
                &fixture.path("remote.git"),
                &["rev-parse", "refs/heads/main"]
            ),
            remote_head
        );
    }

    #[test]
    fn explicit_public_visibility_and_configured_ssh_transport_are_respected() {
        let fixture = Fixture::new("github");
        fixture.metadata("public", URL);
        let output = fixture
            .publish_visibility("public")
            .env("REPOT_PROTOCOL", "ssh")
            .output()
            .expect("public SSH publication");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            fixture
                .log("forge.log")
                .contains("repo create https://forge.example/owner/project --public")
        );
        assert_eq!(
            fixture.git(&fixture.destination(), &["config", "remote.origin.url"]),
            "git@forge.example:owner/project.git"
        );
    }
}
