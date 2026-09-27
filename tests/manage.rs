//! Manifest registration, discovery and restoration against isolated local repositories.

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
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
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
        fn repot(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot")
        }
        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("Git output")
                .trim()
                .to_owned()
        }
        fn init(&self, path: &str, remote: Option<&str>) {
            let path = self.path(path);
            self.git(
                self.temp.path(),
                &["init", "--initial-branch=main", text(&path)],
            );
            if let Some(remote) = remote {
                self.git(&path, &["remote", "add", "origin", remote]);
            }
        }
        fn manifest(&self, content: &str) {
            fs::create_dir_all(self.path("config/repot")).expect("manifest parent");
            fs::write(self.path("config/repot/repos.toml"), content).expect("manifest");
        }
        fn manifest_text(&self) -> String {
            fs::read_to_string(self.path("config/repot/repos.toml")).expect("manifest text")
        }
        fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            let mut entries = BTreeMap::new();
            snapshot(self.temp.path(), self.temp.path(), &mut entries);
            entries
        }
        fn bare_remote(&self) -> PathBuf {
            self.init("source", None);
            fs::write(self.path("source/keep"), "restored content\n").expect("source content");
            self.git(&self.path("source"), &["add", "."]);
            self.git(&self.path("source"), &["commit", "-m", "initial"]);
            self.git(
                self.temp.path(),
                &["clone", "--bare", "source", "remote.git"],
            );
            self.path("remote.git")
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
    fn rows(output: &Output) -> Vec<serde_json::Value> {
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("JSON {error}: {}", String::from_utf8_lossy(&output.stderr))
        })
    }
    fn snapshot(base: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).expect("read snapshot") {
            let entry = entry.expect("snapshot entry");
            let relative = entry
                .path()
                .strip_prefix(base)
                .expect("relative")
                .to_path_buf();
            let kind = entry.file_type().expect("kind");
            if kind.is_dir() {
                entries.insert(relative, Vec::new());
                snapshot(base, &entry.path(), entries);
            } else if kind.is_symlink() {
                entries.insert(
                    relative,
                    fs::read_link(entry.path())
                        .expect("link")
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                );
            } else {
                entries.insert(relative, fs::read(entry.path()).expect("bytes"));
            }
        }
    }

    #[test]
    fn find_reports_only_unregistered_strays_and_skips_generated_directories() {
        let world = World::new();
        world.init("ghq/known", None);
        world.init("extra", None);
        world.init("stray", None);
        world.init("node_modules/dependency", None);
        world.init("target/build", None);
        world.init(".cache/cached", None);
        world.init("stray/nested", None);
        world.manifest("[[repo]]\nurl = ''\npath = '~/extra'\nrestore = false\n");
        let output = world.repot(&["find", text(world.temp.path()), "--json"]);
        success(&output);
        let results = rows(&output);
        assert_eq!(results.len(), 1);
        assert_eq!(
            PathBuf::from(results[0]["path"].as_str().expect("path")),
            world.path("stray").canonicalize().expect("stray")
        );
        assert_eq!(results[0]["action"], "found");
    }

    #[test]
    fn register_local_only_checkout_is_portable_comment_preserving_and_idempotent() {
        let world = World::new();
        world.init("stray$dollar", None);
        world.manifest("# Keep this manifest introduction\n[settings]\nowners = ['me'] # Keep this owner note\n");
        let args = ["adopt", "stray$dollar", "--register", "--json"];
        success(&world.repot(&args));
        let original = world.manifest_text();
        assert!(original.contains("# Keep this manifest introduction"));
        assert!(original.contains("# Keep this owner note"));
        let parsed: toml::Value = toml::from_str(&original).expect("valid manifest");
        assert_eq!(parsed["repo"][0]["path"].as_str(), Some("~/stray$$dollar"));
        assert_eq!(parsed["repo"][0]["url"].as_str(), Some(""));
        assert_eq!(parsed["repo"][0]["restore"].as_bool(), Some(false));
        success(&world.repot(&args));
        assert_eq!(world.manifest_text(), original);
        let list = world.repot(&["list", "--json"]);
        success(&list);
        assert_eq!(rows(&list).len(), 1);
    }

    #[test]
    fn adopting_previously_registered_checkout_updates_entry_without_duplicates() {
        let world = World::new();
        world.init(
            "stray",
            Some("https://gitlab.example/group/subgroup/project.git"),
        );
        world.manifest("# global comment\n[[repo]] # project comment\nurl = 'https://gitlab.example/group/subgroup/project.git' # URL comment\npath = '~/stray'\nrestore = false # user choice\n");
        let output = world.repot(&["adopt", "stray", "--json"]);
        success(&output);
        assert!(!world.path("stray").exists());
        let destination = world.path("ghq/gitlab.example/group/subgroup/project");
        assert!(destination.join(".git").is_dir());
        let manifest = world.manifest_text();
        assert!(manifest.contains("# global comment"));
        assert!(manifest.contains("# URL comment"));
        assert!(manifest.contains("# user choice"));
        let parsed: toml::Value = toml::from_str(&manifest).expect("manifest");
        let repos = parsed["repo"].as_array().expect("repos");
        assert_eq!(repos.len(), 1);
        assert!(repos[0].get("path").is_none());
        assert_eq!(repos[0]["restore"].as_bool(), Some(false));
        success(&world.repot(&["adopt", text(&destination), "--json"]));
        assert_eq!(world.manifest_text(), manifest);
    }

    #[test]
    fn adopt_dry_run_validates_without_mutating_checkout_manifest_or_handoff() {
        let world = World::new();
        world.init("stray", Some("https://github.com/me/project"));
        fs::write(world.path("handoff"), "unchanged").expect("handoff");
        let before = world.snapshot();
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("REPOT_CD_FILE", world.path("handoff"))
            .args(["adopt", "stray", "--dry-run", "--json"])
            .output()
            .expect("adopt dry-run");
        success(&output);
        assert_eq!(rows(&output)[0]["action"], "move");
        assert_eq!(world.snapshot(), before);
    }

    #[test]
    fn explicit_new_manifest_and_json_handoff_work_together() {
        let world = World::new();
        world.init("stray", None);
        let output = world
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("REPOT_CD_FILE", world.path("handoff"))
            .args([
                "--manifest",
                "custom/repos.toml",
                "adopt",
                "stray",
                "--register",
                "--json",
            ])
            .output()
            .expect("custom manifest registration");
        success(&output);
        assert_eq!(rows(&output)[0]["action"], "register");
        assert!(world.path("custom/repos.toml").is_file());
        assert_eq!(
            PathBuf::from(fs::read_to_string(world.path("handoff")).expect("handoff")),
            world.path("stray").canonicalize().expect("stray")
        );
    }

    #[test]
    fn restore_clones_missing_local_remotes_and_leaves_existing_and_disabled_paths_untouched() {
        let world = World::new();
        let remote = world.bare_remote();
        fs::create_dir(world.path("occupied")).expect("occupied");
        fs::write(world.path("occupied/work"), "local-only\n").expect("local work");
        world.manifest(&format!("[[repo]]\nurl = '{}'\npath = '~/restored'\n\n[[repo]]\nurl = '{}'\npath = '~/occupied'\n\n[[repo]]\nurl = '{}'\npath = '~/disabled'\nrestore = false\n", remote.display(), remote.display(), remote.display()));
        let before = world.snapshot();
        let dryrun = world.repot(&["restore", "--dry-run", "--json"]);
        success(&dryrun);
        assert_eq!(world.snapshot(), before);
        let output = world.repot(&["restore", "--json"]);
        success(&output);
        assert_eq!(
            rows(&output)
                .iter()
                .map(|row| row["action"].as_str().expect("action"))
                .collect::<Vec<_>>(),
            ["clone", "skip", "skip"]
        );
        assert_eq!(
            fs::read_to_string(world.path("restored/keep")).expect("restored file"),
            "restored content\n"
        );
        assert_eq!(
            world.git(&world.path("restored"), &["rev-parse", "HEAD"]),
            world.git(&world.path("source"), &["rev-parse", "HEAD"])
        );
        assert_eq!(
            fs::read_to_string(world.path("occupied/work")).expect("local work"),
            "local-only\n"
        );
        assert!(!world.path("occupied/.git").exists());
        assert!(!world.path("disabled").exists());
        let after = world.snapshot();
        success(&world.repot(&["restore", "--json"]));
        assert_eq!(world.snapshot(), after);
    }

    #[test]
    fn failed_restore_cleans_staging_and_continues_the_batch() {
        let world = World::new();
        let remote = world.bare_remote();
        world.manifest(&format!("[[repo]]\nurl = '{}'\npath = '~/destinations/failure'\n\n[[repo]]\nurl = '{}'\npath = '~/destinations/success'\n", world.path("missing.git").display(), remote.display()));
        let output = world.repot(&["restore", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        let results = rows(&output);
        assert_eq!(results[0]["action"], "error");
        assert_eq!(results[1]["action"], "clone");
        assert!(!world.path("destinations/failure").exists());
        let names: Vec<_> = fs::read_dir(world.path("destinations"))
            .expect("destinations")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("success")]);
    }

    #[test]
    fn restore_dry_run_rejects_a_file_in_the_destination_parent_chain() {
        let world = World::new();
        fs::write(world.path("blocked"), "existing bytes").expect("blocking file");
        world.manifest(&format!(
            "[[repo]]\nurl = '{}'\npath = '~/blocked/child/project'\n",
            world.path("remote.git").display()
        ));
        let before = world.snapshot();
        let output = world.repot(&["restore", "--dry-run", "--json"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(rows(&output)[0]["action"], "error");
        assert_eq!(world.snapshot(), before);
    }
}
