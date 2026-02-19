//! GitHub API helpers for posting diff-coverage comments on pull requests
//! and creating check runs with line-level annotations.

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;

use crate::model::Annotation;

const COMMENT_MARKER: &str = "<!-- covrs-comment -->";

/// Maximum annotations per GitHub Check Runs API request.
const MAX_ANNOTATIONS_PER_REQUEST: usize = 50;

/// Build a ureq request with standard GitHub API headers.
fn github_request(method: &str, url: &str, token: &str) -> ureq::Request {
    ureq::request(method, url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "covrs")
        .set("X-GitHub-Api-Version", "2022-11-28")
}

/// Map a ureq response result into an anyhow error with context.
fn check_response(
    result: Result<ureq::Response, ureq::Error>,
    action: &str,
) -> Result<ureq::Response> {
    match result {
        Ok(resp) => Ok(resp),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("GitHub API error {action} (HTTP {code}): {body}");
        }
        Err(e) => bail!("Failed to {action}: {e}"),
    }
}

/// Resolved GitHub Actions context, read from environment variables.
pub struct Context {
    token: String,
    repo: String,
    pr_number: u64,
    pub sha: Option<String>,
}

impl Context {
    /// Build a context from standard GitHub Actions environment variables
    /// (`GITHUB_TOKEN`, `GITHUB_REPOSITORY`, `GITHUB_REF`).
    ///
    /// The commit SHA is resolved by querying the GitHub API for the PR head
    /// commit rather than using `GITHUB_SHA`, which on `pull_request` events
    /// points to a temporary merge commit instead of the actual PR head.
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .context("GITHUB_TOKEN environment variable is required")?;
        let repo = std::env::var("GITHUB_REPOSITORY")
            .context("GITHUB_REPOSITORY environment variable is required")?;
        let pr_number =
            pr_number_from_ref().context("could not determine PR number from GITHUB_REF")?;
        let sha = fetch_pr_head_sha(&token, &repo, pr_number)
            .map(Some)
            .unwrap_or_else(|e| {
                eprintln!("Warning: could not fetch PR head SHA: {e}");
                std::env::var("GITHUB_SHA").ok()
            });
        Ok(Self {
            token,
            repo,
            pr_number,
            sha,
        })
    }

    /// Fetch the unified diff for the pull request.
    pub fn fetch_diff(&self) -> Result<String> {
        eprintln!(
            "Fetching diff for {}/pull/{} ...",
            self.repo, self.pr_number
        );
        fetch_pr_diff(&self.token, &self.repo, self.pr_number)
    }

    /// Create or update a comment on the pull request.
    pub fn post_comment(&self, body: &str) -> Result<()> {
        post_comment(&self.token, &self.repo, self.pr_number, body)?;
        eprintln!("Comment posted to {}/pull/{}", self.repo, self.pr_number);
        Ok(())
    }

    /// Create a check run with line-level annotations for uncovered lines.
    ///
    /// Annotations are submitted in batches of 50 (the GitHub API limit).
    /// The check run is created with conclusion `neutral` so it never
    /// blocks merges.
    pub fn post_annotations(&self, annotations: &[Annotation]) -> Result<()> {
        let sha = self
            .sha
            .as_deref()
            .context("commit SHA is required for check run annotations")?;

        post_check_run(&self.token, &self.repo, sha, annotations)?;
        eprintln!(
            "Check run with {} annotations posted to {}/pull/{}",
            annotations.len(),
            self.repo,
            self.pr_number
        );
        Ok(())
    }
}

/// Extract PR number from GITHUB_REF (e.g. "refs/pull/42/merge" → 42).
fn pr_number_from_ref() -> Option<u64> {
    let github_ref = std::env::var("GITHUB_REF").ok()?;
    let parts: Vec<&str> = github_ref.split('/').collect();
    if parts.len() >= 3 && parts[0] == "refs" && parts[1] == "pull" {
        parts[2].parse().ok()
    } else {
        None
    }
}

fn fetch_pr_diff(token: &str, repo: &str, pr_number: u64) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/pulls/{pr_number}");
    let resp = check_response(
        github_request("GET", &url, token)
            .set("Accept", "application/vnd.github.v3.diff")
            .call(),
        "fetching PR diff",
    )?;
    resp.into_string()
        .context("Failed to read PR diff response body")
}

#[derive(Deserialize)]
struct PullRequest {
    head: PullRequestHead,
}

#[derive(Deserialize)]
struct PullRequestHead {
    sha: String,
}

/// Fetch the head commit SHA for a pull request from the GitHub API.
///
/// On `pull_request` events `GITHUB_SHA` is the merge commit, not the actual
/// head commit of the PR branch.  This function queries the Pulls API to get
/// the real head SHA so that check-run annotations and blob permalinks resolve
/// correctly.
fn fetch_pr_head_sha(token: &str, repo: &str, pr_number: u64) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/pulls/{pr_number}");
    let resp = check_response(
        github_request("GET", &url, token).call(),
        "fetching PR head SHA",
    )?;
    let pr: PullRequest = resp
        .into_json()
        .context("Failed to parse pull request JSON")?;
    Ok(pr.head.sha)
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    body: Option<String>,
}

