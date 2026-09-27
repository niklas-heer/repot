//! Where you have been: a small append-only log of repository visits that
//! ranks the picker by frecency (how often and how recently you went there).
//!
//! The log lives in `$XDG_STATE_HOME/repot/visits` (usually
//! `~/.local/state/repot/visits`). Each line is `<unix seconds>\t<path>`, with
//! `%`, tab, newline and carriage return percent-encoded. Appends are single
//! `O_APPEND` writes, so concurrent shells never interleave partial lines, and
//! the file is compacted by atomic rename once it grows large.

use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::Result;

/// Compaction keeps this many of the most recent visits.
const KEEP: usize = 4000;
/// Compact once the log passes this size.
const COMPACT_AT: u64 = 512 * 1024;

pub fn log_path() -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join("repot").join("visits"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn encode(path: &Path) -> Vec<u8> {
    let mut encoded = Vec::new();
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'%' => encoded.extend_from_slice(b"%25"),
            b'\t' => encoded.extend_from_slice(b"%09"),
            b'\n' => encoded.extend_from_slice(b"%0A"),
            b'\r' => encoded.extend_from_slice(b"%0D"),
            other => encoded.push(*other),
        }
    }
    encoded
}

fn decode(encoded: &[u8]) -> PathBuf {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded;
    while let Some((first, tail)) = rest.split_first() {
        let decoded = match (first, tail) {
            (b'%', [b'2', b'5', ..]) => Some(b'%'),
            (b'%', [b'0', b'9', ..]) => Some(b'\t'),
            (b'%', [b'0', b'A', ..]) => Some(b'\n'),
            (b'%', [b'0', b'D', ..]) => Some(b'\r'),
            _ => None,
        };
        if let Some(byte) = decoded {
            bytes.push(byte);
            rest = tail.get(2..).unwrap_or_default();
        } else {
            bytes.push(*first);
            rest = tail;
        }
    }
    PathBuf::from(OsString::from_vec(bytes))
}

/// The checkout containing `path`, found by looking for `.git` upwards.
fn checkout_root(path: &Path) -> Option<PathBuf> {
    let path = path.canonicalize().ok()?;
    path.ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .filter(|root| root.parent().is_some())
        .map(Path::to_path_buf)
}

fn last_entry(file: &mut fs::File) -> Option<Vec<u8>> {
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(4096);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let line = tail
        .strip_suffix(b"\n")?
        .rsplit(|byte| *byte == b'\n')
        .next()?;
    let (_, path) = line.split_at(
        line.iter()
            .position(|byte| *byte == b'\t')?
            .saturating_add(1),
    );
    Some(path.to_vec())
}

/// Record that the shell entered `path`. Paths outside a Git checkout and
/// moves within the checkout you are already in are ignored, so the log counts
/// switches between repositories rather than every `cd`.
pub fn record(path: &Path) -> Result<()> {
    let Some(root) = checkout_root(path) else {
        return Ok(());
    };
    let Some(log) = log_path() else {
        return Ok(());
    };
    let directory = log.parent().ok_or("visit log has no directory")?;
    fs::create_dir_all(directory).map_err(|error| format!("create state directory: {error}"))?;
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&log)
        .map_err(|error| format!("open visit log: {error}"))?;
    let encoded = encode(&root);
    if last_entry(&mut file).as_deref() == Some(encoded.as_slice()) {
        return Ok(());
    }
    let mut line = now().to_string().into_bytes();
    line.push(b'\t');
    line.extend_from_slice(&encoded);
    line.push(b'\n');
    file.write_all(&line)
        .map_err(|error| format!("write visit log: {error}"))?;
    if file
        .metadata()
        .is_ok_and(|metadata| metadata.len() > COMPACT_AT)
    {
        compact(&log);
    }
    Ok(())
}

/// Keep the most recent visits. A visit appended during the rewrite can be
/// lost; ranking tolerates that and the log is never left half-written.
fn compact(log: &Path) {
    let Ok(content) = fs::read(log) else {
        return;
    };
    let lines: Vec<&[u8]> = content
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    let kept = lines
        .get(lines.len().saturating_sub(KEEP)..)
        .unwrap_or_default();
    let Some(directory) = log.parent() else {
        return;
    };
    let Ok(mut temporary) = tempfile::NamedTempFile::new_in(directory) else {
        return;
    };
    let mut body = kept.join(&b'\n');
    body.push(b'\n');
    if temporary.write_all(&body).is_ok() {
        let _ = temporary.persist(log);
    }
}

/// Aggregated visits for one checkout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Visits {
    pub count: usize,
    pub last: Option<u64>,
    /// Sum of age-weighted visits; one visit in the last hour counts 16.
    pub frecency: u64,
}

/// Recent visits weigh more; old ones still count a little.
const fn weight(age: u64) -> u64 {
    const HOUR: u64 = 3600;
    match age {
        0..HOUR => 16,
        HOUR..86_400 => 8,
        86_400..604_800 => 4,
        604_800..2_592_000 => 2,
        _ => 1,
    }
}

pub fn load() -> HashMap<PathBuf, Visits> {
    let Some(content) = log_path().and_then(|log| fs::read(log).ok()) else {
        return HashMap::new();
    };
    aggregate(&content, now())
}

