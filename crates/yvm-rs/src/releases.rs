use once_cell::sync::Lazy;
use semver::Version;
use serde::{
    de::{self, Deserializer},
    Deserialize, Serialize,
};
use std::collections::BTreeMap;
use url::Url;

use crate::{error::YlemVmError, platform::Platform};

const YLEM_RELEASES_URL: &str = "https://github.com/core-coin/ylem/releases/download";

static YLEM_AARCH_RELEASES: Lazy<Releases> = Lazy::new(|| {
    serde_json::from_str(include_str!("../list/arm/list.json"))
        .expect("Couldn't parse ylem releases")
});

static YLEM_AMD_RELEASES: Lazy<Releases> = Lazy::new(|| {
    serde_json::from_str(include_str!("../list/x86/list.json"))
        .expect("Couldn't parse ylem releases")
});

/// Defines the struct that the JSON-formatted release list can be deserialized into.
///
/// {
///     "builds": [
///         {
///             "version": "0.8.7",
///             "sha256": "0x0xcc5c663d1fe17d4eb4aca09253787ac86b8785235fca71d9200569e662677990"
///         }
///     ]
///     "releases": {
///         "0.8.7": "ylem-macosx-amd64-v0.8.7+commit.e28d00a7",
///         "0.8.6": "ylem-macosx-amd64-v0.8.6+commit.11564f7e",
///         ...
///     }
/// }
///
/// Both the key and value are deserialized into semver::Version.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Releases {
    pub builds: Vec<BuildInfo>,
    pub releases: BTreeMap<Version, String>,
}

impl Releases {
    /// Get the checksum of a ylem version's binary if it exists.
    pub fn get_checksum(&self, v: &Version) -> Option<Vec<u8>> {
        for build in self.builds.iter() {
            if build.version.eq(v) {
                return Some(build.sha256.clone());
            }
        }
        None
    }

    /// Returns the artifact of the version if any
    pub fn get_artifact(&self, version: &Version) -> Option<&String> {
        self.releases.get(version)
    }

    /// Returns a sorted list of all versions
    pub fn into_versions(self) -> Vec<Version> {
        let mut versions = self.releases.into_keys().collect::<Vec<_>>();
        versions.sort_unstable();
        versions
    }
}

/// Build info contains the SHA256 checksum of a ylem binary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: Version,
    #[serde(with = "hex_string")]
    pub sha256: Vec<u8>,
}

/// Helper serde module to serialize and deserialize bytes as hex.
mod hex_string {
    use super::*;
    use serde::Serializer;
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let str_hex = String::deserialize(deserializer)?;
        let str_hex = str_hex.trim_start_matches("0x");
        hex::decode(str_hex).map_err(|err| de::Error::custom(err.to_string()))
    }

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: AsRef<[u8]>,
    {
        let value = hex::encode(value);
        serializer.serialize_str(&value)
    }
}

/// Blocking version for [`all_realeases`]
#[cfg(feature = "blocking")]
pub fn blocking_all_releases(platform: Platform) -> Result<Releases, YlemVmError> {
    if platform == Platform::LinuxAarch64 {
        Ok(YLEM_AARCH_RELEASES.clone())
    } else if platform == Platform::LinuxAmd64 {
        Ok(YLEM_AMD_RELEASES.clone())
    } else {
        Err(YlemVmError::UnsupportedPlatform(platform))
    }
}

/// Fetch all releases available for the provided platform.
pub async fn all_releases(platform: Platform) -> Result<Releases, YlemVmError> {
    if platform == Platform::LinuxAarch64 {
        Ok(YLEM_AARCH_RELEASES.clone())
    } else if platform == Platform::LinuxAmd64 {
        Ok(YLEM_AMD_RELEASES.clone())
    } else {
        Err(YlemVmError::UnsupportedPlatform(platform))
    }
}

/// Construct the URL to the Ylem binary for the specified release version and target platform.
pub fn artifact_url(
    platform: Platform,
    version: &Version,
    artifact: &str,
) -> Result<Url, YlemVmError> {
    if platform == Platform::LinuxAmd64 || platform == Platform::LinuxAarch64 {
        return Ok(Url::parse(&format!(
            "{YLEM_RELEASES_URL}/{version}/{artifact}"
        ))?);
    }

    Err(YlemVmError::UnsupportedPlatform(platform))
}
