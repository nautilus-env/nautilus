use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use std::{fmt, path::Path, time::Duration};

use crate::{
    github_release::{
        fetch_latest_release as fetch_github_latest_release, latest_release_page_url,
        GitHubLatestReleaseNotFound, GitHubRelease, GitHubReleaseAsset, LatestReleaseResponse,
    },
    tui,
};

use super::{STUDIO_GITHUB_REPO, STUDIO_RELEASE_ASSET_PREFIX};

#[derive(Debug)]
pub(super) struct StudioReleaseUnavailable {
    pub(super) detail: String,
}

impl StudioReleaseUnavailable {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for StudioReleaseUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for StudioReleaseUnavailable {}

fn fetch_latest_release() -> Result<GitHubRelease> {
    match fetch_github_latest_release(STUDIO_GITHUB_REPO, None, None) {
        Ok(LatestReleaseResponse::Found { release, .. }) => Ok(release),
        Ok(LatestReleaseResponse::NotModified) => {
            bail!("GitHub returned 304 Not Modified without an ETag request")
        }
        Err(err) if err.downcast_ref::<GitHubLatestReleaseNotFound>().is_some() => {
            Err(StudioReleaseUnavailable::new(format!(
                "No published Nautilus Studio release was found for {}. Publish a release first or update STUDIO_GITHUB_REPO.",
                latest_release_page_url(STUDIO_GITHUB_REPO)
            ))
            .into())
        }
        Err(err) => Err(err)
            .context("Could not resolve or decode the latest Nautilus Studio release metadata"),
    }
}

pub(super) fn latest_release_asset() -> Result<GitHubReleaseAsset> {
    let release = fetch_latest_release()?;
    let asset = select_release_asset(&release)?;
    Ok(asset)
}

fn select_release_asset(release: &GitHubRelease) -> Result<GitHubReleaseAsset> {
    select_release_asset_for_platform(release, release_asset_platform())
}

fn select_release_asset_for_platform(
    release: &GitHubRelease,
    platform: &str,
) -> Result<GitHubReleaseAsset> {
    let expected_name = expected_release_asset_name_for_platform(&release.tag_name, platform);

    release
        .assets
        .iter()
        .find(|asset| asset.name == expected_name)
        .cloned()
        .ok_or_else(|| {
            StudioReleaseUnavailable::new(format!(
                "The latest Nautilus Studio release for {} does not include the expected asset `{}` for the current platform ({})",
                STUDIO_GITHUB_REPO, expected_name, platform,
            ))
            .into()
        })
}

fn expected_release_asset_name_for_platform(tag_name: &str, platform: &str) -> String {
    format!(
        "{}{}-{}.zip",
        STUDIO_RELEASE_ASSET_PREFIX, tag_name, platform
    )
}

fn release_asset_platform() -> &'static str {
    std::env::consts::OS
}

pub(super) fn fetch_latest_release_tag_silently() -> Option<String> {
    match fetch_github_latest_release(STUDIO_GITHUB_REPO, Some(Duration::from_secs(5)), None)
        .ok()?
    {
        LatestReleaseResponse::Found { release, .. } => Some(release.tag_name),
        LatestReleaseResponse::NotModified => None,
    }
}

pub(super) fn download_release_archive(url: &str, archive_path: &Path) -> Result<()> {
    let client = Client::builder()
        .build()
        .context("Failed to create HTTP client for Studio download")?;

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, "nautilus-cli")
        .send()
        .with_context(|| format!("Failed to request {}", url))?;

    let status = response.status();
    if !status.is_success() {
        bail!(
            "Could not download Nautilus Studio from {} (HTTP {}). The release asset may not exist yet.",
            url,
            status
        );
    }

    let bytes = response
        .bytes()
        .with_context(|| format!("Failed to read response body from {}", url))?;

    std::fs::write(archive_path, &bytes)
        .with_context(|| format!("Failed to write {}", archive_path.display()))?;

    tui::print_ok(&format!("Downloaded {}", archive_path.display()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_release_asset_name_matches_workflow() {
        assert_eq!(
            expected_release_asset_name_for_platform("v0.1.0", "windows"),
            "nautilus-orm-studio-v0.1.0-windows.zip"
        );
    }

    #[test]
    fn release_asset_selection_matches_release_workflow_naming() {
        let release = GitHubRelease {
            tag_name: "v0.1.0".to_string(),
            html_url: None,
            assets: vec![
                GitHubReleaseAsset {
                    name: "checksums.txt".to_string(),
                    browser_download_url: "https://example.com/checksums.txt".to_string(),
                },
                GitHubReleaseAsset {
                    name: "nautilus-orm-studio-v0.1.0-windows.zip".to_string(),
                    browser_download_url:
                        "https://example.com/nautilus-orm-studio-v0.1.0-windows.zip".to_string(),
                },
            ],
        };

        let asset = select_release_asset_for_platform(&release, "windows").expect("studio asset");
        assert_eq!(asset.name, "nautilus-orm-studio-v0.1.0-windows.zip");
    }

    #[test]
    fn release_asset_selection_uses_current_platform() {
        let release = GitHubRelease {
            tag_name: "v0.1.0".to_string(),
            html_url: None,
            assets: vec![
                GitHubReleaseAsset {
                    name: "nautilus-orm-studio-v0.1.0-linux.zip".to_string(),
                    browser_download_url:
                        "https://example.com/nautilus-orm-studio-v0.1.0-linux.zip".to_string(),
                },
                GitHubReleaseAsset {
                    name: "nautilus-orm-studio-v0.1.0-macos.zip".to_string(),
                    browser_download_url:
                        "https://example.com/nautilus-orm-studio-v0.1.0-macos.zip".to_string(),
                },
                GitHubReleaseAsset {
                    name: "nautilus-orm-studio-v0.1.0-windows.zip".to_string(),
                    browser_download_url:
                        "https://example.com/nautilus-orm-studio-v0.1.0-windows.zip".to_string(),
                },
            ],
        };

        let asset = select_release_asset(&release).expect("studio asset");
        assert_eq!(
            asset.name,
            format!(
                "nautilus-orm-studio-v0.1.0-{}.zip",
                release_asset_platform()
            )
        );
    }

    #[test]
    fn release_asset_selection_reports_missing_zip() {
        let release = GitHubRelease {
            tag_name: "v0.1.0".to_string(),
            html_url: None,
            assets: vec![GitHubReleaseAsset {
                name: "checksums.txt".to_string(),
                browser_download_url: "https://example.com/checksums.txt".to_string(),
            }],
        };

        let err = select_release_asset(&release).expect_err("missing asset should fail");
        assert!(err.downcast_ref::<StudioReleaseUnavailable>().is_some());
        assert!(err.to_string().contains(STUDIO_GITHUB_REPO));
    }
}
