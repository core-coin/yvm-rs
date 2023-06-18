use once_cell::sync::Lazy;
use semver::{Version, VersionReq};
use sha2::Digest;

use std::{
    ffi::OsString,
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    process::Command,
};

use std::time::Duration;
/// Use permissions extensions on unix
#[cfg(target_family = "unix")]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

mod error;
pub use error::YlemVmError;

mod platform;
pub use platform::{platform, Platform};

mod releases;
pub use releases::{all_releases, Releases};

#[cfg(feature = "blocking")]
pub use releases::blocking_all_releases;

/// Declare path to Ylem Version Manager's home directory
/// On unix-based machines, if "~/.yvm" already exists, then keep using it.
/// Otherwise, use $XDG_DATA_HOME or ~/.local/share/yvm
pub static YVM_DATA_DIR: Lazy<PathBuf> = Lazy::new(|| {
    #[cfg(test)]
    {
        let dir = tempfile::tempdir().expect("could not create temp directory");
        dir.path().join(".yvm")
    }
    #[cfg(not(test))]
    {
        resolve_data_dir()
    }
});

fn resolve_data_dir() -> PathBuf {
    let home_dir = dirs::home_dir()
        .expect("could not detect user home directory")
        .join(".yvm");

    let data_dir = dirs::data_dir().expect("could not detect user data directory");
    if !home_dir.as_path().exists() && data_dir.as_path().exists() {
        data_dir.join("yvm")
    } else {
        home_dir
    }
}

/// The timeout to use for requests to the source
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Version beyond which ylem binaries are not fully static, hence need to be patched for NixOS.
static NIXOS_PATCH_REQ: Lazy<VersionReq> = Lazy::new(|| VersionReq::parse(">=0.7.6").unwrap());

// Installer type that copies binary data to the appropriate ylem binary file:
// 1. create target file to copy binary data
// 2. copy data
struct Installer {
    // version of ylem
    version: Version,
    // binary data of the ylem executable
    binbytes: Vec<u8>,
}

impl Installer {
    /// Installs the ylem version at the version specific destination and returns the path to the installed ylem file.
    fn install(&self) -> Result<PathBuf, YlemVmError> {
        let version_path = version_path(self.version.to_string().as_str());
        let ylem_path = version_path.join(format!("ylem-{}", self.version));
        // create ylem file.
        let mut f = fs::File::create(&ylem_path)?;

        #[cfg(target_family = "unix")]
        f.set_permissions(Permissions::from_mode(0o777))?;

        // copy contents over
        let mut content = Cursor::new(&self.binbytes);
        std::io::copy(&mut content, &mut f)?;

        if platform::is_nixos() && NIXOS_PATCH_REQ.matches(&self.version) {
            patch_for_nixos(ylem_path)
        } else {
            Ok(ylem_path)
        }
    }

    /// Extracts the ylem archive at the version specified destination and returns the path to the
    /// installed ylem binary.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn install_zip(&self) -> Result<PathBuf, YlemVmError> {
        let version_path = version_path(self.version.to_string().as_str());
        let ylem_path = version_path.join(&format!("ylem-{}", self.version));

        // extract archive
        let mut content = Cursor::new(&self.binbytes);
        let mut archive = zip::ZipArchive::new(&mut content)?;
        archive.extract(version_path.as_path())?;

        // rename ylem binary
        std::fs::rename(version_path.join("ylem.exe"), ylem_path.as_path())?;

        Ok(ylem_path)
    }
}

/// Patch the given binary to use the dynamic linker provided by nixos
pub fn patch_for_nixos(bin: PathBuf) -> Result<PathBuf, YlemVmError> {
    let output = Command::new("nix-shell")
        .arg("-p")
        .arg("patchelf")
        .arg("--run")
        .arg(format!(
            "patchelf --set-interpreter \"$(cat $NIX_CC/nix-support/dynamic-linker)\" {}",
            bin.display()
        ))
        .output()
        .expect("Failed to execute command");

    match output.status.success() {
        true => Ok(bin),
        false => Err(YlemVmError::CouldNotPatchForNixOs(
            String::from_utf8(output.stdout).expect("Found invalid UTF-8 when parsing stdout"),
            String::from_utf8(output.stderr).expect("Found invalid UTF-8 when parsing stderr"),
        )),
    }
}

