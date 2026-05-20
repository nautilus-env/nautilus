use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    github_release::{
        fetch_latest_release, latest_release_page_url, normalized_version_label,
        parse_version_label, LatestReleaseResponse,
    },
    local_paths, tui,
};

const GITHUB_REPO: &str = "nautilus-env/nautilus";
const DISABLE_ENV: &str = "NAUTILUS_NO_UPDATE_CHECK";
const CACHE_FILE_NAME: &str = "update-check.json";
const UPDATE_CHECK_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Deserialize, Serialize)]
struct UpdateCheckCache {
    checked_at_unix: u64,
    latest_tag: Option<String>,
    latest_version: Option<String>,
    release_url: Option<String>,
    etag: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UpdateNotice {
    current_version: Version,
    latest_tag: String,
    release_url: String,
}

pub fn check_for_update() {
    if disabled_by_env() {
        return;
    }

    let Some(notice) = resolve_update_notice(env!("CARGO_PKG_VERSION"), SystemTime::now()) else {
        return;
    };

    tui::eprint_warning(&format!(
        "A newer Nautilus version is available: {} -> {}. See {}",
        notice.current_version, notice.latest_tag, notice.release_url
    ));
}

fn resolve_update_notice(current_version: &str, now: SystemTime) -> Option<UpdateNotice> {
    let current = parse_version_label(current_version)?;
    let cache_path = cache_file_path().ok();
    let cached = cache_path.as_deref().and_then(read_cache);

    if let Some(notice) = cached
        .as_ref()
        .and_then(|cache| cached_notice_if_fresh(cache, &current, now))
    {
        return Some(notice);
    }

    let outcome = fetch_latest_release(
        GITHUB_REPO,
        Some(HTTP_TIMEOUT),
        cached.as_ref().and_then(|cache| cache.etag.as_deref()),
    )
    .ok()?;
    let cache = match outcome {
        LatestReleaseResponse::NotModified => {
            let mut cache = cached?;
            cache.checked_at_unix = unix_timestamp(now);
            cache
        }
        LatestReleaseResponse::Found { release, etag } => {
            let latest_version = normalized_version_label(&release.tag_name)?;
            UpdateCheckCache {
                checked_at_unix: unix_timestamp(now),
                latest_tag: Some(release.tag_name),
                latest_version: Some(latest_version),
                release_url: Some(
                    release
                        .html_url
                        .unwrap_or_else(|| latest_release_page_url(GITHUB_REPO)),
                ),
                etag,
            }
        }
    };

    if let Some(path) = cache_path.as_deref() {
        write_cache(path, &cache);
    }

    notice_from_cache(&cache, &current)
}

fn cached_notice_if_fresh(
    cache: &UpdateCheckCache,
    current: &Version,
    now: SystemTime,
) -> Option<UpdateNotice> {
    if !cache_is_fresh(cache, now) {
        return None;
    }

    notice_from_cache(cache, current)
}

fn notice_from_cache(cache: &UpdateCheckCache, current: &Version) -> Option<UpdateNotice> {
    let latest = cache
        .latest_version
        .as_deref()
        .or(cache.latest_tag.as_deref())
        .and_then(parse_version_label)?;

    if &latest <= current {
        return None;
    }

    Some(UpdateNotice {
        current_version: current.clone(),
        latest_tag: cache
            .latest_tag
            .clone()
            .unwrap_or_else(|| format!("v{latest}")),
        release_url: cache
            .release_url
            .clone()
            .unwrap_or_else(|| latest_release_page_url(GITHUB_REPO)),
    })
}

fn cache_is_fresh(cache: &UpdateCheckCache, now: SystemTime) -> bool {
    unix_timestamp(now).saturating_sub(cache.checked_at_unix) < UPDATE_CHECK_TTL.as_secs()
}

fn read_cache(path: &Path) -> Option<UpdateCheckCache> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
}

fn write_cache(path: &Path, cache: &UpdateCheckCache) {
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(contents) = serde_json::to_string_pretty(cache) else {
        return;
    };
    let _ = fs::write(path, contents);
}

fn cache_file_path() -> Result<PathBuf, ()> {
    local_paths::nautilus_home()
        .map(|path| path.join(CACHE_FILE_NAME))
        .map_err(|_| ())
}

fn disabled_by_env() -> bool {
    let Ok(value) = env::var(DISABLE_ENV) else {
        return false;
    };

    is_truthy_opt_out(&value)
}

fn is_truthy_opt_out(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn unix_timestamp(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::{
        cache_is_fresh, cached_notice_if_fresh, is_truthy_opt_out, UpdateCheckCache,
        UPDATE_CHECK_TTL,
    };
    use crate::github_release::normalized_version_label;
    use semver::Version;
    use std::time::{Duration, UNIX_EPOCH};

    fn cache(checked_at_unix: u64, latest_tag: &str) -> UpdateCheckCache {
        UpdateCheckCache {
            checked_at_unix,
            latest_tag: Some(latest_tag.to_string()),
            latest_version: normalized_version_label(latest_tag),
            release_url: Some(
                "https://github.com/nautilus-env/nautilus/releases/tag/v1.3.0".into(),
            ),
            etag: Some(r#""release-etag""#.into()),
        }
    }

    #[test]
    fn fresh_cache_can_produce_update_notice_without_network() {
        let now = UNIX_EPOCH + Duration::from_secs(100_000);
        let current = Version::parse("1.2.2").expect("current version");
        let cache = cache(99_900, "v1.3.0");

        let notice =
            cached_notice_if_fresh(&cache, &current, now).expect("fresh cache should be used");

        assert_eq!(notice.latest_tag, "v1.3.0");
    }

    #[test]
    fn stale_cache_does_not_produce_cached_notice() {
        let now = UNIX_EPOCH + Duration::from_secs(100_000);
        let checked_at = 100_000 - UPDATE_CHECK_TTL.as_secs() - 1;
        let current = Version::parse("1.2.2").expect("current version");
        let cache = cache(checked_at, "v1.3.0");

        assert!(!cache_is_fresh(&cache, now));
        assert!(cached_notice_if_fresh(&cache, &current, now).is_none());
    }

    #[test]
    fn env_opt_out_accepts_truthy_values() {
        assert!(is_truthy_opt_out("1"));
        assert!(is_truthy_opt_out("true"));
        assert!(is_truthy_opt_out("yes"));
        assert!(is_truthy_opt_out("on"));
        assert!(!is_truthy_opt_out(""));
        assert!(!is_truthy_opt_out("false"));
    }
}
