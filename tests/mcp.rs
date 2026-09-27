//! End-to-end MCP transport, argument safety, and persistent lifecycle checks.

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::env;
    use std::fs;
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    use serde_json::{Value, json};

    struct World {
        home: tempfile::TempDir,
    }
    impl World {
        fn new() -> Self {
            Self {
                home: tempfile::tempdir().expect("temporary home"),
            }
        }
        fn path(&self, relative: &str) -> PathBuf {
            self.home.path().join(relative)
        }
        fn command(&self, program: &str) -> Command {
            let mut command = Command::new(program);
            command
                .current_dir(self.home.path())
                .env("HOME", self.home.path())
                .env("XDG_CONFIG_HOME", self.path("config"))
                .env("XDG_STATE_HOME", self.path("state"))
                .env("GHQ_ROOT", self.path("ghq"))
                .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_AUTHOR_NAME", "MCP Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "MCP Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
                .env("GIT_TERMINAL_PROMPT", "0")
                .env(
                    "PATH",
                    env::join_paths(
                        std::iter::once(self.path("bin"))
                            .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
                    )
                    .expect("test PATH"),
                )
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
        fn git(&self, cwd: &Path, args: &[&str]) {
            let output = self
                .command("git")
                .current_dir(cwd)
                .args(args)
                .output()
                .expect("Git");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fn repository(&self, name: &str, committed: bool) -> PathBuf {
            let path = self.path(name);
            self.git(
                self.home.path(),
                &["init", "--quiet", "--initial-branch=main", text(&path)],
            );
            if committed {
                fs::write(path.join("tracked"), "base").expect("tracked");
                self.git(&path, &["add", "."]);
                self.git(
                    &path,
                    &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
                );
            }
            path.canonicalize().expect("canonical repository")
        }
        fn server(&self, manifest: Option<&Path>) -> Client {
            let mut command = self.command(env!("CARGO_BIN_EXE_repot"));
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                command.process_group(0);
            }
            if let Some(path) = manifest {
                command.arg("--manifest").arg(path);
            }
            command
                .arg("mcp")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = command.spawn().expect("MCP server");
            let input = child.stdin.take().expect("MCP stdin");
            let output = child.stdout.take().expect("MCP stdout");
            let errors = child.stderr.take().expect("MCP stderr");
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(output).lines() {
                    let parsed = line.map_err(|error| error.to_string()).and_then(|line| {
                        serde_json::from_str(&line)
                            .map_err(|error| format!("non-JSON protocol stdout: {error}: {line}"))
                    });
                    if sender.send(parsed).is_err() {
                        break;
                    }
                }
            });
            // Drain stderr separately: diagnostics must never enter protocol stdout.
            std::thread::spawn(move || {
                for line in BufReader::new(errors).lines() {
                    if line.is_err() {
                        break;
                    }
                }
            });
            let mut client = Client {
                child,
                input: Some(input),
                receiver,
                next_id: 1,
            };
            let result = client.request(
                "initialize",
                &json!({
                    "protocolVersion": "2025-11-25", "capabilities": {},
                    "clientInfo": {"name": "repot-e2e", "version": "1"}
                }),
            );
            assert_eq!(result["result"]["serverInfo"]["name"], "repot");
            assert_eq!(
                result["result"]["serverInfo"]["version"],
                env!("CARGO_PKG_VERSION")
            );
            assert!(
                result["result"]["capabilities"]["tools"].is_object(),
                "{result}"
            );
            client.send(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
            client
        }
    }

    struct Client {
        child: Child,
        input: Option<ChildStdin>,
        receiver: Receiver<Result<Value, String>>,
        next_id: u64,
    }
    impl Client {
        fn send(&mut self, message: &Value) {
            let input = self.input.as_mut().expect("connected client");
            serde_json::to_writer(&mut *input, message).expect("serialize request");
            input.write_all(b"\n").expect("request framing");
            input.flush().expect("flush request");
        }
        fn receive(&self) -> Value {
            loop {
                let message = self
                    .receiver
                    .recv_timeout(Duration::from_secs(15))
                    .expect("MCP response deadline")
                    .expect("protocol JSON only");
                assert_eq!(message["jsonrpc"], "2.0");
                if message.get("id").is_some() {
                    return message;
                }
                assert!(
                    message["method"].is_string(),
                    "invalid notification: {message}"
                );
            }
        }
        fn request(&mut self, method: &str, params: &Value) -> Value {
            let id = self.next_id;
            self.next_id = self.next_id.checked_add(1).expect("request ID");
            self.send(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}));
            let response = self.receive();
            assert_eq!(response["id"], id, "response identity");
            response
        }
        fn call(&mut self, name: &str, arguments: &Value) -> Value {
            let response = self.request("tools/call", &json!({"name":name,"arguments":arguments}));
            assert!(response.get("error").is_none(), "{response}");
            let result = &response["result"];
            assert_ne!(result["isError"], true, "{response}");
            let data = result["structuredContent"].clone();
            assert!(data["exit_code"].is_number(), "{response}");
            assert!(data["review_required"].is_boolean(), "{response}");
            data
        }
    }
    impl Drop for Client {
        fn drop(&mut self) {
            #[cfg(unix)]
            if let Ok(pid) = i32::try_from(self.child.id()) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn text(path: &Path) -> &str {
        path.to_str().expect("UTF-8 fixture path")
    }
    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(directory).expect("snapshot directory") {
                let entry = entry.expect("entry");
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .expect("relative path")
                    .to_path_buf();
                if entry.file_type().expect("kind").is_dir() {
                    files.insert(relative, Vec::new());
                    visit(root, &path, files);
                } else {
                    files.insert(relative, fs::read(path).expect("snapshot file"));
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    #[test]
    fn initialize_catalog_and_concurrent_calls_keep_protocol_ids_and_stdout_clean() {
        let world = World::new();
        let mut client = world.server(None);
        let catalog = client.request("tools/list", &json!({}));
        let tools = catalog["result"]["tools"].as_array().expect("tools");
        let names: BTreeSet<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([
                "guide",
                "list",
                "roots",
                "status",
                "sync",
                "get",
                "find",
                "adopt",
                "migrate",
                "new",
                "create",
                "restore",
                "archive",
                "trash_list",
                "trash_restore",
                "publish"
            ])
        );
        for name in [
            "sync",
            "get",
            "adopt",
            "migrate",
            "new",
            "create",
            "restore",
            "archive",
            "trash_restore",
            "publish",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool["name"] == name)
                .expect("mutating tool");
            assert!(
                tool["inputSchema"]["required"]
                    .as_array()
                    .expect("required parameters")
                    .contains(&json!("dry_run")),
                "{tool}"
            );
        }
        for (id, name) in [("alpha", "guide"), ("beta", "roots"), ("gamma", "list")] {
            client.send(&json!({"jsonrpc":"2.0", "id":id, "method":"tools/call", "params":{"name":name,"arguments":{}}}));
        }
        let mut received = BTreeSet::new();
        for _ in 0..3 {
            let response = client.receive();
            assert!(response.get("error").is_none(), "{response}");
            if response["id"] == "alpha" {
                assert!(
                    response["result"]["content"][0]["text"]
                        .as_str()
                        .expect("agent guide")
                        .contains("dry-run")
                );
            } else {
                assert_eq!(response["result"]["structuredContent"]["exit_code"], 0);
            }
            assert!(received.insert(response["id"].as_str().expect("string ID").to_owned()));
        }
        assert_eq!(
            received,
            BTreeSet::from(["alpha".into(), "beta".into(), "gamma".into()])
        );
    }

    #[test]
    fn malformed_arguments_fail_without_side_effects_and_connection_survives() {
        let world = World::new();
        let mut client = world.server(None);
        let before = snapshot(world.home.path());
        for (name, args) in [
            ("new", &json!({"name":"safe"})),
            ("new", &json!({"name":"safe","dry_run":"false"})),
            (
                "new",
                &json!({"name":"safe","dry_run":false,"unknown":true}),
            ),
            ("get", &json!({"repositories":[],"dry_run":false})),
            ("get", &json!({"repositories":["--help"],"dry_run":false})),
            ("status", &json!({"jobs":0})),
            ("restore", &json!({"dry_run":false,"timeout":3601})),
            (
                "publish",
                &json!({"path":".","repository":"owner/project","dry_run":false}),
            ),
            ("does_not_exist", &json!({})),
        ] {
            let response = client.request("tools/call", &json!({"name":name,"arguments":args}));
            assert!(
                response.get("error").is_some() || response["result"]["isError"] == true,
                "{response}"
            );
            assert_eq!(snapshot(world.home.path()), before);
        }
        let result = client.call("new", &json!({"name":"safe","dry_run":true}));
        assert_eq!(result["exit_code"], 0);
        assert_eq!(snapshot(world.home.path()), before);
    }

    #[cfg(unix)]
    struct BlockedHelper(nix::unistd::Pid);

    #[cfg(unix)]
    impl BlockedHelper {
        fn install(world: &World) {
            use std::os::unix::fs::PermissionsExt as _;
            let git = env::split_paths(&env::var_os("PATH").expect("PATH"))
                .map(|directory| directory.join("git"))
                .find(|path| path.is_file())
                .expect("real Git");
            fs::create_dir_all(world.path("bin")).expect("shim directory");
            let shim = world.path("bin/git");
            let script = format!(
                "#!/bin/sh\ncase \" $* \" in\n  *' init '*)\n    printf 'init\\n' >> \"$HOME/helper.log\"\n    printf '%s' \"$$\" > \"$HOME/helper.pid\"\n    exec sleep 60;;\nesac\nexec '{}' \"$@\"\n",
                text(&git).replace('\'', "'\\''")
            );
            fs::write(&shim, script).expect("blocking Git shim");
            fs::set_permissions(shim, fs::Permissions::from_mode(0o755)).expect("executable shim");
        }
        fn started(world: &World) -> Self {
            let mut pid = None;
            observed("Git helper starts", || {
                pid = fs::read_to_string(world.path("helper.pid"))
                    .ok()
                    .and_then(|value| value.parse::<i32>().ok());
                pid.is_some()
            });
            Self(nix::unistd::Pid::from_raw(pid.expect("helper PID")))
        }
        fn stopped(&self) {
            observed("Git helper is reaped", || {
                matches!(
                    nix::sys::signal::kill(self.0, None),
                    Err(nix::errno::Errno::ESRCH)
                )
            });
        }
    }
    #[cfg(unix)]
    impl Drop for BlockedHelper {
        fn drop(&mut self) {
            let _ = nix::sys::signal::killpg(self.0, nix::sys::signal::Signal::SIGKILL);
        }
    }
    #[cfg(unix)]
    fn observed(description: &str, mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(10))
            .expect("deadline");
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out: {description}"
            );
            std::thread::yield_now();
        }
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_active_and_queued_mutations_stops_helpers_and_never_installs_checkouts() {
        let world = World::new();
        BlockedHelper::install(&world);
        let mut client = world.server(None);
        client.send(&json!({"jsonrpc":"2.0","id":"active","method":"tools/call","params":{"name":"new","arguments":{"name":"active","dry_run":false}}}));
        let helper = BlockedHelper::started(&world);
        client.send(&json!({"jsonrpc":"2.0","id":"queued","method":"tools/call","params":{"name":"new","arguments":{"name":"queued","dry_run":false}}}));
        // A response on the same transport establishes that queued request input
        // has reached the server; its mutation remains behind the active gate.
        let barrier = client.request("tools/list", &json!({}));
        assert!(barrier["result"]["tools"].is_array());
        for id in ["queued", "active"] {
            client.send(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id,"reason":"test cancellation"}}));
        }
        helper.stopped();
        // MCP discards responses for cancelled IDs. A new gated read proves
        // the server has finished cancellation cleanup and remains usable.
        assert_eq!(client.call("list", &json!({}))["data"], json!([]));
        assert_eq!(
            fs::read_to_string(world.path("helper.log")).expect("helper calls"),
            "init\n"
        );
        assert!(!world.path("ghq/local/scratch/active").exists());
        assert!(!world.path("ghq/local/scratch/queued").exists());
    }

    #[cfg(unix)]
    #[test]
    fn disconnect_and_server_signals_stop_active_helper_without_installing_checkout() {
        use nix::sys::signal::{Signal, kill};
        for signal in [None, Some(Signal::SIGTERM), Some(Signal::SIGINT)] {
            let world = World::new();
            BlockedHelper::install(&world);
            let mut client = world.server(None);
            client.send(&json!({"jsonrpc":"2.0","id":"active","method":"tools/call","params":{"name":"new","arguments":{"name":"disconnected","dry_run":false}}}));
            let helper = BlockedHelper::started(&world);
            if let Some(signal) = signal {
                let pid = i32::try_from(client.child.id()).expect("server PID");
                kill(nix::unistd::Pid::from_raw(pid), signal).expect("server signal");
            } else {
                drop(client.input.take());
            }
            helper.stopped();
            observed(&format!("server exits after {signal:?}"), || {
                client.child.try_wait().expect("server state").is_some()
            });
            assert!(!world.path("ghq/local/scratch/disconnected").exists());
        }
    }

    #[test]
    fn clone_and_create_tools_forward_options_and_preview_without_mutating() {
        let world = World::new();
        let seed = world.repository("seed", true);
        let remote = world.path("remote.git");
        world.git(
            world.home.path(),
            &["clone", "--bare", text(&seed), text(&remote)],
        );
        let url = "https://example.test/team/cloned";
        let rewrite = format!("url.file://{}.insteadOf", remote.display());
        world.git(world.home.path(), &["config", "--global", &rewrite, url]);
        let mut client = world.server(None);
        let before = snapshot(world.home.path());
        let preview = client.call("get", &json!({"repositories":[url],"dry_run":true,"branch":"main","shallow":true,"no_recursive":true,"jobs":2,"timeout":10}));
        assert_eq!(preview["exit_code"], 0);
        assert_eq!(snapshot(world.home.path()), before);
        let cloned = client.call("get", &json!({"repositories":[url],"dry_run":false,"branch":"main","shallow":true,"no_recursive":true,"jobs":2,"timeout":10}));
        assert_eq!(cloned["exit_code"], 0);
        let checkout = world.path("ghq/example.test/team/cloned");
        assert_eq!(
            fs::read(checkout.join("tracked")).expect("cloned data"),
            b"base"
        );
        assert!(
            checkout.join(".git/shallow").is_file(),
            "shallow flag forwarded"
        );
        let before = snapshot(world.home.path());
        for (tool, args) in [
            (
                "new",
                json!({"name":"prototype","namespace":"notes","dry_run":true}),
            ),
            (
                "create",
                json!({"repository":"example.test/team/created","bare":true,"dry_run":true}),
            ),
        ] {
            assert_eq!(client.call(tool, &args)["exit_code"], 0);
            assert_eq!(snapshot(world.home.path()), before);
        }
        assert_eq!(
            client.call(
                "new",
                &json!({"name":"prototype","namespace":"notes","dry_run":false})
            )["exit_code"],
            0
        );
        assert!(world.path("ghq/local/notes/prototype/.git").is_dir());
        assert_eq!(
            client.call(
                "create",
                &json!({"repository":"example.test/team/created","bare":true,"dry_run":false})
            )["exit_code"],
            0
        );
        assert!(
            world
                .path("ghq/example.test/team/created.git/HEAD")
                .is_file()
        );
    }

    #[test]
    fn find_adopt_and_migrate_preserve_literal_arguments_and_dirty_files() {
        let world = World::new();
        let flag_named = world.repository("--json", false);
        let shell_named = world.repository("literal; touch PWNED", false);
        let migrating = world.repository("migrating", false);
        world.git(
            &migrating,
            &[
                "remote",
                "add",
                "origin",
                "https://example.test/team/migrated",
            ],
        );
        fs::write(migrating.join("precious"), b"work\0\xff").expect("dirty file");
        let mut client = world.server(None);
        let found = client.call("find", &json!({"path":text(world.home.path())}));
        let paths: BTreeSet<_> = found["data"]
            .as_array()
            .expect("found entries")
            .iter()
            .map(|entry| entry["path"].as_str().expect("found path").to_owned())
            .collect();
        assert!(paths.contains(text(&flag_named)));
        assert!(paths.contains(text(&shell_named)));
        assert!(paths.contains(text(&migrating)));
        let before = snapshot(world.home.path());
        assert_eq!(
            client.call(
                "adopt",
                &json!({"path":"--json","register":true,"dry_run":true})
            )["exit_code"],
            0
        );
        assert_eq!(
            client.call("migrate", &json!({"path":"migrating","dry_run":true}))["exit_code"],
            0
        );
        assert_eq!(snapshot(world.home.path()), before);
        for path in ["--json", "literal; touch PWNED"] {
            assert_eq!(
                client.call(
                    "adopt",
                    &json!({"path":path,"register":true,"dry_run":false})
                )["exit_code"],
                0
            );
        }
        assert!(
            !world.path("PWNED").exists(),
            "shell metacharacters remain literal"
        );
        assert!(flag_named.is_dir());
        assert!(shell_named.is_dir());
        assert_eq!(
            client.call("migrate", &json!({"path":"migrating","dry_run":false}))["exit_code"],
            0
        );
        assert!(!migrating.exists());
        assert_eq!(
            fs::read(world.path("ghq/example.test/team/migrated/precious"))
                .expect("preserved dirty work"),
            b"work\0\xff"
        );
        assert_eq!(
            client.call("list", &json!({}))["data"]
                .as_array()
                .expect("registered repositories")
                .len(),
            3
        );
    }

    #[cfg(unix)]
    #[test]
    fn publish_preview_uses_explicit_visibility_host_and_relative_manifest_from_server_cwd() {
        use std::os::unix::fs::PermissionsExt as _;
        let world = World::new();
        let source = world.repository("scratch", true);
        fs::create_dir(world.path("bin")).expect("shim directory");
        let shim = world.path("bin/gh");
        fs::write(&shim, "#!/bin/sh\ncase \"$1\" in\n  auth) exit 0;;\n  config) printf 'https\\n';;\n  repo) exit 1;;\n  *) exit 99;;\nesac\n").expect("forge shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("executable shim");
        fs::write(
            world.path("selected.kdl"),
            "// explicitly selected manifest\n",
        )
        .expect("selected manifest");
        // If publish resolves the manifest relative to its checkout, this invalid
        // document would win instead of the server's explicitly selected file.
        fs::write(source.join("selected.kdl"), "invalid {").expect("decoy manifest");
        world.git(&source, &["add", "."]);
        world.git(
            &source,
            &["-c", "commit.gpgsign=false", "commit", "-m", "decoy"],
        );
        let mut client = world.server(Some(Path::new("selected.kdl")));
        let before = snapshot(world.home.path());
        let result = client.call("publish", &json!({"path":text(&source),"repository":"owner/project","visibility":"private","forge":"github","host":"forge.example","dry_run":true,"timeout":10}));
        assert_eq!(result["exit_code"], 0);
        let plan = result["output"].as_str().expect("publish plan");
        assert!(
            plan.contains("private repository https://forge.example/owner/project"),
            "{plan}"
        );
        assert!(plan.contains(text(&source)), "{plan}");
        assert!(plan.contains("ghq/forge.example/owner/project"), "{plan}");
        assert_eq!(snapshot(world.home.path()), before);
    }

    #[test]
    fn manifest_formats_support_local_restore_and_reversible_dirty_archive_over_mcp() {
        for (suffix, source) in [
            ("toml", "[[repo]]\nurl = REMOTE\npath = '~/restored'\n"),
            ("kdl", "repo REMOTE path=\"~/restored\"\n"),
            ("yaml", "repo:\n  - url: REMOTE\n    path: '~/restored'\n"),
        ] {
            let world = World::new();
            let seed = world.path("seed");
            world.git(
                world.home.path(),
                &["init", "--quiet", "--initial-branch=main", text(&seed)],
            );
            fs::write(seed.join("tracked"), "base").expect("tracked");
            world.git(&seed, &["add", "."]);
            world.git(
                &seed,
                &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
            );
            let remote = world.path("remote.git");
            world.git(
                world.home.path(),
                &["clone", "--bare", text(&seed), text(&remote)],
            );
            let manifest = world.path(&format!("manifest.{suffix}"));
            fs::write(
                &manifest,
                source.replace(
                    "REMOTE",
                    &serde_json::to_string(text(&remote)).expect("quoted remote"),
                ),
            )
            .expect("manifest");
            let mut client = world.server(Some(&manifest));
            let before = snapshot(world.home.path());
            let plan = client.call("restore", &json!({"dry_run":true}));
            assert_eq!(plan["exit_code"], 0);
            assert_eq!(snapshot(world.home.path()), before);
            let applied = client.call("restore", &json!({"dry_run":false}));
            assert_eq!(applied["exit_code"], 0);
            let checkout = world
                .path("restored")
                .canonicalize()
                .expect("canonical checkout");
            assert_eq!(
                fs::read(checkout.join("tracked")).expect("restored data"),
                b"base"
            );
            fs::write(checkout.join("tracked"), b"dirty\0\xff").expect("dirty edit");
            fs::write(checkout.join("untracked"), "local work").expect("untracked");
            let dirty = snapshot(&checkout);
            let plan = client.call("archive", &json!({"query":text(&checkout),"dry_run":true}));
            assert_eq!(plan["exit_code"], 0);
            assert_eq!(snapshot(&checkout), dirty);
            let archived =
                client.call("archive", &json!({"query":text(&checkout),"dry_run":false}));
            assert_eq!(archived["exit_code"], 0);
            assert!(!checkout.exists());
            let id = archived["data"][0]["id"].as_str().expect("archive ID");
            let listed = client.call("trash_list", &json!({}));
            assert_eq!(listed["data"][0]["id"], id);
            let planned = client.call("trash_restore", &json!({"id":id,"dry_run":true}));
            assert_eq!(planned["exit_code"], 0);
            assert!(!checkout.exists());
            let restored = client.call("trash_restore", &json!({"id":id,"dry_run":false}));
            assert_eq!(restored["exit_code"], 0);
            assert_eq!(
                snapshot(&checkout),
                dirty,
                "{suffix}: every checkout byte preserved"
            );
            let listed = client.call("list", &json!({}));
            assert_eq!(listed["data"].as_array().expect("repositories").len(), 1);
            let status = client.call("status", &json!({}));
            assert_eq!(status["review_required"], true);
            assert_eq!(status["data"][0]["action"], "review");
            assert_eq!(status["data"][0]["dirty"]["unstaged"], 1);
            assert_eq!(status["data"][0]["dirty"]["untracked"], 1);
            assert_eq!(snapshot(&checkout), dirty, "status defaults to no fetch");
            let before_sync = snapshot(world.home.path());
            let sync = client.call("sync", &json!({"dry_run":true}));
            assert_eq!(sync["review_required"], true);
            assert_eq!(snapshot(world.home.path()), before_sync);
        }
    }
}
