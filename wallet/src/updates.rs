use std::cmp::Ordering;
use std::time::Duration;

use serde::Deserialize;

use crate::config::{GITHUB_RELEASES_TAG_BASE_URL, UPDATE_CHECK_URL};
use crate::storage::settings;

const UPDATE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RELEASE_METADATA_BYTES: usize = 16 * 1024;
const MAX_RELEASE_TAG_LENGTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableUpdate {
    pub(crate) version: String,
    pub(crate) tag: String,
    pub(crate) prerelease: bool,
    pub(crate) release_url: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseMetadata {
    tag: String,
    #[serde(default)]
    prerelease: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Vec<PrereleaseIdentifier>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleaseIdentifier {
    Number(u64),
    Text(String),
}

impl Ord for PrereleaseIdentifier {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => left.cmp(right),
            (Self::Number(_), Self::Text(_)) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) => Ordering::Greater,
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for PrereleaseIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let core_order =
            (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        if core_order != Ordering::Equal {
            return core_order;
        }

        // Per SemVer 2.0.0, a stable release has higher precedence than a prerelease.
        match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.prerelease.cmp(&other.prerelease),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Version {
    fn parse(raw: &str) -> Result<Self, String> {
        let text = raw.trim();
        let without_prefix = text.strip_prefix('v').unwrap_or(text);
        if without_prefix.is_empty() || without_prefix.contains('+') {
            return Err(format!("invalid release version: {raw}"));
        }

        let (core_part, prerelease_part) = match without_prefix.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (without_prefix, None),
        };

        let core_numbers: Vec<&str> = core_part.split('.').collect();
        if core_numbers.len() != 3 {
            return Err(format!("version must have major.minor.patch: {raw}"));
        }

        let major = parse_numeric_part(core_numbers[0], raw)?;
        let minor = parse_numeric_part(core_numbers[1], raw)?;
        let patch = parse_numeric_part(core_numbers[2], raw)?;

        let mut prerelease = Vec::new();
        if let Some(pre) = prerelease_part {
            if pre.is_empty() {
                return Err(format!("prerelease version cannot be empty: {raw}"));
            }
            for part in pre.split('.') {
                if part.is_empty()
                    || !part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                {
                    return Err(format!("invalid prerelease identifier in version: {raw}"));
                }

                if part.bytes().all(|byte| byte.is_ascii_digit()) {
                    let number = part
                        .parse::<u64>()
                        .map_err(|_| format!("invalid prerelease number in version: {raw}"))?;
                    prerelease.push(PrereleaseIdentifier::Number(number));
                } else {
                    prerelease.push(PrereleaseIdentifier::Text(part.to_owned()));
                }
            }
        }

        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    fn is_prerelease(&self) -> bool {
        !self.prerelease.is_empty()
    }
}

fn parse_numeric_part(part: &str, full_version: &str) -> Result<u64, String> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("invalid version number: {full_version}"));
    }

    part.parse::<u64>()
        .map_err(|_| format!("version number is out of range: {full_version}"))
}

fn is_safe_release_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_RELEASE_TAG_LENGTH
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn evaluate_release(
    current_version_str: &str,
    release: ReleaseMetadata,
    skipped_version: Option<&str>,
) -> Result<Option<AvailableUpdate>, String> {
    if !is_safe_release_tag(&release.tag) {
        return Err("Release tag contains unsupported characters.".to_owned());
    }

    let current = Version::parse(current_version_str)?;
    let latest = Version::parse(&release.tag)?;
    let release_is_prerelease = release.prerelease || latest.is_prerelease();

    // Stable builds should not prompt users to install prereleases.
    if !current.is_prerelease() && release_is_prerelease {
        return Ok(None);
    }

    if latest <= current {
        return Ok(None);
    }

    if let Some(skipped) = skipped_version
        && Version::parse(skipped).is_ok_and(|skipped| latest <= skipped)
    {
        return Ok(None);
    }

    let normalized_version = release
        .tag
        .strip_prefix('v')
        .unwrap_or(&release.tag)
        .to_owned();
    let release_url = format!("{GITHUB_RELEASES_TAG_BASE_URL}/{}", release.tag);

    Ok(Some(AvailableUpdate {
        version: normalized_version,
        tag: release.tag,
        prerelease: release_is_prerelease,
        release_url,
    }))
}

async fn fetch_release_metadata(url: &str) -> Result<ReleaseMetadata, String> {
    let client = reqwest::Client::builder()
        .timeout(UPDATE_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Northbound/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("could not create update check client: {error}"))?;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("could not fetch update metadata: {error}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "update metadata request returned HTTP {}",
            response.status()
        ));
    }

    if let Some(length) = response.content_length()
        && length > MAX_RELEASE_METADATA_BYTES as u64
    {
        return Err("update metadata response is too large".to_owned());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("could not read update metadata: {error}"))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RELEASE_METADATA_BYTES {
            return Err("update metadata response is too large".to_owned());
        }
        body.extend_from_slice(&chunk);
    }

    serde_json::from_slice(&body)
        .map_err(|error| format!("update metadata JSON is invalid: {error}"))
}

pub(crate) async fn check_for_update() -> Option<AvailableUpdate> {
    let skipped = settings::skipped_update_version().ok().flatten();
    let release = fetch_release_metadata(UPDATE_CHECK_URL).await.ok()?;

    evaluate_release(env!("CARGO_PKG_VERSION"), release, skipped.as_deref())
        .ok()
        .flatten()
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod tests;
