//! Starting, stopping and inspecting the QEMU process.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::{Arch, Port};

pub struct Launch<'a> {
    pub arch: Arch,
    pub cpus: u32,
    pub memory: &'a str,
    pub disk: &'a Path,
    pub seed: &'a Path,
    pub ssh_port: u16,
    pub qmp_port: u16,
    pub ports: &'a [Port],
    pub console: &'a Path,
    pub pidfile: &'a Path,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accel {
    Kvm,
    Hvf,
    Tcg,
}

/// The fastest accelerator available for `arch` on this host.
pub fn accel(arch: Arch) -> Accel {
    if arch != Arch::host() {
        return Accel::Tcg;
    }
    if cfg!(target_os = "macos") {
        return Accel::Hvf;
    }
    let kvm = std::fs::OpenOptions::new().read(true).write(true).open("/dev/kvm");
    if cfg!(target_os = "linux") && kvm.is_ok() { Accel::Kvm } else { Accel::Tcg }
}

pub fn create_overlay(base: &Path, overlay: &Path, size: &str) -> Result<()> {
    let output = Command::new("qemu-img")
        .args(["create", "-q", "-f", "qcow2", "-F", "qcow2", "-b"])
        .arg(base)
        .arg(overlay)
        .arg(size)
        .output()
        .context("running qemu-img (is QEMU installed?)")?;
    if !output.status.success() {
        bail!("qemu-img create failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

pub fn start(l: &Launch) -> Result<Accel> {
    let accel = accel(l.arch);
    let (binary, machine) = match l.arch {
        Arch::Aarch64 => ("qemu-system-aarch64", "virt"),
        _ => ("qemu-system-x86_64", "q35"),
    };
    let cpu = match accel {
        Accel::Kvm | Accel::Hvf => "host",
        Accel::Tcg => "max",
    };
    let accel_name = match accel {
        Accel::Kvm => "kvm",
        Accel::Hvf => "hvf",
        Accel::Tcg => "tcg",
    };

    let mut netdev = format!("user,id=net0,hostfwd=tcp:127.0.0.1:{}-:22", l.ssh_port);
    for port in l.ports {
        netdev.push_str(&format!(",hostfwd=tcp:127.0.0.1:{}-:{}", port.host, port.guest));
    }

    let mut cmd = Command::new(binary);
    cmd.args(["-machine", machine, "-accel", accel_name, "-cpu", cpu])
        .args(["-smp", &l.cpus.to_string(), "-m", l.memory])
        .args(["-display", "none", "-vga", "none"])
        .arg("-drive")
        .arg(drive(l.disk, "qcow2"))
        .arg("-drive")
        .arg(drive(l.seed, "raw"))
        .args(["-netdev", &netdev, "-device", "virtio-net-pci,netdev=net0"])
        .args(["-device", "virtio-rng-pci"])
        .arg("-serial")
        .arg(format!("file:{}", l.console.display()))
        .args(["-qmp", &format!("tcp:127.0.0.1:{},server=on,wait=off", l.qmp_port)])
        .arg("-pidfile")
        .arg(l.pidfile)
        .arg("-daemonize");
    if l.arch == Arch::Aarch64 {
        cmd.arg("-bios").arg(aarch64_firmware(binary)?);
    }

    let output = cmd.output().with_context(|| format!("running {binary} (is QEMU installed?)"))?;
    if !output.status.success() {
        bail!("{binary} failed to start: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(accel)
}

fn drive(path: &Path, format: &str) -> String {
    // QEMU option values escape commas by doubling them.
    let file = path.to_string_lossy().replace(',', ",,");
    format!("file={file},format={format},if=virtio")
}

/// UEFI firmware for the aarch64 `virt` machine, from QEMU's own share
/// directory or the usual distribution packages.
fn aarch64_firmware(binary: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(prefix) = which(binary).and_then(|p| p.parent()?.parent().map(Path::to_path_buf)) {
        candidates.push(prefix.join("share/qemu/edk2-aarch64-code.fd"));
    }
    for path in [
        "/opt/homebrew/share/qemu/edk2-aarch64-code.fd",
        "/usr/local/share/qemu/edk2-aarch64-code.fd",
        "/usr/share/qemu/edk2-aarch64-code.fd",
        "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
        "/usr/share/AAVMF/AAVMF_CODE.fd",
        "/usr/share/edk2/aarch64/QEMU_EFI.fd",
    ] {
        candidates.push(path.into());
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .context("no aarch64 UEFI firmware found; install it (e.g. apt install qemu-efi-aarch64)")
}

fn which(binary: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(binary)).find(|p| p.is_file())
}

/// A free TCP port on 127.0.0.1.
pub fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// The PID in `pidfile` if that process is still alive.
pub fn running_pid(pidfile: &Path) -> Option<i32> {
    let pid: i32 = std::fs::read_to_string(pidfile).ok()?.trim().parse().ok()?;
    // Signal 0 only checks that the process exists.
    (unsafe { libc::kill(pid, 0) } == 0).then_some(pid)
}

/// Send one QMP command, e.g. `system_powerdown` or `quit`.
pub fn qmp(port: u16, command: &str) -> Result<()> {
    let stream = TcpStream::connect(("127.0.0.1", port)).context("connecting to the QEMU monitor")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    reader.read_line(&mut line)?; // greeting
    for cmd in ["qmp_capabilities", command] {
        writeln!(writer, "{{\"execute\":\"{cmd}\"}}")?;
        line.clear();
        reader.read_line(&mut line)?;
    }
    Ok(())
}

/// Ask the guest to power off via ACPI and wait up to `timeout` for QEMU to
/// exit, then force it.
pub fn stop(pidfile: &Path, qmp_port: u16, timeout: Duration) -> Result<()> {
    let Some(pid) = running_pid(pidfile) else { return Ok(()) };
    if qmp(qmp_port, "system_powerdown").is_ok() && wait_exit(pidfile, timeout) {
        return Ok(());
    }
    kill(pid, pidfile)
}

/// Stop QEMU immediately, without a guest shutdown.
pub fn kill(pid: i32, pidfile: &Path) -> Result<()> {
    unsafe { libc::kill(pid, libc::SIGTERM) };
    if !wait_exit(pidfile, Duration::from_secs(10)) {
        unsafe { libc::kill(pid, libc::SIGKILL) };
        wait_exit(pidfile, Duration::from_secs(5));
    }
    Ok(())
}

fn wait_exit(pidfile: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if running_pid(pidfile).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    running_pid(pidfile).is_none()
}