/// Derive path to a specific Ylem version's binary.
pub fn version_path(version: &str) -> PathBuf {
    let mut version_path = YVM_DATA_DIR.to_path_buf();
    version_path.push(version);
    version_path
}

/// Derive path to YVM's global version file.
pub fn global_version_path() -> PathBuf {
    let mut global_version_path = YVM_DATA_DIR.to_path_buf();
    global_version_path.push(".global-version");
    global_version_path
}

/// Reads the currently set global version for Ylem. Returns None if none has yet been set.
pub fn current_version() -> Result<Option<Version>, YlemVmError> {
    let v = fs::read_to_string(global_version_path().as_path())?;
    Ok(Version::parse(v.trim_end_matches('\n').to_string().as_str()).ok())
}

/// Sets the provided version as the global version for Ylem.
pub fn use_version(version: &Version) -> Result<(), YlemVmError> {
    let mut v = fs::File::create(global_version_path().as_path())?;
    v.write_all(version.to_string().as_bytes())?;
    Ok(())
}

/// Unset the global version. This should be done if all versions are removed.
pub fn unset_global_version() -> Result<(), YlemVmError> {
    let mut v = fs::File::create(global_version_path().as_path())?;
    v.write_all("".as_bytes())?;
    Ok(())
}

/// Reads the list of Ylem versions that have been installed in the machine. The version list is
/// sorted in ascending order.
pub fn installed_versions() -> Result<Vec<Version>, YlemVmError> {
    let home_dir = YVM_DATA_DIR.to_path_buf();
    let mut versions = vec![];
    for v in fs::read_dir(home_dir)? {
        let v = v?;
        if v.file_name() != OsString::from(".global-version".to_string()) {
            versions.push(Version::parse(
                v.path()
                    .file_name()
                    .ok_or(YlemVmError::UnknownVersion)?
                    .to_str()
                    .ok_or(YlemVmError::UnknownVersion)?
                    .to_string()
                    .as_str(),
            )?);
        }
    }
    versions.sort();
    Ok(versions)
}

/// Blocking version of [`all_versions`]
#[cfg(feature = "blocking")]
pub fn blocking_all_versions() -> Result<Vec<Version>, YlemVmError> {
    Ok(releases::blocking_all_releases(platform::platform())?.into_versions())
}

/// Fetches the list of all the available versions of YLem. The list is platform dependent, so
/// different versions can be found for macosx vs linux.
pub async fn all_versions() -> Result<Vec<Version>, YlemVmError> {
    Ok(releases::all_releases(platform::platform())
        .await?
        .into_versions())
}

/// Blocking version of [`install`]
#[cfg(feature = "blocking")]
pub fn blocking_install(version: &Version) -> Result<PathBuf, YlemVmError> {
    setup_data_dir()?;

    let artifacts = releases::blocking_all_releases(platform::platform())?;
    let artifact = artifacts
        .get_artifact(version)
        .ok_or(YlemVmError::UnknownVersion)?;
    let download_url =
        releases::artifact_url(platform::platform(), version, artifact.to_string().as_str())?;

    let checksum = artifacts
        .get_checksum(version)
        .unwrap_or_else(|| panic!("checksum not available: {:?}", version.to_string()));

    let res = reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("reqwest::Client::new()")
        .get(download_url.clone())
        .send()?;

    if !res.status().is_success() {
        return Err(YlemVmError::UnsuccessfulResponse(
            download_url,
            res.status(),
        ));
    }

    let binbytes = res.bytes()?;
    ensure_checksum(&binbytes, version, checksum)?;

    // lock file to indicate that installation of this ylem version will be in progress.
    let lock_path = lock_file_path(version);
    // wait until lock file is released, possibly by another parallel thread trying to install the
    // same version of ylem.
    let _lock = try_lock_file(lock_path)?;

    do_install(
        version.clone(),
        binbytes.to_vec(),
        artifact.to_string().as_str(),
    )
}

