//! YAML-specific grammar and lossless-edit contracts through the actual CLI.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    struct World(tempfile::TempDir);

    impl World {
        fn new() -> Self {
            Self(tempfile::tempdir().expect("isolated home"))
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.path().join(name)
        }
        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.0.path())
                .env("HOME", self.0.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_TERMINAL_PROMPT", "0")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("REPOT_CD_FILE");
            command
        }
        fn run(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .arg("--manifest")
                .arg(self.path("repos.yaml"))
                .args(args)
                .output()
                .expect("repot")
        }
        fn write(&self, source: &str) {
            fs::write(self.path("repos.yaml"), source).expect("manifest");
        }
        fn read(&self) -> String {
            fs::read_to_string(self.path("repos.yaml")).expect("manifest")
        }
        fn init(&self, name: &str, remote: bool) -> PathBuf {
            let path = self.path(name);
            success(
                &self
                    .command("git")
                    .args(["init", "--initial-branch=main"])
                    .arg(&path)
                    .output()
                    .expect("git init"),
            );
            if remote {
                success(
                    &self
                        .command("git")
                        .arg("-C")
                        .arg(&path)
                        .args([
                            "remote",
                            "add",
                            "origin",
                            "https://example.test/team/project",
                        ])
                        .output()
                        .expect("git remote"),
                );
            }
            path
        }
    }

    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("fixture path")
    }

    #[test]
    fn rejects_duplicates_unknown_fields_wrong_types_and_indirection_without_leaking() {
        let invalid = [
            "unknown: DO_NOT_PRINT_CREDENTIAL\n",
            "repo: []\nrepo: []\n",
            "repo: []\n'repo': []\n",
            "settings: {}\nsettings: {}\n",
            "settings: {owners: [], owners: []}\n",
            "settings: {unknown: []}\n",
            "settings: {owners: [true]}\n",
            "settings: {owners: [123456789012345678901234567890]}\n",
            "settings: {owners: [0xFFFFFFFFFFFFFFFFFFFFFFFF]}\n",
            "settings: {owners: [0o777777777777777777777777777777]}\n",
            "settings: {owners: [1e9999]}\n",
            "settings: {owners: [.nan]}\n",
            "settings: {owners: [-.Inf]}\n",
            "settings:\n  owners:\n    -\n",
            "settings: {owners: null}\n",
            "repo: [{url: '', url: ''}]\n",
            "repo: [{url: '', path: '~/stray', path: '~/stray'}]\n",
            "repo: [{url: '', restore: false, restore: true}]\n",
            "repo: [{path: '~/stray'}]\n",
            "repo: [{url: '', restore: 'false'}]\n",
            "repo: [{url: '', restore: no}]\n",
            "repo: [{url: '', restore: 0}]\n",
            "repo: [{url: false}]\n",
            concat!("repo: [{", "url: }]\n"),
            "repo: [{url: '', path: null}]\n",
            "repo: [{url: '', path: }]\n",
            "repo: [{url: '', extra: secret}]\n",
            "repo: {}\n",
            "repo: [null]\n",
            "null\n",
            "[]\n",
            "repo: &repos []\n",
            "repo: *repos\n",
            "repo: !!seq []\n",
            "repo: [{<<: {url: ''}}]\n",
            "%YAML 1.1\n---\nrepo: []\n",
            "---\nrepo: []\n---\nrepo: []\n",
            "repo: [{url: 'unterminated}]\n",
            "repo: [{url: \"\\q\"}]\n",
        ];
        let world = World::new();
        world.init("stray", false);
        for source in invalid {
            let source = format!("# DO_NOT_PRINT_CREDENTIAL\n{source}");
            world.write(&source);
            for args in [vec!["list", "--json"], vec!["adopt", "stray", "--register"]] {
                let output = world.run(&args);
                assert!(!output.status.success(), "accepted {source:?}");
                assert!(
                    !String::from_utf8_lossy(&output.stderr).contains("DO_NOT_PRINT_CREDENTIAL")
                );
                assert!(
                    !String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT_CREDENTIAL")
                );
                assert_eq!(world.read(), source);
            }
        }
    }

    #[test]
    fn supports_flow_block_quoted_and_literal_strings() {
        let world = World::new();
        let expected = world
            .init("stray", false)
            .canonicalize()
            .expect("canonical");
        for source in [
            "{settings: {owners: [yes, 'false', \"123\", 0b10, 0XFF, -0xFF, nan, Infinity]}, repo: [{url: '', path: '~/stray', restore: false}]}\n",
            "---\nsettings:\n  owners: [on, off]\nrepo:\n  - url: ''\n    path: |-\n      ~/stray\n    restore: FALSE\n...\n",
            "repo:\n  - url: \"\"\n    path: >-\n      ~/stray\n    restore: False\n",
        ] {
            world.write(source);
            let output = world.run(&["list", "--json"]);
            success(&output);
            let rows: Vec<serde_json::Value> =
                serde_json::from_slice(&output.stdout).expect("rows");
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["path"], text(&expected));
            success(&world.run(&["adopt", "stray", "--register"]));
            assert_eq!(world.read(), source, "idempotent existing registration");
        }
    }

    #[test]
    fn comments_survive_empty_documents_flow_appends_and_path_removal() {
        for source in [
            "# sentinel EOF",
            "{} # sentinel EOF",
            "---\n{}\n...\n# sentinel EOF",
            "repo: [] # sentinel EOF",
        ] {
            let world = World::new();
            world.init("first", false);
            world.init("second", false);
            world.write(source);
            for name in ["first", "second"] {
                success(&world.run(&["adopt", name, "--register"]));
                assert!(world.read().contains("sentinel EOF"));
            }
            let output = world.run(&["list", "--json"]);
            success(&output);
            assert_eq!(
                serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout)
                    .expect("list")
                    .len(),
                2
            );
        }
        for source in [
            "# header\nrepo:\n  - url: https://example.test/team/project # URL note\n    path: ~/stray # path note\n    restore: false # restore note\n# footer\n",
            "repo: [{url: 'https://example.test/team/project', path: '~/stray', # path note\n  restore: false}] # footer\n",
        ] {
            let world = World::new();
            world.init("stray", true);
            world.write(source);
            success(&world.run(&["adopt", "stray"]));
            let edited = world.read();
            for note in ["path note", "footer"] {
                assert!(edited.contains(note), "lost {note}: {edited}");
            }
            assert!(!edited.contains("~/stray"));
            assert!(!world.path("stray").exists());
            assert!(world.path("ghq/example.test/team/project/.git").is_dir());
            success(&world.run(&["restore", "--dry-run", "--json"]));
            assert!(edited.contains("restore: false"));
        }
    }

    #[test]
    fn generated_scalars_roundtrip_special_paths_and_changed_urls_keep_comments() {
        let world = World::new();
        world.write("# human header\nrepo: []\n");
        let mut expected = std::collections::BTreeSet::new();
        for name in [
            "quote'and\"double",
            "line\nfeed",
            "dollar$cash",
            "colon: [braces] # hash",
            "日本語",
        ] {
            let path = world.init(name, false);
            success(&world.run(&["adopt", text(&path), "--register"]));
            expected.insert(path.canonicalize().expect("canonical"));
        }
        let output = world.run(&["list", "--json"]);
        success(&output);
        let actual: std::collections::BTreeSet<_> =
            serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout)
                .expect("list")
                .into_iter()
                .map(|row| PathBuf::from(row["path"].as_str().expect("path")))
                .collect();
        assert_eq!(actual, expected);
        let world = World::new();
        let path = world.init("stray", false);
        world.write("repo:\n  - url: '' # URL note\n    path: ~/stray # path note\n    restore: false # manual opt out\n");
        success(
            &world
                .command("git")
                .arg("-C")
                .arg(&path)
                .args([
                    "remote",
                    "add",
                    "origin",
                    "https://example.test/team/project",
                ])
                .output()
                .expect("remote"),
        );
        success(&world.run(&["adopt", "stray", "--register"]));
        let edited = world.read();
        for note in [
            "URL note",
            "path note",
            "manual opt out",
            "https://example.test/team/project",
            "restore: false",
        ] {
            assert!(edited.contains(note), "lost {note}: {edited}");
        }
    }

    #[test]
    fn numeric_looking_strings_never_panic_and_keep_their_spelling_when_edited() {
        for scalar in [
            "--9223372036854775808",
            "'--9223372036854775808'",
            "\"--9223372036854775808\"",
            "--09223372036854775808",
            "+-9223372036854775808",
        ] {
            let world = World::new();
            world.init("stray", false);
            world.write(&format!("settings: {{owners: [{scalar}]}} # owner note\n"));
            success(&world.run(&["list", "--json"]));
            success(&world.run(&["adopt", "stray", "--register"]));
            let edited = world.read();
            assert!(edited.contains(scalar), "changed scalar: {edited}");
            assert!(edited.contains("owner note"));
            success(&world.run(&["adopt", "stray", "--register"]));
            assert_eq!(world.read(), edited);
        }
    }
}
