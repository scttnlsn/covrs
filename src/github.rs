//! GitHub API helpers for posting diff-coverage comments on pull requests.

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;

const COMMENT_MARKER: &str = "<!-- covrs-comment -->";

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
    /// (`GITHUB_TOKEN`, `GITHUB_REPOSITORY`, `GITHUB_REF`, `GITHUB_SHA`).
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN")
            .context("GITHUB_TOKEN environment variable is required")?;
        let repo = std::env::var("GITHUB_REPOSITORY")
            .context("GITHUB_REPOSITORY environment variable is required")?;
        let pr_number =
            pr_number_from_ref().context("could not determine PR number from GITHUB_REF")?;
        let sha = std::env::var("GITHUB_SHA").ok();
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
    let resp = github_request("GET", &url, token)
        .set("Accept", "application/vnd.github.v3.diff")
        .call()
        .context("Failed to fetch PR diff from GitHub")?;
    resp.into_string()
        .context("Failed to read PR diff response body")
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
