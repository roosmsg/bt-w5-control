//! Firmware and driver update checks against Creative's public file server (display only),
//! and downloads (never run) of Creative's firmware and audio driver installers.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use md5::Digest;
use serde::Deserialize;

const PRODUCT: &str = "AYGO4";
const DRIVER_TYPE: &str = "USB";
const BASE: &str = "https://files.creative.com/creative/bin";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    UpToDate(String),
    Available { installed: String, latest: String },
    Unknown(String),
}

/// Compares dotted versions numerically, so `2.2.6.1` equals `2.02.06.01`.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (mut a, mut b) = (parse(a), parse(b));
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    a.cmp(&b)
}

fn status(installed: &str, latest: String) -> Status {
    if compare_versions(&latest, installed) == Ordering::Greater {
        Status::Available {
            installed: installed.to_string(),
            latest,
        }
    } else {
        Status::UpToDate(latest)
    }
}

fn get_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirmwareRelease {
    latest_version: String,
}

pub fn latest_firmware() -> Result<String, String> {
    let url = format!("{BASE}/firmware/fwureleases/{PRODUCT}/{PRODUCT}_release.json");
    let release: FirmwareRelease =
        serde_json::from_str(&get_text(&url)?).map_err(|e| e.to_string())?;
    Ok(release.latest_version)
}

pub fn check_firmware(installed: &str) -> Status {
    match latest_firmware() {
        Ok(latest) => status(installed, latest),
        Err(e) => Status::Unknown(e),
    }
}

#[derive(Deserialize)]
struct DriverInfo {
    releases: Vec<DriverRelease>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriverRelease {
    products: Vec<String>,
    version: String,
    file_name: String,
    /// MD5 of the installer, as published by Creative.
    hash: String,
}

/// The newest audio driver release Creative lists for this device.
fn latest_driver_release() -> Result<DriverRelease, String> {
    let url = format!("{BASE}/driver/drvureleases/{DRIVER_TYPE}/driverinfo_release.json");
    let info: DriverInfo = serde_json::from_str(&get_text(&url)?).map_err(|e| e.to_string())?;
    info.releases
        .into_iter()
        .filter(|r| r.products.iter().any(|p| p == PRODUCT))
        .max_by(|a, b| compare_versions(&a.version, &b.version))
        .ok_or_else(|| "no driver release listed for this device".to_string())
}

pub fn check_driver(installed: &str) -> Status {
    match latest_driver_release() {
        Ok(latest) => status(installed, latest.version),
        Err(e) => Status::Unknown(e),
    }
}

/// Downloads Creative's audio driver installer for the newest release into `dir`, verifies the
/// MD5 from Creative's driver list and the Authenticode signature. Never runs it.
pub fn download_driver_installer(dir: &Path) -> Result<PathBuf, String> {
    let release = latest_driver_release()?;
    check_file_name(&release.file_name)?;
    if !release
        .version
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(format!("unexpected driver version {}", release.version));
    }
    let url = format!(
        "{BASE}/driver/drvureleases/{DRIVER_TYPE}/{}/{}",
        release.version, release.file_name
    );
    let bytes = fetch(&url)?;
    let md5 = md5_hex(&bytes);
    if !md5.eq_ignore_ascii_case(release.hash.trim()) {
        return Err(format!("MD5 {md5} differs from {}", release.hash.trim()));
    }
    save_verified(dir, &release.file_name, &bytes)
}

fn check_file_name(name: &str) -> Result<(), String> {
    if name.contains(['/', '\\', ':']) || !name.ends_with(".exe") {
        return Err(format!("unexpected installer name {name}"));
    }
    Ok(())
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// Writes `bytes` to `dir/name` and keeps the file only if its Authenticode signature is valid.
fn save_verified(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let path = save(dir, name, bytes)?;
    if let Err(e) = crate::winutil::verify_signature(&path) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

fn save(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn md5_hex(bytes: &[u8]) -> String {
    md5::Md5::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Deserialize)]
struct FirmwareMetadataFile {
    metadata: Vec<FirmwareModule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirmwareModule {
    module_id: String,
    file_name: String,
    file_size: String,
    checksum: String,
}

/// Downloads Creative's installer for the current firmware release into `dir` and verifies it
/// against the release metadata (size and CRC32) and its Authenticode signature. This function
/// never runs it. Returns the saved path.
pub fn download_firmware_installer(dir: &Path) -> Result<PathBuf, String> {
    let version = latest_firmware()?;
    let url = format!("{BASE}/firmware/fwureleases/{PRODUCT}/{version}/{PRODUCT}_metadata.json");
    let meta: FirmwareMetadataFile =
        serde_json::from_str(&get_text(&url)?).map_err(|e| e.to_string())?;
    let module = meta
        .metadata
        .into_iter()
        .next()
        .ok_or("firmware metadata is empty")?;
    check_file_name(&module.file_name)?;

    let url = format!(
        "{BASE}/firmware/fwureleases/{PRODUCT}/zifs/{}/{}",
        module.module_id, module.file_name
    );
    let bytes = fetch(&url)?;

    let expected_size: usize = module
        .file_size
        .trim()
        .parse()
        .map_err(|_| "bad file size")?;
    if bytes.len() != expected_size {
        return Err(format!(
            "download size {} differs from {expected_size}",
            bytes.len()
        ));
    }
    let expected_crc =
        u32::from_str_radix(module.checksum.trim(), 16).map_err(|_| "bad checksum")?;
    let crc = crc32fast::hash(&bytes);
    if crc != expected_crc {
        return Err(format!(
            "checksum {crc:08X} differs from {expected_crc:08X}"
        ));
    }

    save_verified(dir, &module.file_name, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Live check against Creative's server: `cargo test -- --ignored`.
    #[test]
    #[ignore = "downloads from files.creative.com"]
    fn downloads_both_installers() {
        let dir = std::env::temp_dir().join("bt-w5-control-download-test");
        let _ = std::fs::remove_dir_all(&dir);
        for (name, f) in [
            (
                "firmware",
                download_firmware_installer as fn(&Path) -> Result<PathBuf, String>,
            ),
            ("driver", download_driver_installer),
        ] {
            let path = f(&dir).unwrap_or_else(|e| panic!("{name}: {e}"));
            let size = std::fs::metadata(&path).unwrap().len();
            println!("{name}: {} ({size} bytes)", path.display());
            assert!(size > 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("2.2.6.1", "2.02.06.01"), Ordering::Equal);
        assert_eq!(
            compare_versions("2.02.06.01", "2.01.03.00"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.2.230511.1220", "1.0.230224.1400"),
            Ordering::Greater
        );
        assert_eq!(compare_versions("1.2", "1.2.0.0"), Ordering::Equal);
    }

    #[test]
    fn status_reports_newer_only() {
        assert_eq!(
            status("2.2.6.1", "2.02.06.01".into()),
            Status::UpToDate("2.02.06.01".into())
        );
        assert!(matches!(
            status("1.0.230224.1400", "1.2.230511.1220".into()),
            Status::Available { .. }
        ));
    }
}
