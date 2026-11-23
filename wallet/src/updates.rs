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
mod tests {
    use serde_json::json;

    use super::{
        AvailableUpdate, MAX_RELEASE_METADATA_BYTES, ReleaseMetadata, Version, evaluate_release,
        fetch_release_metadata,
    };
    use crate::config::{GITHUB_RELEASES_TAG_BASE_URL, UPDATE_CHECK_URL};
    use crate::server::test_support::serve;

    #[test]
    fn semver_ordering_handles_prereleases_and_numeric_identifiers() {
        let ordered = [
            "0.1.0-rc.9",
            "0.1.0-rc.10",
            "0.1.0",
            "0.2.0-alpha.4",
            "0.2.0-alpha.5",
            "0.2.0-alpha.10",
            "0.2.0-beta.1",
            "0.2.0-rc.1",
            "0.2.0",
            "0.2.1",
            "1.0.0",
        ];

        for pair in ordered.windows(2) {
            let older = Version::parse(pair[0]).unwrap();
            let newer = Version::parse(pair[1]).unwrap();
            assert!(older < newer, "expected {} < {}", pair[0], pair[1]);
        }

        assert_eq!(
            Version::parse("v0.2.0-alpha.4").unwrap(),
            Version::parse("0.2.0-alpha.4").unwrap()
        );
    }

    #[test]
    fn invalid_versions_are_rejected() {
        for invalid in [
            "",
            "v",
            "0.2",
            "0.2.0.1",
            "vv0.2.0",
            "+0.2.0",
            "0.2.0-",
            "0.2.0-alpha..1",
            "0.2.0+build1",
            "0.2.0/evil",
        ] {
            assert!(
                Version::parse(invalid).is_err(),
                "expected invalid version: {invalid}"
            );
        }
    }

    #[test]
    fn prerelease_builds_prompt_for_newer_prereleases_and_stable_releases() {
        let newer_alpha = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.5".to_owned(),
                prerelease: true,
            },
            None,
        )
        .unwrap();
        assert_eq!(
            newer_alpha,
            Some(AvailableUpdate {
                version: "0.2.0-alpha.5".to_owned(),
                tag: "v0.2.0-alpha.5".to_owned(),
                prerelease: true,
                release_url: format!("{GITHUB_RELEASES_TAG_BASE_URL}/v0.2.0-alpha.5"),
            })
        );

        let stable = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0".to_owned(),
                prerelease: false,
            },
            None,
        )
        .unwrap();
        assert_eq!(
            stable,
            Some(AvailableUpdate {
                version: "0.2.0".to_owned(),
                tag: "v0.2.0".to_owned(),
                prerelease: false,
                release_url: format!("{GITHUB_RELEASES_TAG_BASE_URL}/v0.2.0"),
            })
        );
    }

    #[test]
    fn stable_builds_ignore_prereleases_and_prompt_for_newer_stable_releases() {
        let prerelease = evaluate_release(
            "0.2.0",
            ReleaseMetadata {
                tag: "v0.3.0-alpha.1".to_owned(),
                prerelease: true,
            },
            None,
        )
        .unwrap();
        assert_eq!(prerelease, None);

        let untagged_prerelease = evaluate_release(
            "0.2.0",
            ReleaseMetadata {
                tag: "v0.3.0-rc.1".to_owned(),
                prerelease: false,
            },
            None,
        )
        .unwrap();
        assert_eq!(untagged_prerelease, None);

        let stable = evaluate_release(
            "0.2.0",
            ReleaseMetadata {
                tag: "v0.2.1".to_owned(),
                prerelease: false,
            },
            None,
        )
        .unwrap();
        assert_eq!(
            stable.map(|update| update.version),
            Some("0.2.1".to_owned())
        );
    }

    #[test]
    fn equal_older_and_skipped_versions_do_not_prompt() {
        let same = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.4".to_owned(),
                prerelease: true,
            },
            None,
        )
        .unwrap();
        assert_eq!(same, None);

        let older = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.3".to_owned(),
                prerelease: true,
            },
            None,
        )
        .unwrap();
        assert_eq!(older, None);

        let skipped = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.5".to_owned(),
                prerelease: true,
            },
            Some("0.2.0-alpha.5"),
        )
        .unwrap();
        assert_eq!(skipped, None);

        let skipped_with_v = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.5".to_owned(),
                prerelease: true,
            },
            Some("v0.2.0-alpha.5"),
        )
        .unwrap();
        assert_eq!(skipped_with_v, None);

        let older_than_skipped = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.5".to_owned(),
                prerelease: true,
            },
            Some("0.2.0-alpha.6"),
        )
        .unwrap();
        assert_eq!(older_than_skipped, None);

        let newer_than_skipped = evaluate_release(
            "0.2.0-alpha.4",
            ReleaseMetadata {
                tag: "v0.2.0-alpha.6".to_owned(),
                prerelease: true,
            },
            Some("0.2.0-alpha.5"),
        )
        .unwrap();
        assert_eq!(
            newer_than_skipped.map(|update| update.version),
            Some("0.2.0-alpha.6".to_owned())
        );
    }

    #[test]
    fn unsafe_release_tags_are_rejected() {
        for tag in [
            "../v0.3.0",
            "v0.3.0?redirect=evil",
            "v0.3.0#frag",
            "v0.3.0 evil",
        ] {
            let result = evaluate_release(
                "0.2.0",
                ReleaseMetadata {
                    tag: tag.to_owned(),
                    prerelease: false,
                },
                None,
            );
            assert!(result.is_err(), "expected unsafe tag to fail: {tag}");
        }
    }

    #[test]
    fn update_check_url_matches_the_selected_network() {
        if cfg!(feature = "regtest") && option_env!("NORTHBOUND_UPDATE_URL").is_none() {
            assert_eq!(UPDATE_CHECK_URL, "http://127.0.0.1:3001/release.json");
        }
        if cfg!(feature = "mainnet") && option_env!("NORTHBOUND_UPDATE_URL").is_none() {
            assert_eq!(UPDATE_CHECK_URL, "https://northboundwallet.org/release.json");
        }
    }

    #[tokio::test]
    async fn fetches_release_metadata_and_handles_http_and_size_errors() {
        let (url, server) = serve(1, |_, _| {
            (200, json!({"tag": "v0.2.0-alpha.5", "prerelease": true}))
        });
        let metadata = fetch_release_metadata(&format!("{url}/release.json"))
            .await
            .unwrap();
        assert_eq!(metadata.tag, "v0.2.0-alpha.5");
        assert!(metadata.prerelease);
        assert_eq!(
            server.join().unwrap(),
            [json!("GET /release.json HTTP/1.1")]
        );

        let (url, server) = serve(1, |_, _| (404, json!({})));
        assert!(
            fetch_release_metadata(&format!("{url}/release.json"))
                .await
                .is_err()
        );
        server.join().unwrap();

        let oversized = "a".repeat(MAX_RELEASE_METADATA_BYTES + 1);
        let (url, server) = serve(1, move |_, _| {
            (200, json!({"tag": oversized, "prerelease": false}))
        });
        assert!(
            fetch_release_metadata(&format!("{url}/release.json"))
                .await
                .is_err()
        );
        server.join().unwrap();
    }
}
