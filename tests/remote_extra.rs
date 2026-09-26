//! Cloud and vanity resolution through the binary, with only local fixtures.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::{Command, Output};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
        git: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let home = tempfile::tempdir().expect("home");
            let output = Command::new("sh")
                .args(["-c", "command -v git"])
                .output()
                .expect("git path");
            let git = PathBuf::from(String::from_utf8(output.stdout).expect("path").trim());
            fs::create_dir(home.path().join("bin")).expect("bin");
            let f = Self { home, git };
            for args in [
                vec!["init", "--bare", "--initial-branch=main", "remote.git"],
                vec!["init", "--initial-branch=main", "source"],
            ] {
                assert!(
                    f.command(f.git.to_str().expect("git"))
                        .args(args)
                        .output()
                        .expect("git")
                        .status
                        .success()
                );
            }
            for args in [
                vec!["commit", "--allow-empty", "-m", "initial"],
                vec!["push", "../remote.git", "main"],
            ] {
                assert!(
                    f.command(f.git.to_str().expect("git"))
                        .current_dir(home_path(&f).join("source"))
                        .args(args)
                        .output()
                        .expect("git")
                        .status
                        .success()
                );
            }
            f.script(
                "git",
                r#"#!/bin/sh
last=''
clone=0
cloud=0
canonical=0
for arg in "$@"; do
  test "$arg" != clone || clone=1
  case "$arg" in codecommit:*) cloud=1;; https://canonical.example/owner/repo) canonical=1;; esac
  last=$arg
done
if test "$clone" = 1; then
  printf '%s\n' "$GIT_ALLOW_PROTOCOL" >> "$HOME/clone-protocols"
  if test "$cloud" = 1 || test "$canonical" = 1; then
    exec "$REAL_GIT" clone --quiet "$HOME/remote.git" "$last"
  fi
  mkdir -p "$last"
  printf partial > "$last/partial"
  exit 1
