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
        assert_eq!(
            UPDATE_CHECK_URL,
            "https://northboundwallet.org/release.json"
        );
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
