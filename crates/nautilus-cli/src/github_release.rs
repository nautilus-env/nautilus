use anyhow::{Context, Result};
use reqwest::{
    blocking::Client,
    header::{ACCEPT, ETAG, IF_NONE_MATCH, USER_AGENT},
    StatusCode,
};
use semver::Version;
use serde::Deserialize;
use std::{fmt, time::Duration};

const USER_AGENT_VALUE: &str = "nautilus-cli";

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubRelease {
    pub(crate) tag_name: String,
    pub(crate) html_url: Option<String>,
    #[serde(default)]
    pub(crate) assets: Vec<GitHubReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct GitHubReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
}

#[derive(Debug)]
pub(crate) enum LatestReleaseResponse {
    NotModified,
    Found {
        release: GitHubRelease,
        etag: Option<String>,
    },
}

#[derive(Debug)]
pub(crate) struct GitHubLatestReleaseNotFound {
    repo: String,
}

impl fmt::Display for GitHubLatestReleaseNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "No published GitHub release was found for {}",
            latest_release_page_url(&self.repo)
        )
    }
}

impl std::error::Error for GitHubLatestReleaseNotFound {}

pub(crate) fn fetch_latest_release(
    repo: &str,
    timeout: Option<Duration>,
    etag: Option<&str>,
) -> Result<LatestReleaseResponse> {
    let mut client = Client::builder();
    if let Some(timeout) = timeout {
        client = client.timeout(timeout);
    }
    let client = client
        .build()
        .context("Failed to create HTTP client for GitHub release lookup")?;

    let mut request = client
        .get(latest_release_api_url(repo))
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/vnd.github+json");

    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }

    let response = request
        .send()
        .with_context(|| format!("Failed to request latest GitHub release for {repo}"))?;
    let status = response.status();

    if status == StatusCode::NOT_MODIFIED {
        return Ok(LatestReleaseResponse::NotModified);
    }

    if status == StatusCode::NOT_FOUND {
        return Err(GitHubLatestReleaseNotFound {
            repo: repo.to_string(),
        }
        .into());
    }

    let response = response
        .error_for_status()
        .with_context(|| format!("Could not resolve latest GitHub release for {repo}"))?;
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let release = response
        .json::<GitHubRelease>()
        .with_context(|| format!("Failed to decode latest GitHub release metadata for {repo}"))?;

    Ok(LatestReleaseResponse::Found { release, etag })
}

pub(crate) fn latest_release_page_url(repo: &str) -> String {
    format!("https://github.com/{repo}/releases/latest")
}

pub(crate) fn normalized_version_label(label: &str) -> Option<String> {
    parse_version_label(label).map(|version| version.to_string())
}

pub(crate) fn parse_version_label(label: &str) -> Option<Version> {
    let candidate = label
        .trim()
        .rsplit('/')
        .find(|part| !part.trim().is_empty())?
        .trim()
        .trim_start_matches(['v', 'V']);

    Version::parse(candidate).ok()
}

pub(crate) fn same_version(left: &str, right: &str) -> bool {
    match (parse_version_label(left), parse_version_label(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn latest_release_api_url(repo: &str) -> String {
    format!("https://api.github.com/repos/{repo}/releases/latest")
}

#[cfg(test)]
mod tests {
    use super::{normalized_version_label, parse_version_label, same_version};
    use semver::Version;

    #[test]
    fn version_label_normalization_accepts_common_release_tags() {
        assert_eq!(normalized_version_label("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(normalized_version_label("1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(
            normalized_version_label("releases/v1.2.3").as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn version_comparison_detects_newer_equal_and_older_versions() {
        let current = Version::parse("1.2.2").expect("current version");

        assert!(parse_version_label("v1.2.3").expect("latest version") > current);
        assert_eq!(parse_version_label("1.2.2").expect("same version"), current);
        assert!(parse_version_label("v1.2.1").expect("older version") < current);
    }

    #[test]
    fn same_version_ignores_optional_v_prefix() {
        assert!(same_version("0.1.0", "v0.1.0"));
        assert!(same_version("V0.1.0", "0.1.0"));
        assert!(!same_version("0.1.0", "v0.2.0"));
    }
}