fi
exec "$REAL_GIT" "$@"
"#,
            );
            f.script(
                "aws",
                "#!/bin/sh\nprintf '%s\n' \"$@\" > \"$HOME/aws-args\"\nprintf eu-west-3\n",
            );
            f
        }
        fn command(&self, program: &str) -> Command {
            let mut c = Command::new(program);
            c.current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.home.path().join("config"))
                .env("GHQ_ROOT", self.home.path().join("projects"))
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.home.path().join("bin").display()),
                )
                .env("REAL_GIT", &self.git)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.home.path().join("gitconfig"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env_remove("AWS_REGION")
                .env_remove("AWS_DEFAULT_REGION")
                .env_remove("AWS_PROFILE")
                .env_remove("REPOT_CD_FILE");
            for key in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                c.env_remove(key);
            }
            c
        }
        fn script(&self, name: &str, content: &str) {
            let p = self.home.path().join("bin").join(name);
            fs::write(&p, content).expect("script");
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).expect("executable");
        }
        fn run(&self, args: &[&str]) -> Output {
            self.command(env!("CARGO_BIN_EXE_repot"))
                .args(args)
                .output()
                .expect("repot")
        }
        fn server(
            body: impl Fn(&str, usize) -> String,
            status: &str,
            requests: usize,
        ) -> (String, thread::JoinHandle<String>) {
            let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
            listener.set_nonblocking(true).expect("nonblocking");
            let authority = listener.local_addr().expect("address").to_string();
            let response_bodies: Vec<_> =
                (0..requests).map(|index| body(&authority, index)).collect();
            let status = status.to_owned();
            let handle = thread::spawn(move || {
                let start = Instant::now();
                let mut collected = Vec::new();
                loop {
                    if let Ok((mut socket, _)) = listener.accept() {
                        socket.set_nonblocking(false).expect("blocking connection");
                        socket
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .expect("timeout");
                        let mut bytes = [0; 8192];
                        let n = socket.read(&mut bytes).expect("request");
                        let request = String::from_utf8_lossy(&bytes[..n]).into_owned();
                        let response_body =
                            response_bodies.get(collected.len()).expect("response body");
                        let response = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/forbidden\r\nConnection: close\r\n\r\n{response_body}",
                            response_body.len()
                        );
                        let _ = socket.write_all(response.as_bytes());
                        collected.push(request);
                        if collected.len() == requests {
                            return collected.join("\n");
                        }
                    }
                    assert!(
                        start.elapsed() < Duration::from_secs(10),
                        "metadata was not requested"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
            });
            (format!("http://{authority}/module/subpackage"), handle)
        }
    }
    fn home_path(f: &Fixture) -> &std::path::Path {
        f.home.path()
    }

    #[test]
    fn codecommit_explicit_environment_and_profile_resolution() {
        let f = Fixture::new();
        for (input, region, default, expected) in [
            (
                "codecommit::ap-south-1://profile@Repo",
                "eu-west-1",
                "us-east-1",
                "ap-south-1/Repo",
            ),
            (
                "codecommit://Repo",
                "eu-west-1",
                "us-east-1",
                "eu-west-1/Repo",
            ),
        ] {
            let out = f
                .command(env!("CARGO_BIN_EXE_repot"))
                .env("AWS_REGION", region)
                .env("AWS_DEFAULT_REGION", default)
                .args(["get", "--dry-run", input])
                .output()
                .expect("repot");
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
        }
        assert!(!f.home.path().join("aws-args").exists());
        let out = f
            .command(env!("CARGO_BIN_EXE_repot"))
            .env("AWS_DEFAULT_REGION", "us-east-2")
            .args(["get", "--dry-run", "codecommit://Repo"])
            .output()
            .expect("repot");
        assert!(String::from_utf8_lossy(&out.stdout).contains("us-east-2/Repo"));
        let out = f.run(&[
            "get",
            "codecommit://my-profile@Repo",
            "--vcs",
            "codecommit",
            "--json",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(f.home.path().join("projects/eu-west-3/Repo/.git").is_dir());
        assert!(
            fs::read_to_string(f.home.path().join("aws-args"))
                .expect("aws")
                .contains("--profile\nmy-profile")
        );
        assert!(
            fs::read_to_string(f.home.path().join("clone-protocols"))
                .expect("protocols")
                .contains(":codecommit")
        );
    }

    #[test]
    fn codecommit_invalid_inputs_are_rejected_without_helpers_or_secret_output() {
        let f = Fixture::new();
        for input in [
            "codecommit::../bad://Repo",
            "codecommit://profile:secret@Repo",
            "codecommit://Repo/path",
            "codecommit://..",
            "codecommit://-arg",
        ] {
            let out = f.run(&["get", "--dry-run", input]);
            assert!(!out.status.success());
            assert!(!String::from_utf8_lossy(&out.stderr).contains("secret"));
        }
        assert!(!f.home.path().join("aws-args").exists());
        assert!(!f.home.path().join("projects").exists());
    }

    #[test]
    fn vanity_git_meta_resolves_prefix_and_installs_clean_staging() {
        let f = Fixture::new();
        let (url, server) = Fixture::server(
            |host, _| {
                format!(
                    "<!-- <meta name='go-import' content='{host}/module git file:///bad'> -->\n<script><meta name='go-import' content='{host}/module git file:///bad'></script>\n<META content='{host}/module git https://canonical.example/owner/repo' NAME='go-import'>"
                )
            },
            "200 OK",
            2,
        );
        let out = f.run(&["get", &url, "--json"]);
        assert!(
            out.status.success(),
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            server
                .join()
                .expect("server")
                .starts_with("GET /module/subpackage?go-get=1 ")
        );
        let checkout = f.home.path().join("projects/127.0.0.1/module");
        assert!(checkout.join(".git").is_dir());
        assert!(!checkout.join("partial").exists());
        assert!(!checkout.join("subpackage").exists());
        assert!(!fs::read_dir(&checkout).expect("dir").any(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".repot-clone-")
        }));
    }

    #[test]
    fn vanity_rejects_redirects_mismatched_prefix_non_git_credentials_and_large_body() {
        for (status, content) in [
            ("302 Found", "<meta name='go-import' content='PREFIX git https://canonical.example/owner/repo'>".to_owned()),
            ("200 OK", "<meta name='go-import' content='wrong.example/module git https://canonical.example/owner/repo'>".to_owned()),
            ("200 OK", "<meta name='go-import' content='PREFIX hg https://canonical.example/owner/repo'>".to_owned()),
            ("200 OK", "<meta name='go-import' content='PREFIX git https://secret@canonical.example/owner/repo'>".to_owned()),
            ("200 OK", "x".repeat(1024 * 1024 + 1)),
            ("200 OK", "<body><meta name=go-import content='PREFIX git https://canonical.example/owner/repo'>".to_owned()),
        ] {
            let f = Fixture::new();
            let (url, server) = Fixture::server(|host, _| content.replace("PREFIX", &format!("{host}/module")), status, 1);
            let out = f.run(&["get", &url, "--json"]);
            assert!(!out.status.success());
            assert!(!String::from_utf8_lossy(&out.stdout).contains("secret"));
            server.join().expect("server");
            let parent = f.home.path().join("projects/127.0.0.1/module");
            assert!(!parent.exists());
        }
    }
    #[test]
    fn vanity_ancestor_requires_matching_metadata_at_the_prefix() {
        let f = Fixture::new();
        let (url, server) = Fixture::server(
            |host, index| {
                if index == 0 {
                    format!(
                        "<meta name=go-import content='{host}/module git https://canonical.example/owner/repo'>"
                    )
                } else {
                    "<html><head></head></html>".into()
                }
            },
            "200 OK",
            2,
        );
        let out = f.run(&["get", &url, "--json"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("prefix verification"));
        let requests = server.join().expect("server");
        assert!(requests.contains("GET /module?go-get=1 "));
        assert!(!f.home.path().join("projects").exists());
    }

    #[test]
    fn dry_run_and_explicit_or_scoped_git_backend_never_request_metadata() {
        let f = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}/module", listener.local_addr().expect("address"));
        assert!(f.run(&["get", "--dry-run", &url]).status.success());
        assert!(listener.accept().is_err());
        assert!(!f.run(&["get", "--vcs", "git", &url]).status.success());
        assert!(listener.accept().is_err());
        fs::write(
            f.home.path().join("gitconfig"),
            format!("[ghq \"{url}\"]\nvcs = git\n"),
        )
        .expect("config");
        assert!(!f.run(&["get", &url]).status.success());
        assert!(listener.accept().is_err());
        fs::write(
            f.home.path().join("gitconfig"),
            format!("[ghq \"{url}\"]\nvcs = svn\n"),
        )
        .expect("config");
        let out = f.run(&["get", &url]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("supports Git"));
        assert!(listener.accept().is_err());
    }

    #[test]
    fn github_web_paths_shorten_but_gitlab_subgroups_are_preserved() {
        let f = Fixture::new();
        let out = f.run(&[
            "get",
            "--dry-run",
            "https://github.com/owner/repo/tree/main/src",
        ]);
        assert!(out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("github.com/owner/repo"));
        assert!(!text.contains("/tree/"));
        let out = f.run(&["get", "--dry-run", "https://gitlab.com/group/subgroup/repo"]);
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("gitlab.com/group/subgroup/repo"));
    }
    #[test]
    fn vanity_metadata_timeout_is_bounded_and_cleans_failed_staging() {
        let f = Fixture::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let url = format!("http://{}/module", listener.local_addr().expect("address"));
        let server = thread::spawn(move || {
            let started = Instant::now();
            loop {
                if let Ok((socket, _)) = listener.accept() {
                    thread::sleep(Duration::from_secs(2));
                    drop(socket);
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(5));
                thread::sleep(Duration::from_millis(5));
            }
        });
        let started = Instant::now();
        let out = f.run(&["get", "--timeout", "1", &url, "--json"]);
        assert!(!out.status.success());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!f.home.path().join("projects").exists());
        server.join().expect("server");
    }
}