/// Find an existing covrs comment on a PR (by our hidden marker).
fn find_existing_comment(token: &str, repo: &str, pr_number: u64) -> Result<Option<u64>> {
    let mut page = 1u32;
    loop {
        let url = format!(
            "https://api.github.com/repos/{repo}/issues/{pr_number}/comments?per_page=100&page={page}"
        );
        let resp = github_request("GET", &url, token)
            .call()
            .context("Failed to list PR comments")?;

        let comments: Vec<Comment> = resp.into_json().context("Failed to parse comments JSON")?;
        if comments.is_empty() {
            break;
        }
        for c in &comments {
            if let Some(ref body) = c.body {
                if body.contains(COMMENT_MARKER) {
                    return Ok(Some(c.id));
                }
            }
        }
        page += 1;
    }
    Ok(None)
}

/// Create or update the covrs diff-coverage comment on a PR.
fn post_comment(token: &str, repo: &str, pr_number: u64, body: &str) -> Result<()> {
    let body_with_marker = format!("{COMMENT_MARKER}\n{body}");

    match find_existing_comment(token, repo, pr_number)? {
        Some(comment_id) => {
            let url = format!("https://api.github.com/repos/{repo}/issues/comments/{comment_id}");
            let resp = github_request("PATCH", &url, token)
                .send_json(serde_json::json!({ "body": body_with_marker }));
            check_response(resp, "updating comment")?;
        }
        None => {
            let url = format!("https://api.github.com/repos/{repo}/issues/{pr_number}/comments");
            let resp = github_request("POST", &url, token)
                .send_json(serde_json::json!({ "body": body_with_marker }));
            check_response(resp, "creating comment")?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Check Runs (annotations)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CheckRun {
    id: u64,
}

/// GitHub Check Runs API annotation with the required `annotation_level`.
#[derive(serde::Serialize)]
struct CheckAnnotation<'a> {
    path: &'a str,
    start_line: u32,
    end_line: u32,
    annotation_level: &'static str,
    message: &'a str,
}

/// Convert annotations into the JSON format the GitHub Check Runs API expects.
fn annotations_to_json(annotations: &[Annotation]) -> Vec<serde_json::Value> {
    annotations
        .iter()
        .map(|a| {
            serde_json::to_value(CheckAnnotation {
                path: &a.path,
                start_line: a.start_line,
                end_line: a.end_line,
                annotation_level: "warning",
                message: &a.message,
            })
            .expect("annotation serialization is infallible")
        })
        .collect()
}

/// Create a check run with line-level annotations, submitting in chunks of 50.
///
/// The check run is created with the first batch, then subsequent batches are
/// added via PATCH requests. The final request sets the status to `completed`
/// with conclusion `neutral`.
fn post_check_run(token: &str, repo: &str, sha: &str, annotations: &[Annotation]) -> Result<()> {
    let url = format!("https://api.github.com/repos/{repo}/check-runs");
    let chunks: Vec<&[Annotation]> = if annotations.is_empty() {
        vec![&[]]
    } else {
        annotations.chunks(MAX_ANNOTATIONS_PER_REQUEST).collect()
    };

    let total_lines: u32 = annotations
        .iter()
        .map(|a| a.end_line - a.start_line + 1)
        .sum();
    let summary = if total_lines == 1 {
        "1 uncovered line".to_string()
    } else {
        format!("{total_lines} uncovered lines")
    };

    let is_single_request = chunks.len() == 1;

    // First request: create the check run
    let first_chunk = chunks[0];
    let first_annotations = annotations_to_json(first_chunk);

    let mut body = serde_json::json!({
        "name": "covrs",
        "head_sha": sha,
        "output": {
            "title": "Diff Coverage",
            "summary": summary,
            "annotations": first_annotations,
        },
    });

    if is_single_request {
        body["status"] = serde_json::json!("completed");
        body["conclusion"] = serde_json::json!("neutral");
    } else {
        body["status"] = serde_json::json!("in_progress");
    }

    let resp = check_response(
        github_request("POST", &url, token).send_json(body),
        "creating check run",
    )?;

    if !is_single_request {
        let check_run: CheckRun = resp
            .into_json()
            .context("Failed to parse check run response")?;

        let update_url = format!(
            "https://api.github.com/repos/{repo}/check-runs/{}",
            check_run.id
        );

        // Submit remaining chunks via PATCH
        for (i, chunk) in chunks[1..].iter().enumerate() {
            let is_last = i == chunks[1..].len() - 1;
            let chunk_annotations = annotations_to_json(chunk);

            let mut body = serde_json::json!({
                "output": {
                    "title": "Diff Coverage",
                    "summary": summary,
                    "annotations": chunk_annotations,
                },
            });

            if is_last {
                body["status"] = serde_json::json!("completed");
                body["conclusion"] = serde_json::json!("neutral");
            }

            check_response(
                github_request("PATCH", &update_url, token).send_json(body),
                "updating check run",
            )?;
        }
    }

    Ok(())
}