/// Installs the provided version of Ylem in the machine.
///
/// Returns the path to the ylem file.
pub async fn install(version: &Version) -> Result<PathBuf, YlemVmError> {
    setup_data_dir()?;

    let artifacts = releases::all_releases(platform::platform()).await?;
    let artifact = artifacts
        .releases
        .get(version)
        .ok_or(YlemVmError::UnknownVersion)?;
    let download_url =
        releases::artifact_url(platform::platform(), version, artifact.to_string().as_str())?;

    let checksum = artifacts
        .get_checksum(version)
        .unwrap_or_else(|| panic!("checksum not available: {:?}", version.to_string()));

    let res = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .expect("reqwest::Client::new()")
        .get(download_url.clone())
        .send()
        .await?;

    if !res.status().is_success() {
        return Err(YlemVmError::UnsuccessfulResponse(
            download_url,
            res.status(),
        ));
    }

    let binbytes = res.bytes().await?;
    ensure_checksum(&binbytes, version, checksum)?;

    // lock file to indicate that installation of this ylem version will be in progress.
    let lock_path = lock_file_path(version);
    // wait until lock file is released, possibly by another parallel thread trying to install the
    // same version of ylem.
    let _lock = try_lock_file(lock_path)?;

    do_install(
        version.clone(),
        binbytes.to_vec(),
        artifact.to_string().as_str(),
    )
}

fn do_install(
    version: Version,
    binbytes: Vec<u8>,
    _artifact: &str,
) -> Result<PathBuf, YlemVmError> {
    let installer = {
        setup_version(version.to_string().as_str())?;

        Installer { version, binbytes }
    };

    // Ylem versions <= 0.7.1 are .zip files for Windows only
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    if _artifact.ends_with(".zip") {
        return installer.install_zip();
    }

    installer.install()
}

/// Removes the provided version of Ylem from the machine.
pub fn remove_version(version: &Version) -> Result<(), YlemVmError> {
    fs::remove_dir_all(version_path(version.to_string().as_str()))?;
    Ok(())
}

/// Setup YVM home directory.
pub fn setup_data_dir() -> Result<PathBuf, YlemVmError> {
    // create $XDG_DATA_HOME or ~/.local/share/yvm, or fallback to ~/.yvm
    let yvm_dir = YVM_DATA_DIR.to_path_buf();
    if !yvm_dir.as_path().exists() {
        fs::create_dir_all(yvm_dir.clone())?;
    }
    // create $YVM/.global-version
    let mut global_version = YVM_DATA_DIR.to_path_buf();
    global_version.push(".global-version");
    if !global_version.as_path().exists() {
        fs::File::create(global_version.as_path())?;
    }
    Ok(yvm_dir)
}

fn setup_version(version: &str) -> Result<(), YlemVmError> {
    let v = version_path(version);
    if !v.exists() {
        fs::create_dir_all(v.as_path())?
    }
    Ok(())
}

fn ensure_checksum(
    binbytes: impl AsRef<[u8]>,
    version: &Version,
    expected_checksum: Vec<u8>,
) -> Result<(), YlemVmError> {
    let mut hasher = sha2::Sha256::new();
    hasher.update(binbytes);
    let cs = &hasher.finalize()[..];
    // checksum does not match
    if cs != expected_checksum {
        return Err(YlemVmError::ChecksumMismatch {
            version: version.to_string(),
            expected: hex::encode(&expected_checksum),
            actual: hex::encode(cs),
        });
    }
    Ok(())
}

