//! Adoption and restoration preserve user work under races and unusual layouts.

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::env;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    use tempfile::TempDir;

    struct Fixture {
        temp: TempDir,
        git: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let git = env::split_paths(&env::var_os("PATH").expect("PATH"))
                .map(|path| path.join("git"))
                .find(|path| path.is_file())
                .expect("Git installed");
            let fixture = Self {
                temp: tempfile::tempdir().expect("isolated fixture"),
                git,
            };
            fs::create_dir_all(fixture.path("config/repot")).expect("manifest directory");
            fs::write(fixture.manifest(), "[settings]\nowners = [\"existing\"]\n")
                .expect("initial manifest");
            fixture
        }

        fn path(&self, path: &str) -> PathBuf {
            self.temp.path().join(path)
        }

        fn manifest(&self) -> PathBuf {
            self.path("config/repot/repos.toml")
        }

        fn command(&self, program: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.temp.path())
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("projects"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "Safety test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Safety test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command(&self.git)
                .current_dir(cwd)
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

        fn repo(&self, name: &str) -> PathBuf {
            self.git(self.temp.path(), &["init", "--initial-branch=main", name]);
            let path = self.path(name);
            fs::write(path.join("work"), "original\n").expect("tracked work");
            self.git(&path, &["add", "."]);
            self.git(&path, &["commit", "-m", "initial"]);
            self.git(
                &path,
                &[
                    "remote",
                    "add",
                    "origin",
                    &format!("https://forge.example/owner/{name}"),
                ],
            );
            path
        }

        fn repot(&self) -> Command {
            self.command(Path::new(env!("CARGO_BIN_EXE_repot")))
        }

        fn adopt(&self, path: &Path, register: bool) -> Output {
            let mut command = self.repot();
            command.arg("adopt").arg(path).arg("--json");
            if register {
                command.arg("--register");
            }
            command.output().expect("adopt runs")
        }

        fn manifest_data(&self) -> toml::Value {
            toml::from_str(&fs::read_to_string(self.manifest()).expect("manifest file"))
                .expect("manifest TOML")
        }

        fn listed_paths(&self) -> Vec<PathBuf> {
            let output = self
                .repot()
                .args(["list", "--json"])
                .output()
                .expect("list registered paths");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let rows: serde_json::Value =
                serde_json::from_slice(&output.stdout).expect("list JSON");
            rows.as_array()
                .expect("repository list")
                .iter()
                .map(|entry| PathBuf::from(entry["path"].as_str().expect("repository path")))
                .collect()
        }
    }

    #[test]
    fn linked_worktree_can_register_in_place_but_cannot_move() {
        let fixture = Fixture::new();
        let root = fixture.repo("main-checkout");
        fixture.git(&root, &["worktree", "add", "-b", "topic", "../linked"]);
        let linked = fixture.path("linked");
        let before = fs::read(linked.join(".git")).expect("worktree link");
        let refused = fixture.adopt(&linked, false);
        assert!(!refused.status.success());
        assert_eq!(
            fs::read(linked.join(".git")).expect("unchanged worktree link"),
            before
        );
        let registered = fixture.adopt(&linked, true);
        assert!(
            registered.status.success(),
            "{}",
            String::from_utf8_lossy(&registered.stderr)
        );
        assert!(linked.join("work").exists());
        assert_eq!(
            fixture.git(&linked, &["symbolic-ref", "--short", "HEAD"]),
            "topic"
        );
        let manifest = fixture.manifest_data();
        let entries = manifest["repo"].as_array().expect("registered entries");
        assert_eq!(entries.len(), 1);
        assert!(
            fixture
                .listed_paths()
                .contains(&linked.canonicalize().expect("canonical linked path"))
        );
    }

    #[test]
    fn dirty_adoption_preserves_index_files_symlinks_and_commit_exactly() {
        let fixture = Fixture::new();
        let source = fixture.repo("dirty");
        fs::write(source.join("work"), "staged change\n").expect("staged edit");
        fixture.git(&source, &["add", "work"]);
        fs::write(source.join("work"), "unstaged change\n").expect("unstaged edit");
        fs::create_dir(source.join("untracked")).expect("untracked directory");
        fs::write(source.join("untracked/binary"), [0, 255, 128, 10]).expect("untracked bytes");
        symlink("work", source.join("untracked/link")).expect("untracked symlink");
        let index = fs::read(source.join(".git/index")).expect("Git index");
        let head = fixture.git(&source, &["rev-parse", "HEAD"]);
        let config = fs::read(source.join(".git/config")).expect("Git config");
        let output = fixture.adopt(&source, false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let destination = fixture.path("projects/forge.example/owner/dirty");
        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join(".git/index")).expect("moved index"),
            index
        );
        assert_eq!(
            fs::read(destination.join(".git/config")).expect("moved config"),
            config
        );
        assert_eq!(fixture.git(&destination, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read(destination.join("work")).expect("moved unstaged edit"),
            b"unstaged change\n"
        );
        assert_eq!(
            fs::read(destination.join("untracked/binary")).expect("moved binary"),
            [0, 255, 128, 10]
        );
        assert_eq!(
            fs::read_link(destination.join("untracked/link")).expect("moved symlink"),
            Path::new("work")
        );
    }

    #[test]
    fn registering_through_dotfiles_symlink_preserves_link_and_other_settings() {
        let fixture = Fixture::new();
        let source = fixture.repo("registered");
        fs::create_dir(fixture.path("dotfiles")).expect("dotfiles directory");
        let target = fixture.path("dotfiles/repos.toml");
        fs::rename(fixture.manifest(), &target).expect("move tracked manifest");
        symlink("../../dotfiles/repos.toml", fixture.manifest()).expect("dotfiles symlink");
        let output = fixture.adopt(&source, true);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_link(fixture.manifest()).expect("preserved symlink"),
            Path::new("../../dotfiles/repos.toml")
        );
        let data = fixture.manifest_data();
        assert_eq!(data["settings"]["owners"][0].as_str(), Some("existing"));
        assert_eq!(
            data["repo"].as_array().expect("registered entries").len(),
            1
        );
        assert_eq!(
            fs::read(&target).expect("updated dotfiles source"),
            fs::read(fixture.manifest()).expect("linked manifest")
        );
    }

    #[test]
    fn concurrent_registrations_preserve_every_successful_entry() {
        let fixture = Fixture::new();
        let sources: Vec<_> = (0..8)
            .map(|index| fixture.repo(&format!("parallel-{index}")))
            .collect();
        let children: Vec<_> = sources
            .iter()
            .map(|source| {
                fixture
                    .repot()
                    .arg("adopt")
                    .arg(source)
                    .args(["--register", "--json"])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("concurrent registration")
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().expect("registration completion");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let data = fixture.manifest_data();
        let entries = data["repo"].as_array().expect("registered repositories");
        assert_eq!(entries.len(), sources.len());
        let listed = fixture.listed_paths();
        for source in sources {
            let canonical = source.canonicalize().expect("source path");
            assert_eq!(listed.iter().filter(|path| **path == canonical).count(), 1);
        }
        assert_eq!(data["settings"]["owners"][0].as_str(), Some("existing"));
    }

    #[test]
    fn unsafe_remote_values_are_rejected_without_leaking_them() {
        for remote in [
            "https://user:credential-marker@forge.example/owner/unsafe",
            "ext::sh -c credential-marker",
            "file:///credential-marker/owner/unsafe",
        ] {
            let fixture = Fixture::new();
            let source = fixture.repo("unsafe");
            fixture.git(&source, &["remote", "set-url", "origin", remote]);
            let before = fs::read(fixture.manifest()).expect("original manifest");
            let output = fixture.adopt(&source, false);
            assert!(!output.status.success());
            assert!(!String::from_utf8_lossy(&output.stdout).contains("credential-marker"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("credential-marker"));
            assert!(source.join("work").exists());
            assert_eq!(
                fs::read(fixture.manifest()).expect("unchanged manifest"),
                before
            );
        }
    }

    #[test]
    fn restore_never_overwrites_a_destination_created_while_cloning() {
        let fixture = Fixture::new();
        fixture.repo("seed");
        fixture.git(
            fixture.temp.path(),
            &["clone", "--bare", "seed", "remote.git"],
        );
        let destination = fixture.path("projects/forge.example/owner/racing");
        fs::write(
            fixture.manifest(),
            "[[repo]]\nurl = 'https://forge.example/owner/racing'\n",
        )
        .expect("restore manifest");
        fixture.git(
            fixture.temp.path(),
            &[
                "config",
                "--global",
                &format!(
                    "url.file://{}.insteadOf",
                    fixture.path("remote.git").display()
                ),
                "https://forge.example/owner/racing",
            ],
        );
        fs::create_dir(fixture.path("bin")).expect("shim directory");
        let stub = fixture.path("bin/git");
        fs::write(
            &stub,
            r#"#!/bin/sh
for argument do
  if [ "$argument" = clone ]; then
    mkdir -p "$REPOT_RACE_DEST" || exit 1
    printf '%s' 'other process work' > "$REPOT_RACE_DEST/keep"
  fi
done
exec "$REPOT_REAL_GIT" "$@"
"#,
        )
        .expect("clone race shim");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("executable shim");
        let path = env::join_paths(
            std::iter::once(fixture.path("bin"))
                .chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
        )
        .expect("test PATH");
        let output = fixture
            .repot()
            .args(["restore", "--json"])
            .env("PATH", path)
            .env("REPOT_REAL_GIT", &fixture.git)
            .env("REPOT_RACE_DEST", &destination)
            .output()
            .expect("restore race");
        assert!(!output.status.success());
        assert_eq!(
            fs::read(destination.join("keep")).expect("other process work"),
            b"other process work"
        );
        assert!(!destination.join(".git").exists());
        assert!(!destination.join("work").exists());
    }

    #[test]
    fn restore_rejects_credential_bearing_or_executable_urls_before_git_runs() {
        for remote in [
            "https://user:credential-marker@forge.example/owner/unsafe",
            "ext::sh -c credential-marker",
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.manifest(), format!("[[repo]]\nurl = '{remote}'\n"))
                .expect("unsafe entry");
            fs::create_dir(fixture.path("bin")).expect("shim directory");
            let stub = fixture.path("bin/git");
            fs::write(
                &stub,
                "#!/bin/sh\nprintf '%s' invoked > \"$REPOT_GIT_CALLED\"\nexit 99\n",
            )
            .expect("Git guard");
            fs::set_permissions(stub, fs::Permissions::from_mode(0o755)).expect("executable guard");
            let output = fixture
                .repot()
                .args(["restore", "--json", "--timeout", "1"])
                .env("PATH", fixture.path("bin"))
                .env("REPOT_GIT_CALLED", fixture.path("git-called"))
                .output()
                .expect("unsafe restore");
            assert!(!output.status.success());
            assert!(!String::from_utf8_lossy(&output.stdout).contains("credential-marker"));
            assert!(!String::from_utf8_lossy(&output.stderr).contains("credential-marker"));
            assert!(!fixture.path("git-called").exists());
            assert!(!fixture.path("projects").exists());
        }
    }

    #[test]
    fn externally_dependent_git_metadata_can_register_but_cannot_move() {
        for layout in ["objects", "commondir"] {
            let fixture = Fixture::new();
            let source = fixture.repo("dependent");
            if layout == "objects" {
                fs::rename(
                    source.join(".git/objects"),
                    fixture.path("external-objects"),
                )
                .expect("external object storage");
                symlink("../../external-objects", source.join(".git/objects"))
                    .expect("relative external objects");
            } else {
                fs::rename(source.join(".git"), fixture.path("external-common"))
                    .expect("external common metadata");
                fs::create_dir(source.join(".git")).expect("per-worktree metadata");
                fs::write(source.join(".git/commondir"), "../../external-common\n")
                    .expect("common directory link");
                fs::copy(
                    fixture.path("external-common/HEAD"),
                    source.join(".git/HEAD"),
                )
                .expect("per-worktree HEAD");
                fs::copy(
                    fixture.path("external-common/index"),
                    source.join(".git/index"),
                )
                .expect("per-worktree index");
            }
            let head = fixture.git(&source, &["rev-parse", "HEAD"]);
            fixture.git(&source, &["cat-file", "-e", "HEAD"]);
            let refused = fixture.adopt(&source, false);
            assert!(!refused.status.success(), "move accepted external {layout}");
            assert!(source.join("work").is_file());
            assert!(
                !fixture
                    .path("projects/forge.example/owner/dependent")
                    .exists()
            );
            assert_eq!(fixture.git(&source, &["rev-parse", "HEAD"]), head);
            fixture.git(&source, &["cat-file", "-e", "HEAD"]);
            let registered = fixture.adopt(&source, true);
            assert!(
                registered.status.success(),
                "{layout}: {}",
                String::from_utf8_lossy(&registered.stderr)
            );
            assert!(
                fixture
                    .listed_paths()
                    .contains(&source.canonicalize().expect("source path"))
            );
        }
    }

    #[test]
    fn failed_optional_core_worktree_probe_is_not_treated_as_absence() {
        let fixture = Fixture::new();
        let source = fixture.repo("configured");
        fixture.git(
            &source,
            &[
                "config",
                "core.worktree",
                source.to_str().expect("source UTF-8"),
            ],
        );
        fs::create_dir(fixture.path("bin")).expect("shim directory");
        let stub = fixture.path("bin/git");
        fs::write(
            &stub,
            r#"#!/bin/sh
for argument do
  if [ "$argument" = core.worktree ]; then
    exit 128
  fi
done
exec "$REPOT_REAL_GIT" "$@"
"#,
        )
        .expect("Git failure shim");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("executable shim");
        let path = env::join_paths(
            std::iter::once(fixture.path("bin"))
                .chain(env::split_paths(&env::var_os("PATH").expect("PATH"))),
        )
        .expect("test PATH");
        let output = fixture
            .repot()
            .arg("adopt")
            .arg(&source)
            .arg("--json")
            .env("PATH", path)
            .env("REPOT_REAL_GIT", &fixture.git)
            .output()
            .expect("failed optional check");
        assert!(!output.status.success());
        assert!(source.join("work").is_file());
        assert!(
            !fixture
                .path("projects/forge.example/owner/configured")
                .exists()
        );
        fixture.git(&source, &["cat-file", "-e", "HEAD"]);
    }

    #[test]
    fn status_refuses_ext_transport_even_when_repository_config_enables_it() {
        let fixture = Fixture::new();
        let source = fixture.repo("projects/unsafe-transport");
        let helper = fixture.path("remote-helper");
        fs::write(
            &helper,
            "#!/bin/sh\nprintf invoked > \"$REPOT_EXEC_MARKER\"\nexit 1\n",
        )
        .expect("unsafe transport marker");
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).expect("executable helper");
        fixture.git(
            &source,
            &[
                "remote",
                "set-url",
                "origin",
                &format!("ext::{}", helper.display()),
            ],
        );
        fixture.git(&source, &["config", "protocol.ext.allow", "always"]);
        let output = fixture
            .repot()
            .args(["status", "--json", "--timeout", "1"])
            .env("REPOT_EXEC_MARKER", fixture.path("executed"))
            .output()
            .expect("status transport guard");
        assert!(!output.status.success());
        assert!(
            !fixture.path("executed").exists(),
            "unsafe helper was executed"
        );
        assert!(source.join("work").is_file());
    }
}
