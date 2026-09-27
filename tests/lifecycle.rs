//! Scratch repository creation exercised through the installed binary.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
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

        fn path(&self, relative: &str) -> PathBuf {
            self.temp.path().join(relative)
        }

        fn command(&self, binary: &str) -> Command {
            let mut command = Command::new(binary);
            command
                .current_dir(self.temp.path())
                .env("HOME", self.temp.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("GHQ_ROOT", self.path("repository roots"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_TEMPLATE_DIR")
                .env_remove("REPOT_CD_FILE");
            command
        }

        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("run repot")
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("run Git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("UTF-8 Git output")
                .trim()
                .to_owned()
        }

        fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut result = BTreeMap::new();
            snapshot(self.temp.path(), self.temp.path(), &mut result);
            result
        }
    }

    fn snapshot(base: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).expect("world directory") {
            let entry = entry.expect("world entry");
            let relative = entry
                .path()
                .strip_prefix(base)
                .expect("relative path")
                .to_path_buf();
            let kind = entry.file_type().expect("file kind");
            if kind.is_dir() {
                result.insert(relative, b"directory".to_vec());
                snapshot(base, &entry.path(), result);
            } else if kind.is_symlink() {
                result.insert(
                    relative,
                    fs::read_link(entry.path())
                        .expect("link target")
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else {
                result.insert(relative, fs::read(entry.path()).expect("file bytes"));
            }
        }
    }

    fn assert_success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn new_creates_an_unborn_main_without_remote_and_supports_list_and_jump() {
        let world = World::new();
        let output = world.repot(&["new", "demo.repo-v2_1"]);
        assert_success(&output);
        let checkout = world.path("repository roots/local/scratch/demo.repo-v2_1");
        assert!(checkout.join(".git").is_dir());
        assert_eq!(
            world.git(&checkout, &["symbolic-ref", "--short", "HEAD"]),
            "main"
        );
        assert!(world.git(&checkout, &["remote"]).is_empty());
        assert!(world.git(&checkout, &["for-each-ref"]).is_empty());
        assert!(world.git(&checkout, &["status", "--porcelain"]).is_empty());
        let list = world.repot(&["list", "--json"]);
        assert_success(&list);
        let rows: serde_json::Value = serde_json::from_slice(&list.stdout).expect("list JSON");
        assert_eq!(rows.as_array().expect("array").len(), 1);
        assert_eq!(
            PathBuf::from(rows[0]["path"].as_str().expect("checkout path")),
            checkout.canonicalize().expect("checkout")
        );
        let jump = world.repot(&["jump", "demo.repo-v2_1"]);
        assert_success(&jump);
        assert_eq!(
            PathBuf::from(String::from_utf8(jump.stdout).expect("jump output").trim()),
            checkout.canonicalize().expect("checkout")
        );
    }

    #[test]
    fn namespace_and_shell_handoff_preserve_roots_containing_spaces() {
        let world = World::new();
        let handoff = world.path("handoff file");
        fs::write(&handoff, "").expect("handoff");
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("REPOT_CD_FILE", &handoff)
            .args(["new", "project", "--namespace", "experiments-v2"])
            .output()
            .expect("new with shell handoff");
        assert_success(&output);
        let selected = PathBuf::from(
            String::from_utf8(fs::read(&handoff).expect("handoff bytes")).expect("handoff UTF-8"),
        );
        assert_eq!(
            selected.canonicalize().expect("handoff destination"),
            world
                .path("repository roots/local/experiments-v2/project")
                .canonicalize()
                .expect("new checkout")
        );
    }

    #[test]
    fn new_dry_run_does_not_create_roots_or_change_the_handoff_file() {
        let world = World::new();
        fs::write(world.path("handoff"), "existing value").expect("handoff");
        let before = world.snapshot();
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("REPOT_CD_FILE", world.path("handoff"))
            .args(["new", "planned", "--dry-run"])
            .output()
            .expect("dry run");
        assert_success(&output);
        assert_eq!(world.snapshot(), before);
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(rendered.contains("local/scratch/planned"));
    }

    #[test]
    fn new_with_an_owner_creates_an_empty_repository_at_its_tree_location() {
        let world = World::new();
        let before = world.snapshot();
        let planned = world.repot(&["new", "example.test/team/project", "--dry-run"]);
        assert_success(&planned);
        assert!(String::from_utf8_lossy(&planned.stdout).contains("example.test/team/project"));
        assert_eq!(world.snapshot(), before, "dry run changed the filesystem");
        assert_success(&world.repot(&["new", "example.test/team/project"]));
        let created = world.path("repository roots/example.test/team/project");
        assert!(created.join(".git").is_dir());
        assert!(!world.path("repository roots/local").exists());
        assert!(
            !world
                .repot(&["new", "example.test/team/project"])
                .status
                .success(),
            "an existing destination is never reused"
        );
        let bare_scratch = world.repot(&["new", "plain", "--bare"]);
        assert!(!bare_scratch.status.success());
        assert!(String::from_utf8_lossy(&bare_scratch.stderr).contains("owner/name"));
    }

    #[test]
    fn new_uses_the_last_configured_ghq_root() {
        let world = World::new();
        world.git(
            world.temp.path(),
            &["config", "--global", "--add", "ghq.root", "~/first"],
        );
        world.git(
            world.temp.path(),
            &["config", "--global", "--add", "ghq.root", "~/primary"],
        );
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env_remove("GHQ_ROOT")
            .args(["new", "project"])
            .output()
            .expect("new with Git roots");
        assert_success(&output);
        assert!(world.path("primary/local/scratch/project/.git").is_dir());
        assert!(!world.path("first").exists());
    }

    #[test]
    fn invalid_names_and_namespaces_never_modify_the_filesystem() {
        let world = World::new();
        let before = world.snapshot();
        for name in [
            "",
            ".",
            "..",
            "bad\\name",
            "two words",
            "-option",
            "ümlaut",
            "line\nbreak",
        ] {
            let output = world.repot(&["new", "--", name]);
            assert!(!output.status.success(), "accepted name {name:?}");
            assert_eq!(
                world.snapshot(),
                before,
                "name {name:?} mutated the filesystem"
            );
            let output = world.repot(&["new", "project", "--namespace", name]);
            assert!(!output.status.success(), "accepted namespace {name:?}");
            assert_eq!(
                world.snapshot(),
                before,
                "namespace {name:?} mutated the filesystem"
            );
        }
    }

    #[test]
    fn preexisting_directory_and_file_collisions_preserve_every_byte() {
        let world = World::new();
        fs::create_dir_all(world.path("repository roots/local/scratch/occupied"))
            .expect("occupied path");
        fs::write(
            world.path("repository roots/local/scratch/occupied/keep"),
            b"precious\0bytes",
        )
        .expect("existing work");
        fs::write(
            world.path("repository roots/local/scratch/file"),
            b"existing file",
        )
        .expect("existing file");
        let before = world.snapshot();
        for name in ["occupied", "file"] {
            for dry_run in [true, false] {
                let args = if dry_run {
                    vec!["new", name, "--dry-run"]
                } else {
                    vec!["new", name]
                };
                let output = world.repot(&args);
                assert!(!output.status.success(), "accepted collision {name}");
                assert_eq!(world.snapshot(), before);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_collision_is_not_replaced_or_followed() {
        let world = World::new();
        fs::create_dir_all(world.path("repository roots/local/scratch")).expect("scratch parent");
        let link = world.path("repository roots/local/scratch/dangling");
        std::os::unix::fs::symlink(world.path("absent"), &link).expect("dangling link");
        let before = world.snapshot();
        let output = world.repot(&["new", "dangling"]);
        assert!(!output.status.success());
        assert_eq!(world.snapshot(), before);
        assert!(
            fs::symlink_metadata(link)
                .expect("preserved link")
                .is_symlink()
        );
        assert!(!world.path("absent").exists());
    }

    #[test]
    fn new_ignores_git_init_templates() {
        let world = World::new();
        fs::create_dir_all(world.path("template/hooks")).expect("template directory");
        fs::write(
            world.path("template/injected-template-file"),
            "untrusted template content",
        )
        .expect("template sentinel");
        fs::write(
            world.path("template/hooks/post-checkout"),
            "#!/bin/sh\nexit 77\n",
        )
        .expect("template hook");
        world.git(
            world.temp.path(),
            &[
                "config",
                "--global",
                "init.templateDir",
                world.path("template").to_str().expect("template path"),
            ],
        );
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("GIT_TEMPLATE_DIR", world.path("template"))
            .args(["new", "plain"])
            .output()
            .expect("new with untrusted template");
        assert_success(&output);
        assert!(
            !world
                .path("repository roots/local/scratch/plain/.git/injected-template-file")
                .exists()
        );
        assert!(
            !world
                .path("repository roots/local/scratch/plain/.git/hooks/post-checkout")
                .exists()
        );
    }

    #[test]
    fn concurrent_creation_has_exactly_one_winner_and_no_partial_checkouts() {
        let world = World::new();
        let mut children = Vec::with_capacity(8);
        for _ in 0..8 {
            children.push(
                world
                    .command(env!("CARGO_BIN_EXE_repot"))
                    .args(["new", "contended"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .expect("spawn concurrent new"),
            );
        }
        let results: Vec<_> = children
            .into_iter()
            .map(|child| child.wait_with_output().expect("creation result"))
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|output| output.status.success())
                .count(),
            1,
            "{results:?}"
        );
        let parent = world.path("repository roots/local/scratch");
        let entries: Vec<_> = fs::read_dir(&parent)
            .expect("scratch directory")
            .map(|entry| entry.expect("scratch entry").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("contended")]);
        assert_eq!(
            world.git(
                &parent.join("contended"),
                &["symbolic-ref", "--short", "HEAD"]
            ),
            "main"
        );
    }
}
