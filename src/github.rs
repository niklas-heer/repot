//! One batched GitHub query per run instead of one network round trip per
//! checkout. It tells status which checkouts are already current, so their
//! fetch can be skipped, and answers the default-branch and merged-PR questions
//! that otherwise need `ls-remote` and `gh pr view` per checkout.
//!
//! Everything here is advisory: any error, missing answer or non-GitHub remote
//! simply means "fetch as usual", and every plan is still built from and
//! revalidated against local refs.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::config::remote_parts;
use crate::discovery::Repository;
use crate::{process, work};

/// What GitHub reported for one checkout's remote.
#[derive(Clone, Debug)]
pub struct Answer {
    /// The Git remote this answer is about.
    pub remote: String,
    /// Local tracking ref of the current branch, e.g. `refs/remotes/origin/feature`.
    pub tracking: String,
    /// Tip of the upstream branch on GitHub; `None` when it no longer exists.
    pub upstream_tip: Option<String>,
    pub default_branch: String,
    pub default_tip: String,
    /// `(head oid, base branch)` of merged pull requests from the current branch.
    pub merged: Vec<(String, String)>,
}

/// A checkout that can be asked about.
struct Question {
    path: PathBuf,
    remote: String,
    tracking: String,
    owner: String,
    name: String,
    remote_branch: String,
    /// Merged pull requests only matter off the default branch, and looking
    /// them up is the slow part of the query on large repositories.
    pull_requests: bool,
}

fn git(path: &Path, args: &[&str]) -> Option<String> {
    process::git_optional(path, args).ok().flatten()
}

