//! The `bsdt.toml` environment file.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const DEFAULT_FILE: &str = "bsdt.toml";

/// The unprivileged user bsdt creates in the guest and syncs files as.
pub const GUEST_USER: &str = "bsdt";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub vm: Vm,
    #[serde(default)]
    pub packages: Packages,
    #[serde(default)]
    pub sync: Sync,
    #[serde(default)]
    pub provision: Provision,
    #[serde(default)]
    pub gui: Gui,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vm {
    pub os: Os,
    pub version: String,
    #[serde(default)]
    pub arch: Arch,
    #[serde(default = "default_cpus")]
    pub cpus: u32,
    #[serde(default = "default_memory")]
    pub memory: String,
    #[serde(default = "default_disk")]
    pub disk: String,
    #[serde(default)]
    pub filesystem: Filesystem,
    pub hostname: Option<String>,
    #[serde(default)]
    pub ports: Vec<Port>,
    /// Let the image install OS security updates on first boot, which
    /// takes a while and reboots. Off by default so every VM starts from
    /// exactly the release image.
    #[serde(default)]
    pub update: bool,
    /// Run a desktop in the guest and open a window on it; see [`Gui`].
    #[serde(default)]
    pub gui: bool,
    /// Attach a USB keyboard and tablet, for `bsdt key` and anything that
    /// reads /dev/input. Defaults to on when `gui` is.
    pub input: Option<bool>,
}

impl Vm {
    pub fn input(&self) -> bool {
        self.input.unwrap_or(self.gui)
    }
}

fn default_cpus() -> u32 {
    2
}

fn default_memory() -> String {
    "2G".into()
}

fn default_disk() -> String {
    "20G".into()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    #[serde(default)]
    pub install: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sync {
    /// Guest directory the project is synced to; defaults to
    /// `/home/bsdt/<project directory name>`.
    pub dest: Option<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provision {
    /// Commands run as root, after packages are installed.
    #[serde(default)]
    pub root: Vec<String>,
    /// Commands run as the guest user in the synced directory.
    #[serde(default)]
    pub run: Vec<String>,
}

/// Settings for `vm.gui`. Every field has a default, so `gui = true` on
/// its own gives a sway desktop in a window.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gui {
    #[serde(default)]
    pub desktop: Desktop,
    /// Command run as the guest user once the desktop is up, on every boot.
    /// Defaults to a terminal on the sway desktop; `""` runs nothing.
    pub start: Option<String>,
    /// Port the guest's VNC server listens on.
    #[serde(default = "default_vnc")]
    pub vnc: u16,
    /// Host port forwarded to `vnc`; a free one is picked when unset.
    pub port: Option<u16>,
    /// Open a viewer window when `bsdt up` finishes.
    #[serde(default = "default_true")]
    pub open: bool,
    #[serde(default = "default_resolution")]
    pub resolution: String,
}

impl Default for Gui {
    fn default() -> Self {
        Gui {
            desktop: Desktop::default(),
            start: None,
            vnc: default_vnc(),
            port: None,
            open: true,
            resolution: default_resolution(),
        }
    }
}

impl Gui {
    pub fn start(&self) -> Option<&str> {
        match (&self.start, self.desktop) {
            (Some(cmd), _) => Some(cmd.as_str()).filter(|c| !c.trim().is_empty()),
            (None, Desktop::Sway) => Some("foot"),
            (None, Desktop::None) => None,
        }
    }
}

fn default_vnc() -> u16 {
    5900
}

fn default_true() -> bool {
    true
}

fn default_resolution() -> String {
    "1280x800".into()
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Desktop {
    /// sway on a headless output, shared over VNC by wayvnc, all installed
    /// and started by bsdt.
    #[default]
    Sway,
    /// Nothing; `start` brings up whatever serves VNC on `vnc`.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    FreeBSD,
    NetBSD,
    OpenBSD,
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Os::FreeBSD => "freebsd",
            Os::NetBSD => "netbsd",
            Os::OpenBSD => "openbsd",
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// Whatever the host is, so the guest gets hardware acceleration.
    #[default]
    Auto,
    #[serde(alias = "x86_64")]
    Amd64,
    #[serde(alias = "arm64")]
    Aarch64,
}

impl Arch {
    pub fn host() -> Arch {
        match std::env::consts::ARCH {
            "aarch64" => Arch::Aarch64,
            _ => Arch::Amd64,
        }
    }

    /// `Auto` replaced by the host architecture.
    pub fn resolve(self) -> Arch {
        match self {
            Arch::Auto => Arch::host(),
            arch => arch,
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Arch::Auto => "auto",
            Arch::Amd64 => "amd64",
            Arch::Aarch64 => "aarch64",
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Filesystem {
    #[default]
    Ufs,
    Zfs,
}

impl fmt::Display for Filesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Filesystem::Ufs => "ufs",
            Filesystem::Zfs => "zfs",
        })
    }
}

/// A TCP port forwarded from `127.0.0.1:host` to the guest, written as
/// `"HOST:GUEST"`, or just `"PORT"` when both are the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Port {
    pub host: u16,
    pub guest: u16,
}