/// Creates the file and locks it exclusively, this will block if the file is currently locked
fn try_lock_file(lock_path: PathBuf) -> Result<LockFile, YlemVmError> {
    use fs2::FileExt;
    let _lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    _lock_file.lock_exclusive()?;
    Ok(LockFile {
        lock_path,
        _lock_file,
    })
}

/// Represents a lockfile that's removed once dropped
struct LockFile {
    _lock_file: fs::File,
    lock_path: PathBuf,
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Returns the lockfile to use for a specific file
fn lock_file_path(version: &Version) -> PathBuf {
    YVM_DATA_DIR.join(format!(".lock-ylem-{version}"))
}

#[cfg(test)]
mod tests {
    use crate::{
        platform::Platform,
        releases::{all_releases, artifact_url},
    };
    use rand::seq::SliceRandom;
    use reqwest::Url;

    use std::process::{Command, Stdio};

    use super::*;

    #[tokio::test]
    async fn test_data_dir_resolution() {
        let home_dir = dirs::home_dir().unwrap().join(".yvm");
        let data_dir = dirs::data_dir();
        let resolved_dir = resolve_data_dir();
        if home_dir.as_path().exists() || data_dir.is_none() {
            assert_eq!(resolved_dir.as_path(), home_dir.as_path());
        } else {
            assert_eq!(resolved_dir.as_path(), data_dir.unwrap().join("yvm"));
        }
    }

    #[tokio::test]
    async fn test_artifact_url() {
        let version = Version::new(1, 0, 1);
        let artifact = "ylem-linux-arm64";
        assert_eq!(
            artifact_url(Platform::LinuxAarch64, &version, artifact).unwrap(),
            Url::parse(&format!(
                "https://github.com/core-coin/ylem/releases/download/{version}/{artifact}"
            ))
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn test_install() {
        let versions = all_releases(platform())
            .await
            .unwrap()
            .releases
            .into_keys()
            .collect::<Vec<Version>>();
        let rand_version = versions.choose(&mut rand::thread_rng()).unwrap();
        install(rand_version).await.unwrap();
    }

    #[cfg(feature = "blocking")]
    #[test]
    fn blocking_test_install() {
        let versions = crate::releases::blocking_all_releases(platform::platform())
            .unwrap()
            .into_versions();
        let rand_version = versions.choose(&mut rand::thread_rng()).unwrap();
        blocking_install(rand_version).unwrap();
    }

    #[tokio::test]
    async fn test_version() {
        let version = "1.0.1".parse().unwrap();
        install(&version).await.unwrap();
        let ylem_path = version_path(version.to_string().as_str()).join(format!("ylem-{version}"));
        let output = Command::new(ylem_path)
            .arg("--version")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&output.stdout)
            .as_ref()
            .contains("1.0.1"));
    }

    #[cfg(feature = "blocking")]
    #[test]
    fn blocking_test_version() {
        let version = "1.0.1".parse().unwrap();
        blocking_install(&version).unwrap();
        let solc_path = version_path(version.to_string().as_str()).join(format!("ylem-{version}"));
        let output = Command::new(solc_path)
            .arg("--version")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .output()
            .unwrap();

        assert!(String::from_utf8_lossy(&output.stdout)
            .as_ref()
            .contains("1.0.1"));
    }

    #[cfg(feature = "blocking")]
    #[test]
    fn can_install_parallel() {
        let version: Version = "1.0.1".parse().unwrap();
        let cloned_version = version.clone();
        let t = std::thread::spawn(move || blocking_install(&cloned_version));
        blocking_install(&version).unwrap();
        t.join().unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn can_install_parallel_async() {
        let version: Version = "1.0.1".parse().unwrap();
        let cloned_version = version.clone();
        let t = tokio::task::spawn(async move { install(&cloned_version).await });
        install(&version).await.unwrap();
        t.await.unwrap().unwrap();
    }
}
