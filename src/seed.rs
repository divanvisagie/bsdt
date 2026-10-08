//! The NoCloud seed disk read by FreeBSD's nuageinit on first boot.
//!
//! nuageinit accepts a cd9660 or msdosfs volume labelled `cidata`. A small
//! FAT image is written here directly, so the host needs no ISO tooling.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;

use crate::config::GUEST_USER;

/// Where sshd looks for the key that may log in as root. `/etc/ssh` exists
/// before first boot, whereas `/root/.ssh` does not, and nuageinit's
/// `write_files` does not create directories.
const AUTHORIZED_KEYS: &str = "/etc/ssh/bsdt_authorized_keys";

pub fn write(path: &Path, hostname: &str, public_key: &str, update: bool) -> Result<()> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.set_len(1 << 20)?;
    fatfs::format_volume(&mut file, fatfs::FormatVolumeOptions::new().volume_label(*b"CIDATA     "))?;

    let fs = fatfs::FileSystem::new(file, fatfs::FsOptions::new())?;
    let root = fs.root_dir();
    write_file(&root, "meta-data", &meta_data(hostname))?;
    write_file(&root, "user-data", &user_data(public_key, update))?;
    drop(root);
    fs.unmount()?;
    Ok(())
}

fn write_file(dir: &fatfs::Dir<'_, File>, name: &str, contents: &str) -> Result<()> {
    let mut f = dir.create_file(name)?;
    f.truncate()?;
    f.write_all(contents.as_bytes())?;
    f.flush()?;
    Ok(())
}

fn meta_data(hostname: &str) -> String {
    format!("instance-id: bsdt-{hostname}\nlocal-hostname: {hostname}\n")
}

/// The `#cloud-config` user data. nuageinit parses it with libyaml, which
/// also reads JSON, so it is built as JSON to get quoting right for free.
fn user_data(public_key: &str, update: bool) -> String {
    let key = public_key.trim();
    // Appended before sshd first starts. FreeBSD's sshd_config already sets
    // AuthorizedKeysFile, and sshd keeps the first value it reads, so root's
    // key file is set in a Match block, which overrides it. Root logs in
    // with the key only, and the forwarded port is bound to 127.0.0.1.
    let sshd = format!(
        "\n# Added by bsdt\nPermitRootLogin prohibit-password\nMatch User root\n\tAuthorizedKeysFile {AUTHORIZED_KEYS}\n"
    );
    let mut files = vec![
        json!({ "path": AUTHORIZED_KEYS, "content": format!("{key}\n"), "permissions": "0644" }),
        json!({ "path": "/etc/ssh/sshd_config", "content": sshd, "append": true }),
    ];
    if !update {
        // The images upgrade the base system on first boot and then reboot:
        // firstboot_pkg_upgrade on 15, firstboot_freebsd_update on 14. rc
        // reads rc.conf only once, before nuageinit runs, but each script
        // also reads /etc/rc.conf.d/<script> as it starts.
        for script in ["firstboot_pkg_upgrade", "firstboot_freebsd_update"] {
            files.push(json!({
                "path": format!("/etc/rc.conf.d/{script}"),
                "content": format!("# Added by bsdt\n{script}_enable=\"NO\"\n"),
            }));
        }
    }
    let config = json!({
        "users": [{
            "name": GUEST_USER,
            "gecos": "bsdt user",
            "groups": ["wheel"],
            "shell": "/bin/sh",
            "ssh_authorized_keys": [key],
        }],
        "write_files": files,
    });
    format!("#cloud-config\n{}\n", serde_json::to_string_pretty(&config).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_data_is_cloud_config() {
        let data = user_data("ssh-ed25519 AAAA bsdt\n", false);
        let (header, body) = data.split_once('\n').unwrap();
        assert_eq!(header, "#cloud-config");
        let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(parsed["users"][0]["name"], GUEST_USER);
        assert_eq!(parsed["users"][0]["ssh_authorized_keys"][0], "ssh-ed25519 AAAA bsdt");
        assert_eq!(parsed["write_files"][0]["content"], "ssh-ed25519 AAAA bsdt\n");
        assert_eq!(parsed["write_files"][2]["path"], "/etc/rc.conf.d/firstboot_pkg_upgrade");
        assert!(parsed["write_files"][2]["content"].as_str().unwrap().contains("firstboot_pkg_upgrade_enable=\"NO\""));
        let updating: serde_json::Value = serde_json::from_str(user_data("k", true).split_once('\n').unwrap().1).unwrap();
        assert_eq!(updating["write_files"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn seed_image_has_label_and_files() {
        let dir = std::env::temp_dir().join(format!("bsdt-seed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("seed.img");
        write(&path, "box", "ssh-ed25519 AAAA bsdt", false).unwrap();

        let fs = fatfs::FileSystem::new(File::open(&path).unwrap(), fatfs::FsOptions::new()).unwrap();
        assert_eq!(fs.volume_label(), "CIDATA");
        let mut names: Vec<String> = fs.root_dir().iter().map(|e| e.unwrap().file_name()).collect();
        names.sort();
        assert_eq!(names, ["meta-data", "user-data"]);
        drop(fs);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
