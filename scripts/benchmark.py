#!/usr/bin/env python3
"""Compare optimized repot and ghq on an isolated, deterministic local corpus.

Python only orchestrates executable-level measurements. No real home, checkout,
credential helper or remote is used. All samples include process startup and
captured output. Listing is comparable; ghq has no equivalent to repot status.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repot", type=Path, default=Path("target/release/repot"))
    parser.add_argument("--baseline", type=Path, help="optional optimized pre-change repot")
    parser.add_argument("--baseline-revision", help="source revision of the baseline binary")
    parser.add_argument("--root-mode", choices=["env", "config"], default="env")
    parser.add_argument("--ghq", default="ghq")
    parser.add_argument("--layout-bench", type=Path, help="optional optimized layout_bench example")
    parser.add_argument("--repos", type=int, default=24)
    parser.add_argument("--repeats", type=int, default=9)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.repos < 1 or args.repeats < 3:
        parser.error("use at least one repository and three measured repetitions")
    repot = str(args.repot.resolve(strict=True))
    baseline = str(args.baseline.resolve(strict=True)) if args.baseline else None
    ghq = shutil.which(args.ghq)
    if not ghq:
        parser.error("ghq executable is required for the listing comparison")
    with tempfile.TemporaryDirectory(prefix="repot-benchmark-") as temporary:
        home = Path(temporary).resolve()
        root = home / "projects"
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("GIT_", "REPOT_", "GHQ_", "XDG_"))}
        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
                   GHQ_ROOT=str(root), GIT_CONFIG_NOSYSTEM="1",
                   GIT_CONFIG_GLOBAL=str(home / "gitconfig"),
                   GIT_AUTHOR_NAME="Benchmark", GIT_AUTHOR_EMAIL="bench@example.invalid",
                   GIT_COMMITTER_NAME="Benchmark", GIT_COMMITTER_EMAIL="bench@example.invalid",
                   GIT_AUTHOR_DATE="2020-01-01T00:00:00Z",
                   GIT_COMMITTER_DATE="2020-01-01T00:00:00Z", GIT_TERMINAL_PROMPT="0")

        if args.root_mode == "config":
            env.pop("GHQ_ROOT")
            (home / "gitconfig").write_text(f'[ghq]\nroot = "{root}"\n')

        def run(command, cwd=home):
            result = subprocess.run(command, cwd=cwd, env=env, capture_output=True,
                                    check=True, timeout=120)
            return result.stdout

        source = home / "source"
        remote = home / "remote.git"
        run(["git", "init", "--initial-branch=main", str(source)])
        for number in range(16):
            (source / f"file-{number:02}.txt").write_text(f"stable fixture {number}\n" * 16)
        run(["git", "add", "."], source)
        run(["git", "commit", "-m", "benchmark fixture"], source)
        run(["git", "clone", "--bare", str(source), str(remote)])
        repositories = []
        for number in range(args.repos):
            destination = root / "example.invalid" / f"owner-{number % 4}" / f"repo-{number:04}"
            destination.parent.mkdir(parents=True, exist_ok=True)
            run(["git", "clone", "--quiet", str(remote), str(destination)])
            repositories.append(str(destination))

        commands = {
            "ghq_list": [ghq, "list", "--full-path"],
            "repot_list": [repot, "list", "--full-path"],
            "repot_status": [repot, "status", "--no-fetch", "--json", "--jobs", "4"],
        }
        if baseline:
            commands["baseline_status"] = [baseline, "status", "--no-fetch", "--json", "--jobs", "4"]
        expected = {name: run(command) for name, command in commands.items()}
        if sorted(expected["ghq_list"].splitlines()) != sorted(expected["repot_list"].splitlines()):
            raise RuntimeError("listing output parity failed")
        if len(expected["repot_list"].splitlines()) != args.repos:
            raise RuntimeError("incorrect corpus size")
        if baseline and json.loads(expected["repot_status"]) != json.loads(expected["baseline_status"]):
            raise RuntimeError("status JSON parity failed")
        samples = {name: [] for name in commands}
        # One additional warm-up, then rotate ordering to reduce drift bias.
        for command in commands.values():
            run(command)
        names = list(commands)
        for iteration in range(args.repeats):
            offset = iteration % len(names)
            for name in names[offset:] + names[:offset]:
                started = time.perf_counter_ns()
                output = run(commands[name])
                elapsed = (time.perf_counter_ns() - started) / 1_000_000
                if output != expected[name]:
                    raise RuntimeError(f"unstable output in {name}")
                samples[name].append(elapsed)
        result = {
            "platform": platform.platform(), "architecture": platform.machine(),
            "repositories": args.repos, "tracked_files_per_repository": 16,
            "root_mode": args.root_mode, "baseline_revision": args.baseline_revision,
            "repetitions": args.repeats, "warmups": 2, "status_jobs": 4,
            "cache": "warm OS caches; no cache dropping", "network": "none",
            "parity": "identical sorted full-path lists; identical baseline/current status JSON" if baseline
                      else "identical sorted full-path lists",
            "versions": {"git": run(["git", "--version"]).decode().strip(),
                         "ghq": run([ghq, "--version"]).decode().strip(),
                         "repot": run([repot, "--version"]).decode().strip()},
            "binary_bytes": {"repot": Path(repot).stat().st_size, "ghq": Path(ghq).stat().st_size},
            "binary_sha256": {"repot": hashlib.sha256(Path(repot).read_bytes()).hexdigest(),
                              "ghq": hashlib.sha256(Path(ghq).read_bytes()).hexdigest()},
            "measurements": {name: {"median_ms": statistics.median(values),
                                     "min_ms": min(values), "max_ms": max(values),
                                     "samples_ms": values} for name, values in samples.items()},
            "limitations": ["One machine and synthetic small clean repositories",
                            "Includes process startup and captured stdout",
                            "ghq has no equivalent status command; status compares repot versions",
                            "New repot includes other concurrent feature changes; not an isolated gix microbenchmark"],
        }
        # Count Git subprocesses per status run through a logging wrapper that
        # execs the real Git. Kept out of the timed samples: the wrapper adds a
        # shell to every spawn.
        real_git = shutil.which("git", path=env.get("PATH"))
        wrappers = home / "spawn-count"
        wrappers.mkdir()
        spawn_log = home / "spawns.log"
        wrapper = wrappers / "git"
        wrapper.write_text(f'#!/bin/sh\nprintf "%s\\n" "$1" >> "{spawn_log}"\nexec "{real_git}" "$@"\n')
        wrapper.chmod(0o755)
        spawns = {}
        for name, command in commands.items():
            if not name.endswith("status"):
                continue
            spawn_log.unlink(missing_ok=True)
            subprocess.run(command, cwd=home, env=dict(env, PATH=f"{wrappers}:{env.get('PATH', '')}"),
                           capture_output=True, check=True, timeout=120)
            count = len(spawn_log.read_text().splitlines()) if spawn_log.exists() else 0
            spawns[name] = {"git_spawns": count, "per_repository": round(count / args.repos, 2)}
        result["git_spawns"] = spawns
        if args.layout_bench:
            result["layout_microbenchmark"] = json.loads(run([str(args.layout_bench.resolve(strict=True)), *repositories]))
        if baseline:
            result["binary_bytes"]["baseline"] = Path(baseline).stat().st_size
            result["binary_sha256"]["baseline"] = hashlib.sha256(Path(baseline).read_bytes()).hexdigest()
        text = json.dumps(result, indent=2) + "\n"
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(text)
        print(text, end="")


if __name__ == "__main__":
    main()
