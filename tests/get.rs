//! Real cloning, importing and safe updates against isolated local remotes.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    struct World {
        _temp: tempfile::TempDir,
        home: PathBuf,
        root: PathBuf,
        remotes: PathBuf,
        seed: PathBuf,
    }

    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    impl World {
        fn new() -> Self {
            let temp = tempfile::tempdir().expect("fixture");
            let home = temp.path().join("home");
            let root = temp.path().join("repos with spaces");
            let remotes = temp.path().join("remotes");
            let seed = temp.path().join("seed");
            for path in [&home, &root, &remotes, &seed] {
                fs::create_dir(path).expect("directory");
            }
            let result = Self {
                _temp: temp,
                home,
                root,
                remotes,
                seed,
            };
            result.git(&result.home, &["config", "--global", "user.name", "Test"]);
            result.git(
                &result.home,
                &["config", "--global", "user.email", "test@example.invalid"],
            );
            result.git(
                &result.home,
                &["config", "--global", "commit.gpgsign", "false"],
            );
            result.git(&result.home, &["config", "--global", "ghq.user", "team"]);
            result.git(
                &result.home,
                &["config", "--global", "ghq.defaultHost", "example.test"],
            );
            let key = format!("url.file://{}/.insteadOf", result.remotes.display());
            result.git(
                &result.home,
                &[
                    "config",
                    "--global",
                    "--add",
                    &key,
                    "https://example.test/team/",
                ],
            );
            result.git(
                &result.home,
                &[
                    "config",
                    "--global",
                    "--add",
                    &key,
                    "ssh://git@example.test/team/",
                ],
            );
            result.git(&result.seed, &["init", "--initial-branch=main"]);
            fs::write(result.seed.join("README"), "initial\n").expect("write");
            result.git(&result.seed, &["add", "."]);
            result.git(&result.seed, &["commit", "-m", "initial"]);
            result
        }

        fn command(&self, program: &str, cwd: &Path) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(cwd)
                .env("HOME", &self.home)
                .env("GHQ_ROOT", &self.root)
                .env("XDG_CONFIG_HOME", self.home.join("config"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("LC_ALL", "C")
                .env_remove("REPOT_CD_FILE")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE");
            command
        }

        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self.command("git", cwd).args(args).output().expect("git");
            success(&output);
            String::from_utf8(output.stdout)
                .expect("UTF8")
                .trim()
                .to_owned()
        }

        fn remote(&self, name: &str) -> PathBuf {
            let path = self.remotes.join(name);
            self.git(
                &self.home,
                &[
                    "clone",
                    "--bare",
                    self.seed.to_str().expect("seed"),
                    path.to_str().expect("remote"),
                ],
            );
            path
        }

        fn run(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"), &self.home)
                .args(args)
                .output()
                .expect("repot")
        }

        fn destination(&self, name: &str) -> PathBuf {
            self.root.join("example.test/team").join(name)
        }
    }

    #[test]
    fn clone_alias_shorthand_ssh_and_existing_checkout_are_ghq_compatible() {
        let world = World::new();
        world.remote("one");
        world.remote("two");
        success(&world.run(&["clone", "one"]));
        success(&world.run(&["get", "-p", "team/two"]));
        let one = world.destination("one");
        assert_eq!(
            fs::read_to_string(one.join("README")).expect("file"),
            "initial\n"
        );
        assert_eq!(
            world.git(&one, &["remote", "get-url", "origin"]),
            format!("file://{}/one", world.remotes.display())
        );
        fs::write(one.join("local"), "preserve").expect("local");
        success(&world.run(&["get", "example.test/team/one"]));
        assert_eq!(
            fs::read_to_string(one.join("local")).expect("local"),
            "preserve"
        );
    }

    #[test]
    fn clone_depth_branch_partial_bare_and_branch_suffix_use_real_git() {
        let world = World::new();
        world.git(&world.seed, &["switch", "-c", "feature"]);
        fs::write(world.seed.join("feature"), "feature").expect("file");
        world.git(&world.seed, &["add", "."]);
        world.git(&world.seed, &["commit", "-m", "feature"]);
        world.git(&world.seed, &["switch", "main"]);
        world.remote("depth");
        let partial = world.remote("partial");
        world.git(&partial, &["config", "uploadpack.allowFilter", "true"]);
        world.remote("bare");
        success(&world.run(&["get", "team/depth@feature", "--shallow"]));
        let depth = world.destination("depth");
        assert_eq!(world.git(&depth, &["branch", "--show-current"]), "feature");
        assert_eq!(
            world.git(&depth, &["rev-parse", "--is-shallow-repository"]),
            "true"
        );
        assert!(depth.join("feature").exists());
        success(&world.run(&[
            "get",
            "partial",
            "--partial",
            "blobless",
            "--branch",
            "main",
        ]));
        assert_eq!(
            world.git(
                &world.destination("partial"),
                &["config", "remote.origin.partialclonefilter"]
            ),
            "blob:none"
        );
        success(&world.run(&["get", "bare", "--bare"]));
        assert_eq!(
            world.git(
                &world.destination("bare.git"),
                &["rev-parse", "--is-bare-repository"]
            ),
            "true"
        );
    }

    #[test]
    fn stdin_parallel_import_continues_after_failure_and_deduplicates() {
        let world = World::new();
        world.remote("one");
        world.remote("two");
        let mut child = world
            .command(env!("CARGO_BIN_EXE_repot"), &world.home)
            .args(["get", "--parallel", "--json", "--vcs", "git"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"one\nmissing\ntwo\none\n")
            .expect("input");
        let output = child.wait_with_output().expect("wait");
        assert_eq!(output.status.code(), Some(1));
        let reports: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).expect("JSON");
        assert_eq!(reports.len(), 3);
        assert!(world.destination("one/.git").is_dir());
        assert!(world.destination("two/.git").is_dir());
        assert!(!world.destination("missing").exists());
        assert!(
            !fs::read_dir(world.destination("one").parent().expect("parent"))
                .expect("entries")
                .any(|entry| entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".repot-clone-"))
        );
    }

    #[test]
    fn update_fast_forwards_and_dirty_or_diverged_work_is_preserved() {
        let world = World::new();
        let remote = world.remote("one");
        success(&world.run(&["get", "one"]));
        let checkout = world.destination("one");
        let old = world.git(&checkout, &["rev-parse", "HEAD"]);
        fs::write(world.seed.join("next"), "remote").expect("file");
        world.git(&world.seed, &["add", "."]);
        world.git(&world.seed, &["commit", "-m", "next"]);
        world.git(
            &world.seed,
            &["push", remote.to_str().expect("remote"), "main"],
        );
        success(&world.run(&["get", "one", "--update", "--dry-run"]));
        assert_eq!(world.git(&checkout, &["rev-parse", "HEAD"]), old);
        assert_eq!(world.git(&checkout, &["rev-parse", "origin/main"]), old);
        success(&world.run(&["get", "one", "-u"]));
        assert_eq!(
            fs::read_to_string(checkout.join("next")).expect("next"),
            "remote"
        );
        fs::write(checkout.join("local"), "untracked").expect("file");
        let head = world.git(&checkout, &["rev-parse", "HEAD"]);
        assert_eq!(world.run(&["get", "one", "-u"]).status.code(), Some(3));
        assert_eq!(world.git(&checkout, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read_to_string(checkout.join("local")).expect("local"),
            "untracked"
        );
    }

    #[test]
    fn create_tree_and_bare_respect_owner_configuration_and_refuse_collisions() {
        let world = World::new();
        world.git(
            &world.home,
            &["config", "--global", "ghq.completeUser", "false"],
        );
        success(&world.run(&["create", "scratch", "--dry-run"]));
        assert!(!world.destination("scratch").exists());
        success(&world.run(&["create", "scratch"]));
        assert!(world.destination("scratch/.git").is_dir());
        assert!(
            world
                .git(&world.destination("scratch"), &["remote"])
                .is_empty()
        );
        assert!(!world.run(&["create", "scratch"]).status.success());
        success(&world.run(&["create", "empty", "--bare"]));
        assert_eq!(
            world.git(
                &world.destination("empty.git"),
                &["rev-parse", "--is-bare-repository"]
            ),
            "true"
        );
        let preview = world.run(&["get", "ruby", "--dry-run"]);
        success(&preview);
        assert!(String::from_utf8_lossy(&preview.stdout).contains("example.test/ruby/ruby"));
    }

    #[test]
    fn invalid_specs_and_clone_collisions_never_create_or_disclose_secrets() {
        let world = World::new();
        for spec in [
            "https://secret:token@example.test/team/x",
            "team/../escape",
            "ext::echo-secret",
            "https://example.test/team/x?secret=token",
        ] {
            let output = world.run(&["get", spec]);
            assert!(!output.status.success());
            assert!(!String::from_utf8_lossy(&output.stderr).contains("secret"));
        }
        world.remote("one");
        let target = world.destination("one");
        fs::create_dir_all(target.parent().expect("parent")).expect("parent");
        fs::write(&target, "keep").expect("collision");
        assert!(!world.run(&["get", "one"]).status.success());
        assert_eq!(fs::read_to_string(&target).expect("read"), "keep");
    }
}