impl TryFrom<String> for Port {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let parse = |p: &str| p.trim().parse::<u16>().ok().filter(|&n| n != 0);
        let port = match s.split_once(':') {
            Some((host, guest)) => parse(host).zip(parse(guest)),
            None => parse(&s).map(|p| (p, p)),
        };
        match port {
            Some((host, guest)) => Ok(Port { host, guest }),
            None => Err(format!("invalid port {s:?}, expected \"HOST:GUEST\" or \"PORT\"")),
        }
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "127.0.0.1:{} -> {}", self.host, self.guest)
    }
}

/// A loaded environment file and where it lives.
pub struct Project {
    /// Directory holding the environment file; this is what gets synced.
    pub dir: PathBuf,
    pub file: PathBuf,
    pub config: Config,
}

impl Project {
    /// Load `file`, or find `bsdt.toml` in the current directory or one of
    /// its parents.
    pub fn load(file: Option<&Path>) -> Result<Project> {
        let file = match file {
            Some(file) => file.to_path_buf(),
            None => find(&std::env::current_dir()?)?,
        };
        let file = std::path::absolute(&file)?;
        let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        let config: Config = toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        let dir = file.parent().context("environment file has no parent directory")?.to_path_buf();
        Ok(Project { dir, file, config })
    }

    /// The project directory's name, cleaned up for use as a hostname.
    pub fn name(&self) -> String {
        let raw = self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        sanitize_hostname(&raw)
    }

    pub fn hostname(&self) -> String {
        match &self.config.vm.hostname {
            Some(h) => h.clone(),
            None => self.name(),
        }
    }

    /// Where VM state lives: `.bsdt/<file stem>/` beside the environment
    /// file, so `bsdt.toml` and `-f other.toml` get separate VMs.
    pub fn state_dir(&self) -> PathBuf {
        let stem = self.file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "bsdt".into());
        self.dir.join(".bsdt").join(stem)
    }

    pub fn dest(&self) -> String {
        match &self.config.sync.dest {
            Some(dest) => dest.trim_end_matches('/').to_string(),
            None => format!("/home/{GUEST_USER}/{}", self.name()),
        }
    }

    /// The guest directory matching the host's current directory, falling
    /// back to the sync destination when outside the project.
    pub fn guest_cwd(&self) -> String {
        let dest = self.dest();
        let rel = std::env::current_dir().ok().and_then(|cwd| cwd.strip_prefix(&self.dir).ok().map(Path::to_path_buf));
        match rel {
            Some(rel) if !rel.as_os_str().is_empty() => format!("{dest}/{}", rel.to_string_lossy()),
            _ => dest,
        }
    }
}

