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
    /// Host port forwarded to the guest's VNC server, when `gui` is on.
    pub vnc: Option<(u16, u16)>,
    /// Attach a USB keyboard and tablet.
    pub input: bool,
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
    if let Some((host, guest)) = l.vnc {
        netdev.push_str(&format!(",hostfwd=tcp:127.0.0.1:{host}-:{guest}"));
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
    if l.input {
        // USB rather than virtio input: FreeBSD's USB HID drivers feed evdev,
        // so the devices show up in /dev/input for libinput.
        cmd.args(["-device", "qemu-xhci", "-device", "usb-kbd", "-device", "usb-tablet"]);
    }
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

/// A QMP connection to a running QEMU.
pub struct Qmp {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Qmp {
    pub fn connect(port: u16) -> Result<Qmp> {
        let stream = TcpStream::connect(("127.0.0.1", port)).context("connecting to the QEMU monitor")?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut qmp = Qmp { reader: BufReader::new(stream.try_clone()?), writer: stream };
        qmp.read()?; // greeting
        qmp.execute("qmp_capabilities", serde_json::Value::Null)?;
        Ok(qmp)
    }

    /// Run `command`, failing if QEMU returns an error.
    pub fn execute(&mut self, command: &str, arguments: serde_json::Value) -> Result<serde_json::Value> {
        let mut request = serde_json::json!({ "execute": command });
        if !arguments.is_null() {
            request["arguments"] = arguments;
        }
        writeln!(self.writer, "{request}")?;
        loop {
            let reply = self.read()?;
            if let Some(error) = reply.get("error") {
                bail!("QEMU rejected {command}: {}", error["desc"].as_str().unwrap_or("unknown error"));
            }
            // Asynchronous events can arrive before the reply.
            if let Some(ret) = reply.get("return") {
                return Ok(ret.clone());
            }
        }
    }

    fn read(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            bail!("the QEMU monitor closed the connection");
        }
        Ok(serde_json::from_str(&line)?)
    }

    /// Press or release one key, by QEMU key code name.
    pub fn key(&mut self, code: &str, down: bool) -> Result<()> {
        let event = serde_json::json!({ "type": "key", "data": { "down": down, "key": { "type": "qcode", "data": code } } });
        self.execute("input-send-event", serde_json::json!({ "events": [event] }))?;
        // The guest's USB keyboard polls; sending a whole chord in one
        // command, or keys back to back, loses some of them.
        std::thread::sleep(Duration::from_millis(30));
        Ok(())
    }

    /// Press `codes` in order, then release them in reverse. If QEMU rejects
    /// a key, the ones already pressed are released so none stay held.
    pub fn chord(&mut self, codes: &[&str]) -> Result<()> {
        for (i, code) in codes.iter().enumerate() {
            if let Err(err) = self.key(code, true) {
                for held in codes[..i].iter().rev() {
                    self.key(held, false).ok();
                }
                return Err(err);
            }
        }
        for code in codes.iter().rev() {
            self.key(code, false)?;
        }
        Ok(())
    }
}

/// Send one QMP command, e.g. `system_powerdown` or `quit`.
pub fn qmp(port: u16, command: &str) -> Result<()> {
    Qmp::connect(port)?.execute(command, serde_json::Value::Null)?;
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