fn aggregate(content: &[u8], now: u64) -> HashMap<PathBuf, Visits> {
    let mut visits: HashMap<PathBuf, Visits> = HashMap::new();
    for line in content.split(|byte| *byte == b'\n') {
        let Some(tab) = line.iter().position(|byte| *byte == b'\t') else {
            continue;
        };
        let (time, path) = line.split_at(tab);
        let Some(time) = std::str::from_utf8(time)
            .ok()
            .and_then(|time| time.parse::<u64>().ok())
        else {
            continue;
        };
        let entry = visits
            .entry(decode(path.get(1..).unwrap_or_default()))
            .or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last = entry.last.max(Some(time));
        entry.frecency = entry
            .frecency
            .saturating_add(weight(now.saturating_sub(time)));
    }
    visits
}

/// When you last did something in a checkout with Git: the newest HEAD reflog
/// entry (commit, checkout, merge, reset, pull) that repot did not write itself.
/// repot labels its own entries through `GIT_REFLOG_ACTION`, so fast-forwards
/// from `repot sync` never make a checkout look recently used. Only the tail
/// of one file is read, so this stays cheap for hundreds of repositories.
pub fn activity(path: &Path) -> Option<u64> {
    let dot_git = path.join(".git");
    let git_dir = if dot_git.is_file() {
        let pointer = fs::read_to_string(&dot_git).ok()?;
        let target = PathBuf::from(pointer.strip_prefix("gitdir:")?.trim());
        if target.is_absolute() {
            target
        } else {
            path.join(target)
        }
    } else {
        dot_git
    };
    let mut file = fs::File::open(git_dir.join("logs/HEAD")).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(64 * 1024)))
        .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    last_own_entry(&tail)
}

/// The timestamp of the newest reflog line whose message is not repot's.
fn last_own_entry(reflog: &[u8]) -> Option<u64> {
    reflog.split(|byte| *byte == b'\n').rev().find_map(|line| {
        let tab = line.iter().position(|byte| *byte == b'\t')?;
        let (identity, message) = line.split_at(tab);
        if message
            .get(1..)
            .is_some_and(|message| message.starts_with(b"repot"))
        {
            return None;
        }
        // `<old> <new> <name> <<email>> <seconds> <zone>`
        let identity = std::str::from_utf8(identity).ok()?;
        let mut fields = identity.rsplitn(3, ' ');
        let _zone = fields.next()?;
        fields.next()?.parse().ok()
    })
}

/// "5 min ago"-style age for people, from a Unix timestamp.
pub fn ago(timestamp: u64) -> String {
    let seconds = now().saturating_sub(timestamp);
    let (value, unit) = match seconds {
        0..60 => return "just now".into(),
        60..3600 => (seconds / 60, "min"),
        3600..86_400 => (seconds / 3600, "h"),
        86_400..2_592_000 => (seconds / 86_400, "d"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "mo"),
        _ => (seconds / 31_536_000, "y"),
    };
    format!("{value} {unit} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_ignores_reflog_entries_written_by_repot() {
        let zero = "0".repeat(40);
        let log = format!(
            "{zero} {zero} A <a@b> 100 +0200\tcommit (initial): one\n\
             {zero} {zero} A <a@b> 200 +0200\tcheckout: moving from main to dev\n\
             {zero} {zero} A <a@b> 300 +0200\trepot: Fast-forward\n\
             {zero} {zero} A <a@b> 400 -0500\trepot\n"
        );
        assert_eq!(last_own_entry(log.as_bytes()), Some(200));
        assert_eq!(last_own_entry(b""), None);
        assert_eq!(last_own_entry(b"garbage without tab\n"), None);
    }

    #[test]
    fn paths_with_separators_round_trip_through_the_log_format() {
        for path in [
            "/a/b",
            "/a/100%/b",
            "/a\tb",
            "/new\nline",
            "/cr\r",
            "/%0A literal",
        ] {
            assert_eq!(
                decode(&encode(Path::new(path))),
                Path::new(path),
                "{path:?}"
            );
        }
    }

    #[test]
    fn frecency_prefers_recent_and_frequent_visits_and_skips_garbage() {
        let now = 10_000_000;
        let log = format!(
            "{}\t/recent\n{}\t/old\n{}\t/old\n{}\t/old\nnot a line\n\t/missing-time\n{}\t/often\n{}\t/often\n{}\t/often\n{}\t/often\n{}\t/often\n",
            now - 60,
            now - 90 * 86_400,
            now - 91 * 86_400,
            now - 92 * 86_400,
            now - 86_400,
            now - 2 * 86_400,
            now - 3 * 86_400,
            now - 4 * 86_400,
            now - 5 * 86_400,
        );
        let visits = aggregate(log.as_bytes(), now);
        assert_eq!(visits.len(), 3);
        let score = |path: &str| {
            visits
                .get(Path::new(path))
                .map(|v| v.frecency)
                .unwrap_or_default()
        };
        assert!(score("/recent") > score("/old"));
        // One visit this hour outweighs a few this week; a daily habit wins.
        assert!(score("/often") > score("/recent"));
        assert!(score("/recent") > score("/often") / 2);
        assert_eq!(visits.get(Path::new("/old")).map(|v| v.count), Some(3));
        assert_eq!(
            visits.get(Path::new("/recent")).and_then(|v| v.last),
            Some(now - 60)
        );
    }
}
