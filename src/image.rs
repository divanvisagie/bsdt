//! Downloading, verifying and caching base images.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::os::Image;

/// `$BSDT_CACHE_DIR`, or the platform cache directory plus `bsdt`.
pub fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("BSDT_CACHE_DIR") {
        return Ok(dir.into());
    }
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("bsdt"));
    }
    let home = std::env::var_os("HOME").context("$HOME is not set")?;
    let base = if cfg!(target_os = "macos") { "Library/Caches" } else { ".cache" };
    Ok(PathBuf::from(home).join(base).join("bsdt"))
}

pub fn images_dir() -> Result<PathBuf> {
    Ok(cache_dir()?.join("images"))
}

/// The cached, decompressed image, downloading it first if needed.
pub fn ensure(image: &Image) -> Result<PathBuf> {
    let path = images_dir()?.join(&image.cache_path);
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;

    let expected = expected_sha256(image)?;
    let xz = path.with_extension("qcow2.xz.part");
    eprintln!("bsdt: downloading {}", image.url);
    // -C - resumes an interrupted download of the same file.
    let status = Command::new("curl")
        .args(["-fL", "--progress-bar", "-C", "-", "-o"])
        .arg(&xz)
        .arg(&image.url)
        .status()
        .context("running curl (is it installed?)")?;
    if !status.success() {
        bail!("downloading {} failed", image.url);
    }

    eprintln!("bsdt: verifying checksum");
    let actual = sha256_file(&xz)?;
    if actual != expected {
        std::fs::remove_file(&xz).ok();
        bail!("checksum mismatch for {}: expected {expected}, got {actual}", image.file_name);
    }

    eprintln!("bsdt: decompressing to {}", path.display());
    let part = path.with_extension("qcow2.part");
    let mut reader = liblzma::read::XzDecoder::new(BufReader::new(File::open(&xz)?));
    let mut writer = BufWriter::new(File::create(&part)?);
    io::copy(&mut reader, &mut writer).with_context(|| format!("decompressing {}", xz.display()))?;
    writer.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    std::fs::rename(&part, &path)?;
    std::fs::remove_file(&xz).ok();
    Ok(path)
}

fn expected_sha256(image: &Image) -> Result<String> {
    let output = Command::new("curl")
        .args(["-fsSL", &image.checksum_url])
        .output()
        .context("running curl (is it installed?)")?;
    if !output.status.success() {
        bail!(
            "fetching {} failed; check that the version exists: {}",
            image.checksum_url,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    find_checksum(&listing, &image.file_name)
        .with_context(|| format!("{} is not listed in {}", image.file_name, image.checksum_url))
}

/// Find `name` in a BSD-style `SHA256 (name) = hex` listing.
fn find_checksum(listing: &str, name: &str) -> Option<String> {
    let prefix = format!("SHA256 ({name}) = ");
    listing.lines().find_map(|line| line.strip_prefix(&prefix)).map(|hex| hex.trim().to_ascii_lowercase())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Every cached image, relative to the images directory, with its size.
pub fn list() -> Result<Vec<(PathBuf, u64)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64)>) -> io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(root, &path, out)?;
            } else if path.extension().is_some_and(|e| e == "qcow2") {
                out.push((path.strip_prefix(root).unwrap().to_path_buf(), entry.metadata()?.len()));
            }
        }
        Ok(())
    }

    let root = images_dir()?;
    let mut out = Vec::new();
    if root.is_dir() {
        walk(&root, &root, &mut out)?;
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checksum_line() {
        let listing = "SHA256 (a.qcow2.xz) = AAAA\nSHA256 (b.qcow2.xz) = bbbb\n";
        assert_eq!(find_checksum(listing, "b.qcow2.xz").as_deref(), Some("bbbb"));
        assert_eq!(find_checksum(listing, "a.qcow2.xz").as_deref(), Some("aaaa"));
        assert_eq!(find_checksum(listing, "c.qcow2.xz"), None);
    }
}