fn find(start: &Path) -> Result<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join(DEFAULT_FILE);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("no {DEFAULT_FILE} in {} or its parents (run `bsdt init` to create one)", start.display())
}

fn sanitize_hostname(raw: &str) -> String {
    let mut name: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    name.truncate(63);
    let name = name.trim_matches('-');
    if name.is_empty() { "bsdt".into() } else { name.into() }
}

/// The file `bsdt init` writes.
pub const TEMPLATE: &str = r#"# bsdt environment; see bsdt(1).

[vm]
os = "freebsd"
version = "15.1"
# arch = "auto"         # auto (match the host), amd64 or aarch64
# cpus = 2
# memory = "2G"
# disk = "20G"
# filesystem = "ufs"    # ufs or zfs
# ports = ["8080:80"]   # 127.0.0.1:HOST on the host -> GUEST in the VM
# update = false        # install OS security updates on first boot
# gui = false           # a sway desktop in a window; see [gui] in bsdt(1)

[packages]
install = ["git"]

[sync]
# dest = "/home/bsdt/<project directory>"
exclude = ["target"]

[provision]
# root = ["sysrc foo_enable=YES"]
# run = ["cargo fetch"]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let config: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(config.vm.os, Os::FreeBSD);
        assert_eq!(config.vm.arch, Arch::Auto);
        assert_eq!(config.vm.cpus, 2);
        assert_eq!(config.packages.install, ["git"]);
    }

    #[test]
    fn full_config_parses() {
        let config: Config = toml::from_str(
            r#"
            [vm]
            os = "freebsd"
            version = "14.5"
            arch = "arm64"
            filesystem = "zfs"
            ports = ["8080:80", "5432"]
            [provision]
            root = ["sysrc sshd_enable=YES"]
            run = ["make"]
            "#,
        )
        .unwrap();
        assert_eq!(config.vm.arch, Arch::Aarch64);
        assert_eq!(config.vm.filesystem, Filesystem::Zfs);
        assert_eq!(config.vm.ports, [Port { host: 8080, guest: 80 }, Port { host: 5432, guest: 5432 }]);
        assert_eq!(config.provision.run, ["make"]);
    }

    #[test]
    fn gui_defaults() {
        let config: Config = toml::from_str("[vm]\nos = \"freebsd\"\nversion = \"15.1\"\ngui = true").unwrap();
        assert!(config.vm.input());
        assert_eq!(config.gui.desktop, Desktop::Sway);
        assert_eq!(config.gui.vnc, 5900);
        assert_eq!(config.gui.port, None);
        assert!(config.gui.open);
        assert_eq!(config.gui.start(), Some("foot"));

        let config: Config = toml::from_str(
            "[vm]\nos = \"freebsd\"\nversion = \"15.1\"\ngui = true\ninput = false\n[gui]\ndesktop = \"none\"\nstart = \"x11vnc\"\nport = 5901",
        )
        .unwrap();
        assert!(!config.vm.input());
        assert_eq!(config.gui.start(), Some("x11vnc"));
        assert_eq!(config.gui.port, Some(5901));

        let config: Config = toml::from_str("[vm]\nos = \"freebsd\"\nversion = \"15.1\"\ninput = true\n[gui]\nstart = \"\"").unwrap();
        assert!(config.vm.input() && !config.vm.gui);
        assert_eq!(config.gui.start(), None);
    }

    #[test]
    fn rejects_unknown_keys_and_bad_ports() {
        assert!(toml::from_str::<Config>("[vm]\nos = \"freebsd\"\nversion = \"15.1\"\nram = \"2G\"").is_err());
        for bad in ["0", "x:80", "80:", "70000:80"] {
            assert!(Port::try_from(bad.to_string()).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn hostnames_are_sanitized() {
        assert_eq!(sanitize_hostname("My_Project.rs"), "my-project-rs");
        assert_eq!(sanitize_hostname("..."), "bsdt");
    }
}
