//! Per-OS knowledge: where the base image comes from and how to install
//! packages. Only FreeBSD is implemented; NetBSD and OpenBSD are planned.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::config::{Arch, Os, Vm};

/// A compressed base image published by the OS project.
pub struct Image {
    pub url: String,
    /// File listing `SHA256 (<name>) = <hex>` lines for the image.
    pub checksum_url: String,
    /// The compressed file's name, as it appears in the checksum file.
    pub file_name: String,
    /// Where the decompressed image is cached, relative to the image cache.
    pub cache_path: PathBuf,
}

/// Fail early for OSes that are accepted in the file but not implemented.
pub fn check_supported(os: Os) -> Result<()> {
    match os {
        Os::FreeBSD => Ok(()),
        Os::NetBSD | Os::OpenBSD => bail!("{os} guests are planned but not supported yet; only freebsd works for now"),
    }
}

pub fn image(vm: &Vm) -> Result<Image> {
    check_supported(vm.os)?;
    let arch = vm.arch.resolve();
    let version = &vm.version;
    // FreeBSD names the directory after MACHINE_ARCH, but the file after
    // MACHINE-MACHINE_ARCH, except on amd64 where they are the same.
    let (dir, file_arch) = match arch {
        Arch::Aarch64 => ("aarch64", "arm64-aarch64"),
        _ => ("amd64", "amd64"),
    };
    let base = format!("https://download.freebsd.org/releases/VM-IMAGES/{version}-RELEASE/{dir}/Latest");
    let file_name = format!("FreeBSD-{version}-RELEASE-{file_arch}-BASIC-CLOUDINIT-{}.qcow2.xz", vm.filesystem);
    Ok(Image {
        url: format!("{base}/{file_name}"),
        checksum_url: format!("{base}/CHECKSUM.SHA256"),
        cache_path: PathBuf::from(format!("freebsd/{version}/{arch}-{}.qcow2", vm.filesystem)),
        file_name,
    })
}

/// Shell command, run as root, that installs `packages` plus whatever bsdt
/// itself needs in the guest (rsync).
pub fn install_packages(os: Os, packages: &[String]) -> String {
    let mut cmd = match os {
        // ASSUME_ALWAYS_YES also answers the prompt to bootstrap pkg itself.
        Os::FreeBSD => String::from("env ASSUME_ALWAYS_YES=yes pkg install -y rsync"),
        Os::NetBSD | Os::OpenBSD => unreachable!("checked by check_supported"),
    };
    for package in packages {
        cmd.push(' ');
        cmd.push_str(&crate::ssh::quote(package));
    }
    cmd
}

/// Root setup for `audio = true`, safe to run on every boot: load the
/// driver for QEMU's Intel HDA card now and on later boots, so it shows up
/// as /dev/dsp.
pub const AUDIO_SETUP: &str = "kldload -n snd_hda\nsysrc -q kld_list+=snd_hda >/dev/null\n";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn vm(extra: &str) -> Vm {
        let config: Config = toml::from_str(&format!("[vm]\nos = \"freebsd\"\nversion = \"15.1\"\n{extra}")).unwrap();
        config.vm
    }

    #[test]
    fn freebsd_image_urls() {
        let img = image(&vm("arch = \"amd64\"")).unwrap();
        assert_eq!(
            img.url,
            "https://download.freebsd.org/releases/VM-IMAGES/15.1-RELEASE/amd64/Latest/FreeBSD-15.1-RELEASE-amd64-BASIC-CLOUDINIT-ufs.qcow2.xz"
        );
        assert_eq!(img.cache_path, PathBuf::from("freebsd/15.1/amd64-ufs.qcow2"));

        let img = image(&vm("arch = \"aarch64\"\nfilesystem = \"zfs\"")).unwrap();
        assert_eq!(
            img.url,
            "https://download.freebsd.org/releases/VM-IMAGES/15.1-RELEASE/aarch64/Latest/FreeBSD-15.1-RELEASE-arm64-aarch64-BASIC-CLOUDINIT-zfs.qcow2.xz"
        );
    }

    #[test]
    fn package_names_are_quoted() {
        assert_eq!(
            install_packages(Os::FreeBSD, &["git".into(), "rust".into()]),
            "env ASSUME_ALWAYS_YES=yes pkg install -y rsync 'git' 'rust'"
        );
    }
}
