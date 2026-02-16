//! GitHub API helpers for posting patch-coverage comments on pull requests.

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;

const COMMENT_MARKER: &str = "<!-- covrs-comment -->";

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
    let url = format!("https://api.github.com/repos/{}/pulls/{}", repo, pr_number);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .set("Accept", "application/vnd.github.v3.diff")
        .set("User-Agent", "covrs")
        .set("X-GitHub-Api-Version", "2022-11-28")
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
            "https://api.github.com/repos/{}/issues/{}/comments?per_page=100&page={}",
            repo, pr_number, page
        );
        let resp = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", token))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "covrs")
            .set("X-GitHub-Api-Version", "2022-11-28")
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

/// Create or update the covrs patch-coverage comment on a PR.
fn post_comment(token: &str, repo: &str, pr_number: u64, body: &str) -> Result<()> {
    let body_with_marker = format!("{}\n{}", COMMENT_MARKER, body);

    match find_existing_comment(token, repo, pr_number)? {
        Some(comment_id) => {
            let url = format!(
                "https://api.github.com/repos/{}/issues/comments/{}",
                repo, comment_id
            );
            let resp = ureq::patch(&url)
                .set("Authorization", &format!("Bearer {}", token))
                .set("Accept", "application/vnd.github+json")
                .set("User-Agent", "covrs")
                .set("X-GitHub-Api-Version", "2022-11-28")
                .send_json(serde_json::json!({ "body": body_with_marker }));
            match resp {
                Ok(_) => {}
                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    bail!(
                        "GitHub API error updating comment (HTTP {}): {}",
                        code,
                        body
                    );
                }
                Err(e) => bail!("Failed to update comment: {}", e),
            }
        }
        None => {
            let url = format!(
                "https://api.github.com/repos/{}/issues/{}/comments",
                repo, pr_number
            );
            let resp = ureq::post(&url)
                .set("Authorization", &format!("Bearer {}", token))
                .set("Accept", "application/vnd.github+json")
                .set("User-Agent", "covrs")
                .set("X-GitHub-Api-Version", "2022-11-28")
                .send_json(serde_json::json!({ "body": body_with_marker }));
            match resp {
                Ok(_) => {}
                Err(ureq::Error::Status(code, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    bail!(
                        "GitHub API error creating comment (HTTP {}): {}",
                        code,
                        body
                    );
                }
                Err(e) => bail!("Failed to create comment: {}", e),
            }
        }
    }

    Ok(())
}
