//! Talking to the guest with the host's ssh and rsync.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::GUEST_USER;

/// Quote `s` for a POSIX shell.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Create the per-VM key pair if it does not exist yet and return the
/// public key.
pub fn ensure_key(key: &Path) -> Result<String> {
    if !key.exists() {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "bsdt", "-f"])
            .arg(key)
            .status()
            .context("running ssh-keygen (is OpenSSH installed?)")?;
        if !status.success() {
            bail!("ssh-keygen failed");
        }
    }
    let public = key.with_extension("pub");
    std::fs::read_to_string(&public).with_context(|| format!("reading {}", public.display()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum User {
    Guest,
    Root,
}

impl User {
    fn name(self) -> &'static str {
        match self {
            User::Guest => GUEST_USER,
            User::Root => "root",
        }
    }
}

pub struct Ssh {
    pub key: PathBuf,
    pub port: u16,
}

impl Ssh {
    /// Options shared by ssh and rsync's remote shell. The guest's host key
    /// changes with every new disk, so it is neither checked nor recorded.
    fn options(&self) -> Vec<String> {
        let mut opts = vec!["-i".into(), self.key.to_string_lossy().into_owned(), "-p".into(), self.port.to_string()];
        for opt in [
            "StrictHostKeyChecking=no",
            "UserKnownHostsFile=/dev/null",
            "LogLevel=ERROR",
            "IdentitiesOnly=yes",
            "ConnectTimeout=5",
        ] {
            opts.push("-o".into());
            opts.push(opt.into());
        }
        opts
    }

    fn command(&self, user: User) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.args(self.options()).arg(format!("{}@127.0.0.1", user.name()));
        cmd
    }

    /// Run a shell command in the guest without a terminal, quietly, for
    /// probing. Exit status 255 means ssh itself failed to connect.
    pub fn probe(&self, user: User, script: &str) -> Result<ExitStatus> {
        Ok(self
            .command(user)
            .args(["-o", "BatchMode=yes"])
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?)
    }

    /// Run a shell command in the guest, sharing this process's stdio. A
    /// terminal is allocated when stdin and stdout are terminals.
    pub fn run(&self, user: User, script: &str) -> Result<ExitStatus> {
        let mut cmd = self.command(user);
        if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
            cmd.arg("-t");
        }
        Ok(cmd.arg(script).status()?)
    }

    /// Like [`Ssh::run`], but an unsuccessful exit is an error.
    pub fn run_checked(&self, user: User, script: &str) -> Result<()> {
        let status = self.run(user, script)?;
        if !status.success() {
            bail!("`{script}` failed in the guest ({status})");
        }
        Ok(())
    }

    /// Open an interactive login shell in `dir`.
    pub fn shell(&self, user: User, dir: &str) -> Result<ExitStatus> {
        let mut cmd = self.command(user);
        cmd.arg("-t").arg(format!("cd {} 2>/dev/null; exec \"$SHELL\" -l", quote(dir)));
        Ok(cmd.status()?)
    }

    /// Mirror `src` into `dest` in the guest, as the guest user. Files in
    /// the guest that are excluded here, such as build output, are kept.
    pub fn rsync(&self, src: &Path, dest: &str, exclude: &[String]) -> Result<()> {
        let shell = std::iter::once("ssh".to_string())
            .chain(self.options().iter().map(|o| quote(o)))
            .collect::<Vec<_>>()
            .join(" ");
        let mut cmd = Command::new("rsync");
        cmd.args(["-a", "--delete", "--exclude", "/.bsdt/"]);
        for pattern in exclude {
            cmd.arg("--exclude").arg(pattern);
        }
        let status = cmd
            .arg("-e")
            .arg(shell)
            .arg(format!("--rsync-path=mkdir -p {} && rsync", quote(dest)))
            .arg(format!("{}/", src.display()))
            .arg(format!("{GUEST_USER}@127.0.0.1:{dest}/"))
            .status()
            .context("running rsync (is it installed?)")?;
        if !status.success() {
            bail!("rsync to the guest failed ({status})");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_for_sh() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("$HOME; rm"), "'$HOME; rm'");
    }
}