fn question(path: &Path) -> Option<Question> {
    let branch = git(path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let upstream = git(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%09%(upstream:remoteref)%09%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    let mut fields = upstream.split('\t');
    let remote = fields.next().filter(|remote| !remote.is_empty())?;
    let remote_branch = fields.next()?.strip_prefix("refs/heads/")?;
    let tracking = fields
        .next()
        .filter(|tracking| tracking.starts_with(&format!("refs/remotes/{remote}/")))?;
    // The configured URL, before any insteadOf rewriting, names the repository.
    let url = git(path, &["config", "--get", &format!("remote.{remote}.url")])?;
    let (host, parts) = remote_parts(&url).ok()?;
    let [owner, name] = parts.as_slice() else {
        return None;
    };
    let cached_default = git(
        path,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    );
    let pull_requests =
        cached_default.as_deref() != Some(format!("{remote}/{remote_branch}").as_str());
    (host == "github.com").then(|| Question {
        path: path.to_path_buf(),
        remote: remote.to_owned(),
        tracking: tracking.to_owned(),
        owner: owner.clone(),
        name: name.clone(),
        remote_branch: remote_branch.to_owned(),
        pull_requests,
    })
}

fn quoted(text: &str) -> String {
    // A JSON string is a valid GraphQL string literal.
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn query(questions: &[&Question]) -> String {
    let mut query = String::from("query {");
    for (index, question) in questions.iter().enumerate() {
        let pull_requests = if question.pull_requests {
            format!(
                " pullRequests(headRefName: {}, states: MERGED, first: 5) {{ nodes {{ headRefOid baseRefName }} }}",
                quoted(&question.remote_branch)
            )
        } else {
            String::new()
        };
        let _ = write!(
            query,
            " r{index}: repository(owner: {}, name: {}) {{ \
               defaultBranchRef {{ name target {{ oid }} }} \
               upstream: ref(qualifiedName: {}) {{ target {{ oid }} }}{pull_requests} }}",
            quoted(&question.owner),
            quoted(&question.name),
            quoted(&format!("refs/heads/{}", question.remote_branch)),
        );
    }
    query.push_str(" }");
    query
}

fn parse(question: &Question, answer: &Value) -> Option<Answer> {
    let default = answer.get("defaultBranchRef")?;
    let text = |value: &Value, pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some(Answer {
        remote: question.remote.clone(),
        tracking: question.tracking.clone(),
        upstream_tip: answer
            .get("upstream")
            .filter(|upstream| !upstream.is_null())
            .and_then(|upstream| text(upstream, "/target/oid")),
        default_branch: text(default, "/name")?,
        default_tip: text(default, "/target/oid")?,
        merged: answer
            .pointer("/pullRequests/nodes")
            .and_then(Value::as_array)
            .map(|nodes| {
                nodes
                    .iter()
                    .filter_map(|node| {
                        Some((text(node, "/headRefOid")?, text(node, "/baseRefName")?))
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn ask(questions: &[&Question], timeout: Duration) -> Vec<(PathBuf, Answer)> {
    let query = query(questions);
    let field = format!("query={query}");
    let args: Vec<&OsStr> = ["api", "graphql", "-f", &field]
        .into_iter()
        .map(OsStr::new)
        .collect();
    let Some(first) = questions.first() else {
        return Vec::new();
    };
    let Ok(output) = process::run("gh", &args, &first.path, timeout) else {
        return Vec::new();
    };
    // Partial answers (for example one repository without access) still
    // arrive alongside errors; use whatever is present.
    let Ok(response) = serde_json::from_str::<Value>(&output.stdout) else {
        return Vec::new();
    };
    questions
        .iter()
        .enumerate()
        .filter_map(|(index, question)| {
            let answer = response.pointer(&format!("/data/r{index}"))?;
            parse(question, answer).map(|remote| (question.path.clone(), remote))
        })
        .collect()
}

/// The GitHub-hosted checkouts with an upstream branch, which GitHub can
/// answer for. Local reads only.
pub struct Survey {
    questions: Vec<Question>,
}

impl Survey {
    pub fn prepare(repositories: &[Repository], jobs: usize) -> Self {
        let questions = work::parallel(repositories, jobs, None, |repository| {
            question(&repository.path)
        })
        .map(|questions| questions.into_iter().flatten().collect())
        .unwrap_or_default();
        Self { questions }
    }

    pub fn asks_about(&self, path: &Path) -> bool {
        self.questions.iter().any(|question| question.path == path)
    }

    /// Ask GitHub, in small parallel batches.
    pub fn answer(&self, timeout: Duration) -> HashMap<PathBuf, Answer> {
        answer(&self.questions, timeout)
    }
}

fn answer(questions: &[Question], timeout: Duration) -> HashMap<PathBuf, Answer> {
    if questions.is_empty() {
        return HashMap::new();
    }
    // Small batches in parallel answer faster than one large query.
    let chunks: Vec<Vec<&Question>> = questions
        .chunks(10)
        .map(|chunk| chunk.iter().collect())
        .collect();
    work::parallel(&chunks, 8, None, |chunk| ask(chunk, timeout))
        .map(|answers| answers.into_iter().flatten().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question() -> Question {
        Question {
            path: PathBuf::from("/projects/owner/name"),
            remote: "origin".into(),
            tracking: "refs/remotes/origin/feature".into(),
            owner: "owner".into(),
            name: "name".into(),
            remote_branch: "feature\"} evil".into(),
            pull_requests: true,
        }
    }

    #[test]
    fn branch_names_cannot_break_out_of_the_query() {
        let text = query(&[&question()]);
        assert!(text.contains(r#"headRefName: "feature\"} evil""#), "{text}");
    }

    #[test]
    fn answers_are_parsed_and_deleted_branches_have_no_tip() {
        let answer: Value = serde_json::from_str(
            r#"{"defaultBranchRef":{"name":"main","target":{"oid":"aaa"}},
                "upstream":null,
                "pullRequests":{"nodes":[{"headRefOid":"bbb","baseRefName":"main"}]}}"#,
        )
        .expect("JSON");
        let remote = parse(&question(), &answer).expect("parsed");
        assert_eq!(remote.default_branch, "main");
        assert_eq!(remote.default_tip, "aaa");
        assert_eq!(remote.upstream_tip, None);
        assert_eq!(remote.merged, [("bbb".to_owned(), "main".to_owned())]);
        assert!(parse(&question(), &Value::Null).is_none());
    }
}
