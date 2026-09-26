//! Smoke-test actual local release artifacts without publishing or network access.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    const TARGETS: [&str; 4] = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    ];

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run(program: &str, args: &[&str], cwd: &Path) -> Output {
        Command::new(program)
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("run packaging tool")
    }

    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 fixture path")
    }

    fn host() -> String {
        let output = run("rustc", &["-vV"], &root());
        success(&output);
        String::from_utf8(output.stdout)
            .expect("rustc version")
            .lines()
            .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
            .expect("native Rust target")
    }

    fn hash_program() -> (&'static str, &'static [&'static str]) {
        if Command::new("sha256sum")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            ("sha256sum", &[])
        } else {
            ("shasum", &["-a", "256"])
        }
    }

    fn digest(path: &Path) -> String {
        let (program, args) = hash_program();
        let output = Command::new(program)
            .args(args)
            .arg(path)
            .output()
            .expect("hash archive");
        success(&output);
        String::from_utf8(output.stdout)
            .expect("checksum output")
            .split_whitespace()
            .next()
            .expect("digest")
            .to_owned()
    }

    fn verify_checksums(directory: &Path, filename: &str) {
        let (program, args) = hash_program();
        let output = Command::new(program)
            .args(args)
            .args(["-c", filename])
            .current_dir(directory)
            .output()
            .expect("verify generated checksums");
        success(&output);
    }

    fn packaged_files() -> Vec<String> {
        let base = root();
        let mut files: Vec<String> = [
            "LICENSE",
            "README.md",
            "assets/repot.png",
            "repot",
            "vendor/yaml-edit/LICENSE",
            "vendor/yaml-edit/REPOT_PATCH.md",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let mut directories = vec![base.join("docs")];
        while let Some(directory) = directories.pop() {
            for entry in fs::read_dir(directory).expect("documentation directory") {
                let entry = entry.expect("documentation entry");
                let kind = entry.file_type().expect("documentation type");
                if kind.is_dir() {
                    directories.push(entry.path());
                } else {
                    assert!(
                        kind.is_file(),
                        "release documentation must be regular files"
                    );
                    files.push(
                        entry
                            .path()
                            .strip_prefix(&base)
                            .expect("relative documentation path")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn native_archive_contains_a_working_executable_documentation_and_valid_checksum() {
        let temp = tempfile::tempdir().expect("packaging fixture");
        let destination = temp.path().join("release assets");
        let target = host();
        let version = env!("CARGO_PKG_VERSION");
        let output = run(
            "sh",
            &[
                "scripts/package.sh",
                version,
                &target,
                env!("CARGO_BIN_EXE_repot"),
                text(&destination),
            ],
            &root(),
        );
        success(&output);
        let name = format!("repot-{version}-{target}.tar.gz");
        let archive = destination.join(&name);
        assert!(archive.is_file());
        verify_checksums(&destination, &format!("{name}.sha256"));
        success(&run(
            "sh",
            &[
                "scripts/smoke-archive.sh",
                version,
                &target,
                text(&destination),
            ],
            &root(),
        ));
        let listing = run("tar", &["-tzf", text(&archive)], temp.path());
        success(&listing);
        let mut names: Vec<_> = String::from_utf8(listing.stdout)
            .expect("tar listing")
            .lines()
            .filter(|name| !name.ends_with('/'))
            .map(str::to_owned)
            .collect();
        names.sort();
        let expected = packaged_files();
        assert_eq!(names, expected);
        let unpacked = temp.path().join("unpacked");
        fs::create_dir(&unpacked).expect("unpack directory");
        success(&run(
            "tar",
            &["-xzf", text(&archive), "-C", text(&unpacked)],
            temp.path(),
        ));
        for name in expected.iter().filter(|name| *name != "repot") {
            assert_eq!(
                fs::read(unpacked.join(name)).expect("packaged documentation"),
                fs::read(root().join(name)).expect("source documentation")
            );
        }
        let version_output = Command::new(unpacked.join("repot"))
            .arg("--version")
            .output()
            .expect("run extracted executable");
        success(&version_output);
        assert_eq!(
            String::from_utf8(version_output.stdout)
                .expect("version")
                .trim(),
            format!("repot {version}")
        );
    }

    #[test]
    fn invalid_versions_targets_and_binary_version_mismatches_produce_no_artifacts() {
        let temp = tempfile::tempdir().expect("packaging fixture");
        let destination = temp.path().join("assets");
        let native = host();
        for (version, target) in [
            ("../escape", native.as_str()),
            ("0..1", native.as_str()),
            ("0.1.0\n0.2.0", native.as_str()),
            ("0.1.0\njunk", native.as_str()),
            ("v0.1.0", native.as_str()),
            ("999.999.999", native.as_str()),
            (env!("CARGO_PKG_VERSION"), "unknown-target"),
            (env!("CARGO_PKG_VERSION"), "../../escape"),
        ] {
            let output = run(
                "sh",
                &[
                    "scripts/package.sh",
                    version,
                    target,
                    env!("CARGO_BIN_EXE_repot"),
                    text(&destination),
                ],
                &root(),
            );
            assert!(
                !output.status.success(),
                "accepted version={version} target={target}"
            );
            if version.contains('\n') {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("MAJOR.MINOR.PATCH"),
                    "multiline version must fail validation before binary-version comparison"
                );
            }
            assert!(
                !destination.exists(),
                "invalid request created packaging output"
            );
        }
        let foreign = TARGETS
            .into_iter()
            .find(|target| *target != native)
            .expect("foreign target");
        let output = run(
            "sh",
            &[
                "scripts/package.sh",
                env!("CARGO_PKG_VERSION"),
                foreign,
                env!("CARGO_BIN_EXE_repot"),
                text(&destination),
            ],
            &root(),
        );
        assert!(
            !output.status.success(),
            "native executable was mislabeled as another architecture"
        );
        assert!(!destination.exists());
    }

    fn fixture_archives(directory: &Path, version: &str) -> BTreeMap<String, String> {
        let mut hashes = BTreeMap::new();
        let stage = directory.join("fixture");
        fs::create_dir(&stage).expect("fixture stage");
        fs::write(stage.join("LICENSE"), "fixture license\n").expect("license fixture");
        fs::write(stage.join("README.md"), "fixture documentation\n").expect("README fixture");
        for target in TARGETS {
            // Distinct valid archives expose a formula that swaps architecture digests.
            fs::write(
                stage.join("repot"),
                format!("fixture executable for {target}\n"),
            )
            .expect("distinct fixture payload");
            let name = format!("repot-{version}-{target}.tar.gz");
            let archive = directory.join(&name);
            success(&run(
                "tar",
                &[
                    "-czf",
                    text(&archive),
                    "-C",
                    text(&stage),
                    "repot",
                    "LICENSE",
                    "README.md",
                ],
                directory,
            ));
            hashes.insert(name, digest(&archive));
        }
        hashes
    }

    #[test]
    fn generated_formula_binds_each_release_url_to_its_actual_archive_digest() {
        let temp = tempfile::tempdir().expect("formula fixture");
        let version = env!("CARGO_PKG_VERSION");
        let expected = fixture_archives(temp.path(), version);
        let output = run(
            "sh",
            &["scripts/release-formula.sh", version, text(temp.path())],
            &root(),
        );
        success(&output);
        let formula = fs::read_to_string(temp.path().join("repot.rb")).expect("generated formula");
        let mut seen = BTreeMap::new();
        let mut pending_url = None;
        for line in formula.lines().map(str::trim) {
            if let Some(url) = line
                .strip_prefix("url \"")
                .and_then(|line| line.strip_suffix('"'))
            {
                assert!(url.starts_with(&format!(
                    "https://github.com/niklas-heer/repot/releases/download/v{version}/"
                )));
                pending_url = Some(url.rsplit('/').next().expect("asset filename").to_owned());
            } else if let Some(checksum) = line
                .strip_prefix("sha256 \"")
                .and_then(|line| line.strip_suffix('"'))
            {
                let name = pending_url.take().expect("checksum has a release URL");
                assert!(
                    seen.insert(name, checksum.to_owned()).is_none(),
                    "duplicate formula asset"
                );
            }
        }
        assert_eq!(seen, expected);
        verify_checksums(temp.path(), "SHA256SUMS");
        let sums = fs::read_to_string(temp.path().join("SHA256SUMS")).expect("release checksums");
        assert_eq!(sums.lines().count(), 5);
        assert!(sums.lines().any(|line| line.ends_with("repot.rb")));
        // Checksum validation must actually detect corruption of a packaged asset.
        let first = expected.keys().next().expect("asset");
        fs::write(temp.path().join(first), "corrupted archive").expect("inject corrupt asset");
        let (program, args) = hash_program();
        let output = Command::new(program)
            .args(args)
            .args(["-c", "SHA256SUMS"])
            .current_dir(temp.path())
            .output()
            .expect("verify corrupted release");
        assert!(!output.status.success());
    }

    #[test]
    fn formula_generation_refuses_incomplete_assets_and_invalid_release_versions() {
        let temp = tempfile::tempdir().expect("formula fixture");
        let version = env!("CARGO_PKG_VERSION");
        fixture_archives(temp.path(), version);
        fs::remove_file(
            temp.path()
                .join(format!("repot-{version}-x86_64-apple-darwin.tar.gz")),
        )
        .expect("remove one required target");
        let output = run(
            "sh",
            &["scripts/release-formula.sh", version, text(temp.path())],
            &root(),
        );
        assert!(!output.status.success());
        assert!(!temp.path().join("repot.rb").exists());
        assert!(!temp.path().join("SHA256SUMS").exists());
        for version in [
            "../escape",
            "0..1",
            "v0.1.0",
            "0.1.0;false",
            "0.1.0\n0.2.0",
            "0.1.0\njunk",
        ] {
            let output = run(
                "sh",
                &["scripts/release-formula.sh", version, text(temp.path())],
                &root(),
            );
            assert!(
                !output.status.success(),
                "accepted unsafe version {version}"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("MAJOR.MINOR.PATCH"),
                "invalid version must fail validation before looking for assets"
            );
        }
        assert!(!temp.path().join("repot.rb").exists());
    }
}
