//! TOML/KDL parity, safe persistent edits, and reproducible editing sequences.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output, Stdio};

    #[derive(Clone, Copy, Debug)]
    enum Format {
        Toml,
        Kdl,
    }

    impl Format {
        const fn suffix(self) -> &'static str {
            match self {
                Self::Toml => "toml",
                Self::Kdl => "kdl",
            }
        }
        fn comment(self, text: &str) -> String {
            format!(
                "{} {text}\n",
                match self {
                    Self::Toml => "#",
                    Self::Kdl => "//",
                }
            )
        }
        fn settings(self, owners: &[&str]) -> String {
            let names = owners.iter().map(|owner| quote(owner)).collect::<Vec<_>>();
            match self {
                Self::Toml => format!("[settings]\nowners = [{}] # owner note\n", names.join(", ")),
                Self::Kdl => format!(
                    "settings {{\n    owners {} // owner note\n}}\n",
                    names.join(" ")
                ),
            }
        }
        fn repo(self, url: &str, path: Option<&str>, restore: Option<bool>) -> String {
            let mut text = match self {
                Self::Toml => format!(
                    "\n[[repo]] # repository note\nurl = {} # URL note\n",
                    quote(url)
                ),
                Self::Kdl => format!("\nrepo {} /* URL note */", quote(url)),
            };
            if let Some(path) = path {
                match self {
                    Self::Toml => writeln!(text, "path = {}", quote(path)).expect("TOML path"),
                    Self::Kdl => write!(text, " path={}", quote(path)).expect("KDL path"),
                }
            }
            if let Some(restore) = restore {
                match self {
                    Self::Toml => {
                        writeln!(text, "restore = {restore} # restore note").expect("TOML restore");
                    }
                    Self::Kdl => {
                        write!(text, " restore=#{restore} /* restore note */")
                            .expect("KDL restore");
                    }
                }
            }
            if matches!(self, Self::Kdl) {
                text.push_str(" // repository note\n");
            }
            text
        }
    }

    fn quote(text: &str) -> String {
        serde_json::to_string(text).expect("quoted string")
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 fixture path")
    }
    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn json(output: &Output) -> Vec<serde_json::Value> {
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!("JSON {error}: {}", String::from_utf8_lossy(&output.stderr))
        })
    }

    struct World {
        home: tempfile::TempDir,
    }
    impl World {
        fn new() -> Self {
            Self {
                home: tempfile::tempdir().expect("isolated home"),
            }
        }
        fn path(&self, name: &str) -> PathBuf {
            self.home.path().join(name)
        }
        fn manifest(&self, format: Format) -> PathBuf {
            self.path(&format!("config/repot/repos.{}", format.suffix()))
        }
        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("REPOT_EXTRA", self.home.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env("GIT_OPTIONAL_LOCKS", "0")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_CONFIG_COUNT")
                .env_remove("REPOT_CD_FILE");
            command
        }
        fn run(&self, manifest: Option<&Path>, args: &[&str]) -> Output {
            let mut command = self.command(env!("CARGO_BIN_EXE_repot"));
            if let Some(path) = manifest {
                command.arg("--manifest").arg(path);
            }
            command.args(args).output().expect("repot")
        }
        fn git(&self, cwd: &Path, args: &[&str]) -> String {
            let output = self
                .command("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("git");
            success(&output);
            String::from_utf8(output.stdout)
                .expect("Git UTF-8")
                .trim_end()
                .to_owned()
        }
        fn init(&self, name: &str, remote: Option<&str>) -> PathBuf {
            let path = self.path(name);
            self.git(
                self.home.path(),
                &["init", "--initial-branch=main", text(&path)],
            );
            if let Some(url) = remote {
                self.git(&path, &["remote", "add", "origin", url]);
            }
            path.canonicalize().expect("checkout")
        }
        fn commit(&self, path: &Path, name: &str) {
            fs::write(path.join(name), name).expect("commit file");
            self.git(path, &["add", "--", name]);
            self.git(path, &["-c", "commit.gpgsign=false", "commit", "-m", name]);
        }
        fn write(&self, format: Format, contents: &str) -> PathBuf {
            let path = self.manifest(format);
            fs::create_dir_all(path.parent().expect("manifest parent")).expect("config directory");
            fs::write(&path, contents).expect("manifest");
            path
        }
        fn listed(&self, manifest: Option<&Path>) -> BTreeSet<PathBuf> {
            let output = self.run(manifest, &["list", "--json"]);
            success(&output);
            json(&output)
                .into_iter()
                .map(|entry| PathBuf::from(entry["path"].as_str().expect("repository path")))
                .collect()
        }
        fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
            fn visit(base: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Vec<u8>>) {
                for entry in fs::read_dir(path).expect("snapshot directory") {
                    let entry = entry.expect("snapshot entry");
                    let path = entry.path();
                    let relative = path
                        .strip_prefix(base)
                        .expect("relative path")
                        .to_path_buf();
                    let kind = entry.file_type().expect("file type");
                    if kind.is_dir() {
                        entries.insert(relative, Vec::new());
                        visit(base, &path, entries);
                    } else if kind.is_symlink() {
                        entries.insert(
                            relative,
                            fs::read_link(path)
                                .expect("link")
                                .as_os_str()
                                .as_encoded_bytes()
                                .to_vec(),
                        );
                    } else {
                        entries.insert(relative, fs::read(path).expect("file bytes"));
                    }
                }
            }
            let mut entries = BTreeMap::new();
            visit(self.home.path(), self.home.path(), &mut entries);
            entries
        }
    }

    #[test]
    fn automatic_discovery_reads_either_format_and_expands_paths_identically() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            let expected: BTreeSet<_> =
                ["relative", "home", "expanded", "cash$dollar", "line\nfeed"]
                    .iter()
                    .map(|name| world.init(name, None))
                    .collect();
            let mut source = format.settings(&["alice", "team"]);
            for path in [
                "../../relative",
                "~/home",
                "$REPOT_EXTRA/expanded",
                "~/cash$$dollar",
                "~/line\nfeed",
                "~/home",
            ] {
                source.push_str(&format.repo("", Some(path), Some(false)));
            }
            world.write(format, &source);
            assert_eq!(world.listed(None), expected, "{format:?}");
            let found = world.run(None, &["find", text(world.home.path()), "--json"]);
            success(&found);
            assert!(json(&found).is_empty());
        }
    }

    #[test]
    fn default_ambiguity_fails_closed_and_explicit_selection_stays_independent() {
        let world = World::new();
        let one = world.init("one", None);
        let two = world.init("two", None);
        world.init("third", None);
        let toml = world.write(
            Format::Toml,
            &Format::Toml.repo("", Some("~/one"), Some(false)),
        );
        let kdl = world.write(
            Format::Kdl,
            &Format::Kdl.repo("", Some("~/two"), Some(false)),
        );
        let before = world.snapshot();
        for args in [vec!["list", "--json"], vec!["adopt", "third", "--register"]] {
            let output = world.run(None, &args);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("--manifest"));
            assert_eq!(world.snapshot(), before);
        }
        assert_eq!(world.listed(Some(&toml)), BTreeSet::from([one]));
        assert_eq!(world.listed(Some(&kdl)), BTreeSet::from([two]));
        let original_toml = fs::read(&toml).expect("TOML");
        success(&world.run(Some(&kdl), &["adopt", "third", "--register"]));
        assert_eq!(fs::read(&toml).expect("unchanged TOML"), original_toml);
        assert_eq!(world.listed(Some(&kdl)).len(), 2);
    }

    #[test]
    fn new_default_stays_toml_and_explicit_kdl_or_legacy_extensions_work() {
        let world = World::new();
        let repo = world.init("stray", None);
        success(&world.run(None, &["adopt", "stray", "--register"]));
        assert!(world.manifest(Format::Toml).is_file());
        assert!(!world.manifest(Format::Kdl).exists());
        let custom = world.path("custom/new.kdl");
        success(&world.run(Some(&custom), &["adopt", "stray", "--register"]));
        assert!(fs::read_to_string(&custom).expect("KDL").contains("repo "));
        assert_eq!(world.listed(Some(&custom)), BTreeSet::from([repo.clone()]));
        let legacy = world.path("legacy-config");
        fs::write(&legacy, Format::Toml.repo("", Some("~/stray"), Some(false)))
            .expect("legacy TOML");
        assert_eq!(world.listed(Some(&legacy)), BTreeSet::from([repo]));
    }

    #[test]
    fn owners_and_status_actions_are_equivalent_in_both_formats() {
        let world = World::new();
        let repo = world.init("checkout", Some("https://example.test/alice/project"));
        world.commit(&repo, "base");
        let base = world.git(&repo, &["rev-parse", "HEAD"]);
        world.git(&repo, &["update-ref", "refs/remotes/origin/main", &base]);
        world.git(
            &repo,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        world.git(&repo, &["config", "branch.main.remote", "origin"]);
        world.git(&repo, &["config", "branch.main.merge", "refs/heads/main"]);
        world.commit(&repo, "ahead");
        let mut observations = Vec::new();
        for format in [Format::Toml, Format::Kdl] {
            let manifest = world.write(
                format,
                &(format.settings(&["alice"])
                    + &format.repo(
                        "https://example.test/alice/project",
                        Some("~/checkout"),
                        None,
                    )),
            );
            let output = world.run(Some(&manifest), &["status", "--no-fetch", "--json"]);
            assert_eq!(output.status.code(), Some(3));
            let rows = json(&output);
            assert_eq!(rows[0]["state"], "ahead");
            assert_eq!(rows[0]["action"], "push");
            observations.push(rows);
        }
        assert_eq!(observations[0], observations[1]);
    }

    #[test]
    fn malformed_unknown_duplicate_and_wrong_types_fail_without_leaking_source() {
        let world = World::new();
        world.init("stray", None);
        let secret = "DO_NOT_PRINT_CREDENTIAL";
        let invalid_kdl = [
            "repo \"DO_NOT_PRINT_CREDENTIAL\" {",
            "unknown \"DO_NOT_PRINT_CREDENTIAL\"\n",
            "settings { unknown \"DO_NOT_PRINT_CREDENTIAL\"; }\n",
            "settings {}\nsettings { owners \"DO_NOT_PRINT_CREDENTIAL\"; }\n",
            "settings { owners \"DO_NOT_PRINT_CREDENTIAL\"; owners \"two\"; }\n",
            "settings { owners 42; } // DO_NOT_PRINT_CREDENTIAL\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" unknown=#true\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" path=\"a\" path=\"b\"\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" restore=\"false\"\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" path=42\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" \"extra\"\n",
            "repo 42 // DO_NOT_PRINT_CREDENTIAL\n",
            "repo \"DO_NOT_PRINT_CREDENTIAL\" { child; }\n",
            "(annotated)repo \"DO_NOT_PRINT_CREDENTIAL\"\n",
            "repo (annotated)\"DO_NOT_PRINT_CREDENTIAL\"\n",
        ];
        let invalid_toml = [
            "secret = 'DO_NOT_PRINT_CREDENTIAL'\n",
            "[settings]\nowners = 'DO_NOT_PRINT_CREDENTIAL'\n",
            "[[repo]]\nurl='DO_NOT_PRINT_CREDENTIAL'\nurl='duplicate'\n",
            "[[repo]]\nurl='DO_NOT_PRINT_CREDENTIAL'\nrestore='false'\n",
            "[[repo]]\nurl='DO_NOT_PRINT_CREDENTIAL'\npath=42\n",
        ];
        for (format, cases) in [
            (Format::Kdl, invalid_kdl.as_slice()),
            (Format::Toml, invalid_toml.as_slice()),
        ] {
            for source in cases {
                let manifest = world.write(format, source);
                for args in [vec!["list", "--json"], vec!["adopt", "stray", "--register"]] {
                    let output = world.run(Some(&manifest), &args);
                    assert!(!output.status.success(), "{format:?}: {source}");
                    assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
                    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
                    assert_eq!(
                        fs::read_to_string(&manifest).expect("manifest preserved"),
                        *source
                    );
                }
            }
        }
    }

    #[test]
    fn upsert_preserves_comments_choices_and_idempotency_in_both_formats() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            let url = "https://example.test/team/project";
            world.init("stray", Some(url));
            let source = format.comment("introduction survives")
                + &format.settings(&["team"])
                + &format.repo(url, Some("~/stray"), Some(false));
            let manifest = world.write(format, &source);
            success(&world.run(None, &["adopt", "stray", "--json"]));
            let target = world
                .path("ghq/example.test/team/project")
                .canonicalize()
                .expect("moved checkout");
            let edited = fs::read_to_string(&manifest).expect("edited manifest");
            for comment in [
                "introduction survives",
                "owner note",
                "URL note",
                "restore note",
                "repository note",
            ] {
                assert!(
                    edited.contains(comment),
                    "{format:?}: lost {comment}\n{edited}"
                );
            }
            assert!(!edited.contains("~/stray"));
            success(&world.run(None, &["adopt", text(&target), "--json"]));
            assert_eq!(fs::read_to_string(&manifest).expect("repeat"), edited);
            let restored = world.run(None, &["restore", "--dry-run", "--json"]);
            success(&restored);
            assert_eq!(json(&restored).len(), 1);
            assert!(
                json(&restored)[0]["reason"]
                    .as_str()
                    .expect("reason")
                    .contains("disabled")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_logical_extension_and_relative_paths_survive_edits() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            let registered = world.init("relative", None);
            let next = world.init("next", None);
            let target = world.path(&format!(
                "dotfiles/manifest.{}",
                match format {
                    Format::Toml => "kdl",
                    Format::Kdl => "toml",
                }
            ));
            fs::create_dir_all(target.parent().expect("dotfiles")).expect("dotfiles directory");
            fs::write(
                &target,
                format.comment("dotfile comment")
                    + &format.repo("", Some("../../relative"), Some(false)),
            )
            .expect("dotfile");
            let logical = world.manifest(format);
            fs::create_dir_all(logical.parent().expect("config")).expect("config directory");
            std::os::unix::fs::symlink(&target, &logical).expect("manifest link");
            assert_eq!(world.listed(None), BTreeSet::from([registered.clone()]));
            success(&world.run(None, &["adopt", "next", "--register"]));
            assert!(
                fs::symlink_metadata(&logical)
                    .expect("logical manifest")
                    .is_symlink()
            );
            assert!(
                fs::read_to_string(&target)
                    .expect("dotfile content")
                    .contains("dotfile comment")
            );
            assert_eq!(world.listed(None), BTreeSet::from([registered, next]));
        }
    }

    #[test]
    fn concurrent_registrations_keep_every_entry_in_each_format() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            let manifest = world.write(format, &format.comment("concurrent edits"));
            let expected: BTreeSet<_> = (0..6)
                .map(|index| world.init(&format!("repo-{index}"), None))
                .collect();
            let children: Vec<_> = expected
                .iter()
                .map(|path| {
                    world
                        .command(env!("CARGO_BIN_EXE_repot"))
                        .args(["adopt", text(path), "--register", "--json"])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                        .expect("concurrent registration")
                })
                .collect();
            for child in children {
                success(&child.wait_with_output().expect("registration finishes"));
            }
            assert_eq!(world.listed(None), expected);
            assert!(
                fs::read_to_string(manifest)
                    .expect("manifest")
                    .contains("concurrent edits")
            );
        }
    }

    #[test]
    fn dry_runs_leave_every_byte_unchanged_in_both_formats() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            world.init("stray", Some("https://example.test/team/project"));
            let manifest = world.write(
                format,
                &(format.comment("unchanged") + &format.settings(&["team"])),
            );
            let before = world.snapshot();
            for args in [
                vec!["adopt", "stray", "--dry-run"],
                vec!["adopt", "stray", "--register", "--dry-run"],
                vec!["restore", "--dry-run", "--json"],
            ] {
                success(&world.run(Some(&manifest), &args));
                assert_eq!(world.snapshot(), before, "{format:?} {args:?}");
            }
        }
    }

    #[test]
    fn credential_bearing_urls_are_rejected_without_disclosure_or_side_effects() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            world.init("stray", None);
            world.write(
                format,
                &format.repo(
                    "https://user:DO_NOT_PRINT_CREDENTIAL@example.test/team/private",
                    Some("~/missing"),
                    None,
                ),
            );
            let before = world.snapshot();
            for args in [
                vec!["restore", "--json"],
                vec!["adopt", "stray", "--register"],
            ] {
                let output = world.run(None, &args);
                assert!(!output.status.success());
                assert!(
                    !String::from_utf8_lossy(&output.stdout).contains("DO_NOT_PRINT_CREDENTIAL")
                );
                assert!(
                    !String::from_utf8_lossy(&output.stderr).contains("DO_NOT_PRINT_CREDENTIAL")
                );
                assert_eq!(world.snapshot(), before);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn dangling_default_symlinks_fail_closed_without_creating_another_manifest() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            world.init("stray", None);
            let logical = world.manifest(format);
            fs::create_dir_all(logical.parent().expect("config")).expect("config directory");
            std::os::unix::fs::symlink(world.path("missing-dotfile"), &logical)
                .expect("dangling manifest link");
            let before = world.snapshot();
            for args in [vec!["list", "--json"], vec!["adopt", "stray", "--register"]] {
                assert!(!world.run(None, &args).status.success());
                assert_eq!(world.snapshot(), before);
            }
        }
    }

    #[test]
    fn kdl_eof_comments_and_removed_property_comments_survive_updates() {
        for source in [
            "// sentinel without a final newline",
            "/- repo \"ignored\" path=\"~/ignored\" restore=#false\n// sentinel footer",
        ] {
            let world = World::new();
            let stray = world.init("stray", None);
            world.init("ignored", None);
            let manifest = world.write(Format::Kdl, source);
            success(&world.run(None, &["adopt", "stray", "--register"]));
            let edited = fs::read_to_string(&manifest).expect("edited KDL");
            assert!(edited.contains("sentinel"));
            assert_eq!(world.listed(None), BTreeSet::from([stray]));
        }
        let world = World::new();
        world.init("stray", Some("https://example.test/team/project"));
        let manifest = world.write(Format::Kdl, "repo \"https://example.test/team/project\" path= /* before path */ \"~/stray\" /* after path */ restore=#false // entry footer");
        success(&world.run(None, &["adopt", "stray"]));
        let edited = fs::read_to_string(&manifest).expect("updated KDL");
        for comment in ["before path", "after path", "entry footer"] {
            assert!(edited.contains(comment), "missing {comment}: {edited}");
        }
        assert!(!edited.contains("~/stray"));
        let restore = world.run(None, &["restore", "--dry-run", "--json"]);
        success(&restore);
        assert_eq!(json(&restore).len(), 1);
    }

    #[test]
    fn local_remote_restore_and_duplicate_discovery_match_between_formats() {
        for format in [Format::Toml, Format::Kdl] {
            let world = World::new();
            let seed = world.init("seed", None);
            world.commit(&seed, "content");
            let remote = world.path("remote.git");
            world.git(
                world.home.path(),
                &["clone", "--bare", text(&seed), text(&remote)],
            );
            let source = format.repo(text(&remote), Some("~/restored"), None)
                + &format.repo(text(&remote), Some("~/restored"), None)
                + &format.repo(text(&remote), Some("~/disabled"), Some(false));
            let manifest = world.write(format, &source);
            let before = world.snapshot();
            success(&world.run(None, &["restore", "--dry-run", "--json"]));
            assert_eq!(world.snapshot(), before);
            let output = world.run(None, &["restore", "--json"]);
            success(&output);
            let actions: Vec<_> = json(&output)
                .iter()
                .map(|entry| entry["action"].as_str().expect("action").to_owned())
                .collect();
            assert_eq!(actions, ["clone", "skip", "skip"]);
            assert_eq!(
                fs::read_to_string(world.path("restored/content")).expect("cloned content"),
                "content"
            );
            assert!(!world.path("disabled").exists());
            assert_eq!(
                world.listed(None),
                BTreeSet::from([world
                    .path("restored")
                    .canonicalize()
                    .expect("restored checkout")])
            );
            assert_eq!(
                fs::read_to_string(&manifest).expect("unchanged manifest"),
                source
            );
        }
    }

    #[test]
    fn seeded_persistent_edit_sequences_match_the_same_model_in_both_formats() {
        for seed in [3_u64, 17, 91] {
            for format in [Format::Toml, Format::Kdl] {
                let world = World::new();
                let manifest = world.write(format, &format.comment("simulation sentinel"));
                let repos: Vec<_> = (0..4)
                    .map(|index| world.init(&format!("repo-{index}"), None))
                    .collect();
                let mut registered = BTreeSet::new();
                let mut random = seed;
                let mut trace = String::new();
                for step in 0..18 {
                    random = random
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1);
                    let index = usize::try_from((random >> 8) % 4).expect("repo index");
                    let event = random % 5;
                    writeln!(trace, "{step}: event={event} repo={index}").expect("trace");
                    let before = fs::read(&manifest).expect("manifest snapshot");
                    match event {
                        0 | 1 => {
                            success(
                                &world.run(None, &["adopt", text(&repos[index]), "--register"]),
                            );
                            if !registered.insert(repos[index].clone()) {
                                assert_eq!(
                                    fs::read(&manifest).expect("idempotent bytes"),
                                    before,
                                    "seed={seed} {format:?}\n{trace}"
                                );
                            }
                        }
                        2 => {
                            success(&world.run(
                                None,
                                &["adopt", text(&repos[index]), "--register", "--dry-run"],
                            ));
                            assert_eq!(
                                fs::read(&manifest).expect("dry-run bytes"),
                                before,
                                "seed={seed} {format:?}\n{trace}"
                            );
                        }
                        3 => {
                            let mut edited = String::from_utf8(before).expect("UTF-8 manifest");
                            edited.push_str(&format.comment(&format!("external comment {step}")));
                            fs::write(&manifest, edited).expect("external editor");
                        }
                        _ => {
                            fs::write(&manifest, "invalid DO_NOT_PRINT_CREDENTIAL {")
                                .expect("inject invalid document");
                            let corrupted = fs::read(&manifest).expect("corrupted bytes");
                            let output =
                                world.run(None, &["adopt", text(&repos[index]), "--register"]);
                            assert!(!output.status.success(), "seed={seed} {format:?}\n{trace}");
                            assert!(
                                !String::from_utf8_lossy(&output.stderr)
                                    .contains("DO_NOT_PRINT_CREDENTIAL")
                            );
                            assert_eq!(fs::read(&manifest).expect("failure bytes"), corrupted);
                            fs::write(&manifest, before).expect("repair external edit");
                        }
                    }
                    assert_eq!(
                        world.listed(None),
                        registered,
                        "seed={seed} {format:?}\n{trace}"
                    );
                    assert!(
                        fs::read_to_string(&manifest)
                            .expect("manifest text")
                            .contains("simulation sentinel")
                    );
                }
            }
        }
    }
}
