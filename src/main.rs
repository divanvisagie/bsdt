mod config;
mod gui;
mod image;
mod keys;
mod os;
mod qemu;
mod seed;
mod ssh;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use config::{DEFAULT_FILE, Desktop, Project};
use qemu::Accel;
use ssh::{Ssh, User};

/// Repeatable FreeBSD development VMs from a TOML file.
#[derive(Parser)]
#[command(name = "bsdt", version, about)]
struct Cli {
    /// Environment file to use instead of searching for bsdt.toml in the
    /// current directory and its parents.
    #[arg(short, long, global = true)]
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter bsdt.toml in the current directory.
    Init,
    /// Boot the VM, creating and provisioning it on first use, then sync.
    Up,
    /// Shut the VM down, keeping its disk.
    Down,
    /// Stop the VM and delete its disk and state.
    Destroy,
    /// Show whether the VM exists and is running, and its ports.
    Status,
    /// Open a shell in the synced directory.
    Ssh {
        /// Log in as root instead of the bsdt user.
        #[arg(long)]
        root: bool,
    },
    /// Sync the project, then run a command in the matching guest directory.
    Exec {
        /// Run as root instead of the bsdt user.
        #[arg(long)]
        root: bool,
        /// Skip syncing the project first.
        #[arg(long)]
        no_sync: bool,
        /// The command and its arguments.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Copy the project directory into the guest.
    Sync,
    /// Install packages and run the provision commands again.
    Provision,
    /// Open a viewer window on the VM's desktop (needs gui = true).
    Gui,
    /// Press keys on the VM's keyboard, e.g. super+return; each argument is
    /// one chord, pressed in turn.
    Key {
        #[arg(required = true)]
        chords: Vec<String>,
    },
    /// Type text on the VM's keyboard (US layout).
    Type {
        text: String,
        /// Press Return afterwards.
        #[arg(long)]
        enter: bool,
    },
    /// Print the VM's serial console log.
    Logs {
        /// Keep printing as the log grows.
        #[arg(short = 'F', long)]
        follow: bool,
    },
    /// Download the base image for the environment file without booting.
    Pull,
    /// List cached base images.
    Images,
    /// Print the bsdt(1) manual page.
    Man {
        /// Write it under ../share/man/man1 next to the bsdt binary
        /// (e.g. ~/.cargo/share/man/man1 after `cargo install`) instead.
        #[arg(long)]
        install: bool,
    },
}

const MAN_PAGE: &str = include_str!("../man/bsdt.1");

/// What `bsdt up` records about the running VM in `state.json`.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    ssh_port: u16,
    qmp_port: u16,
    /// Host port forwarded to the guest's VNC server; 0 without gui.
    gui_port: u16,
    provisioned: bool,
    /// The default desktop's packages and seatd are set up.
    desktop_ready: bool,
}

/// Files under the project's state directory.
struct Paths {
    dir: PathBuf,
    disk: PathBuf,
    seed: PathBuf,
    key: PathBuf,
    console: PathBuf,
    pidfile: PathBuf,
    state: PathBuf,
}

impl Paths {
    fn new(project: &Project) -> Paths {
        let dir = project.state_dir();
        Paths {
            disk: dir.join("disk.qcow2"),
            seed: dir.join("seed.img"),
            key: dir.join("id_ed25519"),
            console: dir.join("console.log"),
            pidfile: dir.join("qemu.pid"),
            state: dir.join("state.json"),
            dir,
        }
    }

    fn load_state(&self) -> State {
        std::fs::read_to_string(&self.state).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    fn save_state(&self, state: &State) -> Result<()> {
        std::fs::write(&self.state, serde_json::to_string_pretty(state)?)?;
        Ok(())
    }

    fn ssh(&self, state: &State) -> Ssh {
        Ssh { key: self.key.clone(), port: state.ssh_port }
    }
}

/// The paths and state of a VM that must be running.
fn running(project: &Project) -> Result<(Paths, State)> {
    let paths = Paths::new(project);
    if qemu::running_pid(&paths.pidfile).is_none() {
        bail!("the VM is not running (start it with `bsdt up`)");
    }
    let state = paths.load_state();
    Ok((paths, state))
}

fn cmd_init(file: Option<&Path>) -> Result<()> {
    let path = file.unwrap_or(Path::new(DEFAULT_FILE));
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    std::fs::write(path, config::TEMPLATE)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn cmd_up(project: &Project) -> Result<()> {
    let vm = &project.config.vm;
    os::check_supported(vm.os)?;
    let paths = Paths::new(project);
    if let Some(pid) = qemu::running_pid(&paths.pidfile) {
        println!("already running (pid {pid}); `bsdt sync` to update files");
        return Ok(());
    }

    let gui = &project.config.gui;
    if vm.gui {
        gui.size()?;
    }

    let base = image::ensure(&os::image(vm)?)?;
    std::fs::create_dir_all(&paths.dir)?;
    std::fs::write(paths.dir.join(".gitignore"), "*\n")?;
    let mut state = paths.load_state();
    if !paths.disk.exists() {
        let public_key = ssh::ensure_key(&paths.key)?;
        qemu::create_overlay(&base, &paths.disk, &vm.disk)?;
        seed::write(&paths.seed, &project.hostname(), &public_key, vm.update)?;
        state.provisioned = false;
        state.desktop_ready = false;
    }

    state.ssh_port = qemu::free_port()?;
    state.qmp_port = qemu::free_port()?;
    state.gui_port = match (vm.gui, gui.port) {
        (false, _) => 0,
        (true, Some(port)) => port,
        (true, None) => qemu::free_port()?,
    };
    paths.save_state(&state)?;
    let arch = vm.arch.resolve();
    let accel = qemu::start(&qemu::Launch {
        arch,
        cpus: vm.cpus,
        memory: &vm.memory,
        disk: &paths.disk,
        seed: &paths.seed,
        ssh_port: state.ssh_port,
        qmp_port: state.qmp_port,
        ports: &vm.ports,
        vnc: vm.gui.then_some((state.gui_port, gui.vnc)),
        input: vm.input(),
        console: &paths.console,
        pidfile: &paths.pidfile,
    })?;
    if accel == Accel::Tcg {
        eprintln!("bsdt: warning: no hardware acceleration for {arch} on this host, so the VM will be slow");
    }
    eprintln!("bsdt: booting {} {} ({arch}); console log: {}", vm.os, vm.version, paths.console.display());

    let timeout = Duration::from_secs(if accel == Accel::Tcg { 1800 } else { 300 });
    wait_ready(&paths, &state, timeout)?;

    if !state.provisioned {
        provision(project, &paths, &state)?;
        state.provisioned = true;
        state.desktop_ready = vm.gui && project.config.gui.desktop == Desktop::Sway;
        paths.save_state(&state)?;
    } else {
        sync(project, &paths, &state)?;
    }
    if vm.gui {
        if gui.desktop == Desktop::Sway {
            if !state.desktop_ready {
                setup_desktop(project, &paths, &state)?;
                state.desktop_ready = true;
                paths.save_state(&state)?;
            }
            // Cheap and idempotent, and VMs set up by older versions of
            // bsdt need its seatd settings too.
            paths.ssh(&state).script(User::Root, &gui::sway_root_setup())?;
        }
        eprintln!("bsdt: starting the desktop");
        paths.ssh(&state).script(User::Guest, &gui::start_script(gui, &project.dest()))?;
        gui::wait_vnc(state.gui_port, Duration::from_secs(60))?;
    }

    println!("ready: {} {} at {}", vm.os, vm.version, project.dest());
    println!("  ssh      bsdt ssh");
    println!("  run      bsdt exec -- <command>");
    if vm.gui {
        println!("  gui      bsdt gui (vnc://127.0.0.1:{})", state.gui_port);
    }
    for port in &vm.ports {
        println!("  port     {port}");
    }
    if vm.gui && gui.open {
        open_viewer(state.gui_port, gui.fullscreen)?;
    }
    Ok(())
}


/// Install and configure the default sway desktop.
fn setup_desktop(project: &Project, paths: &Paths, state: &State) -> Result<()> {
    let ssh = paths.ssh(state);
    let packages: Vec<String> = gui::SWAY_PACKAGES.iter().map(|p| p.to_string()).collect();
    eprintln!("bsdt: installing the desktop");
    ssh.run_checked(User::Root, &os::install_packages(project.config.vm.os, &packages))?;
    ssh.script(User::Root, &gui::sway_root_setup())
}

fn open_viewer(port: u16, fullscreen: bool) -> Result<()> {
    match gui::open_viewer(port, fullscreen)? {
        Some(viewer) => eprintln!("bsdt: opened the desktop in {viewer}"),
        None if cfg!(target_os = "macos") => eprintln!(
            "bsdt: TigerVNC not found; install it with `brew install --cask tigervnc-viewer` \
             (Screen Sharing can't connect: it requires a password), then run `bsdt gui`"
        ),
        None => eprintln!(
            "bsdt: no VNC viewer found; install one (e.g. TigerVNC: apt install tigervnc-viewer) \
             or connect yours to 127.0.0.1:{port}"
        ),
    }
    Ok(())
}

/// Wait until root can log in and boot has finished: `/firstboot` is gone
/// and `/etc/rc` is no longer running. Checking rc as well covers the
/// window after a first-boot update removes `/firstboot` and reboots.
fn wait_ready(paths: &Paths, state: &State, timeout: Duration) -> Result<()> {
    let ssh = paths.ssh(state);
    let deadline = Instant::now() + timeout;
    let mut announced = false;
    loop {
        if qemu::running_pid(&paths.pidfile).is_none() {
            bail!("QEMU exited during boot; see {}\n{}", paths.console.display(), console_tail(&paths.console));
        }
        match ssh.probe(User::Root, BOOTED)?.code() {
            Some(0) => return Ok(()),
            Some(1) if !announced => {
                eprintln!("bsdt: waiting for boot to finish");
                announced = true;
            }
            _ => {}
        }
        if Instant::now() > deadline {
            bail!(
                "the VM did not become reachable over SSH within {}s; see {}\n{}",
                timeout.as_secs(),
                paths.console.display(),
                console_tail(&paths.console)
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Exits 0 once booted, 1 while still booting.
const BOOTED: &str = "test ! -e /firstboot && ! pgrep -f '^/bin/sh /etc/rc' >/dev/null";

fn console_tail(path: &Path) -> String {
    let log = std::fs::read(path).unwrap_or_default();
    let log = String::from_utf8_lossy(&log);
    let lines: Vec<&str> = log.lines().collect();
    lines[lines.len().saturating_sub(15)..].join("\n")
}

fn provision(project: &Project, paths: &Paths, state: &State) -> Result<()> {
    let config = &project.config;
    let ssh = paths.ssh(state);
    eprintln!("bsdt: installing packages");
    ssh.run_checked(User::Root, &os::install_packages(config.vm.os, &config.packages.install))?;
    sync(project, paths, state)?;
    if config.vm.gui && config.gui.desktop == Desktop::Sway {
        setup_desktop(project, paths, state)?;
    }
    for cmd in &config.provision.root {
        eprintln!("bsdt: [root] {cmd}");
        ssh.run_checked(User::Root, cmd)?;
    }
    let dest = ssh::quote(&project.dest());
    for cmd in &config.provision.run {
        eprintln!("bsdt: {cmd}");
        ssh.run_checked(User::Guest, &format!("cd {dest} && {cmd}"))?;
    }
    Ok(())
}

fn sync(project: &Project, paths: &Paths, state: &State) -> Result<()> {
    paths.ssh(state).rsync(&project.dir, &project.dest(), &project.config.sync.exclude)
}

fn cmd_down(project: &Project) -> Result<()> {
    let paths = Paths::new(project);
    if qemu::running_pid(&paths.pidfile).is_none() {
        println!("not running");
        return Ok(());
    }
    eprintln!("bsdt: shutting down");
    qemu::stop(&paths.pidfile, paths.load_state().qmp_port, Duration::from_secs(60))?;
    println!("stopped");
    Ok(())
}

fn cmd_destroy(project: &Project) -> Result<()> {
    let paths = Paths::new(project);
    if let Some(pid) = qemu::running_pid(&paths.pidfile) {
        qemu::kill(pid, &paths.pidfile)?;
    }
    if paths.dir.exists() {
        std::fs::remove_dir_all(&paths.dir).with_context(|| format!("removing {}", paths.dir.display()))?;
        println!("removed {}", paths.dir.display());
    } else {
        println!("nothing to destroy");
    }
    Ok(())
}

fn cmd_status(project: &Project) -> Result<()> {
    let vm = &project.config.vm;
    let paths = Paths::new(project);
    println!("file      {}", project.file.display());
    println!("guest     {} {} {} ({})", vm.os, vm.version, vm.arch.resolve(), vm.filesystem);
    println!("sync      {} -> {}", project.dir.display(), project.dest());
    let state = match qemu::running_pid(&paths.pidfile) {
        Some(pid) => format!("running (pid {pid})"),
        None if paths.disk.exists() => "stopped".into(),
        None => "not created".into(),
    };
    println!("state     {state}");
    if qemu::running_pid(&paths.pidfile).is_some() {
        let state = paths.load_state();
        println!("ssh       127.0.0.1:{}", state.ssh_port);
        if state.gui_port != 0 {
            println!("gui       vnc://127.0.0.1:{}", state.gui_port);
        }
        for port in &vm.ports {
            println!("port      {port}");
        }
    }
    Ok(())
}

fn cmd_ssh(project: &Project, root: bool) -> Result<i32> {
    let (paths, state) = running(project)?;
    let user = if root { User::Root } else { User::Guest };
    Ok(paths.ssh(&state).shell(user, &project.guest_cwd())?.code().unwrap_or(1))
}

fn cmd_exec(project: &Project, root: bool, no_sync: bool, command: &[String]) -> Result<i32> {
    let (paths, state) = running(project)?;
    if !no_sync {
        sync(project, &paths, &state)?;
    }
    let user = if root { User::Root } else { User::Guest };
    let args: Vec<String> = command.iter().map(|a| ssh::quote(a)).collect();
    let script = format!("cd {} && {}", ssh::quote(&project.guest_cwd()), args.join(" "));
    Ok(paths.ssh(&state).run(user, &script)?.code().unwrap_or(1))
}

fn cmd_gui(project: &Project) -> Result<()> {
    let (_, state) = running(project)?;
    if state.gui_port == 0 {
        bail!("this VM has no desktop; set gui = true under [vm] and run `bsdt down && bsdt up`");
    }
    open_viewer(state.gui_port, project.config.gui.fullscreen)
}

fn cmd_key(project: &Project, chords: &[String]) -> Result<()> {
    let chords = chords.iter().map(|c| keys::chord(c)).collect::<Result<Vec<_>>>()?;
    send_keys(project, &chords)
}

fn cmd_type(project: &Project, text: &str, enter: bool) -> Result<()> {
    let mut chords = keys::text(text)?;
    if enter {
        chords.push(vec!["ret".into()]);
    }
    send_keys(project, &chords)
}

fn send_keys(project: &Project, chords: &[Vec<String>]) -> Result<()> {
    let (_, state) = running(project)?;
    let mut qmp = qemu::Qmp::connect(state.qmp_port)?;
    for chord in chords {
        let codes: Vec<&str> = chord.iter().map(String::as_str).collect();
        qmp.chord(&codes)?;
    }
    Ok(())
}

fn cmd_logs(project: &Project, follow: bool) -> Result<()> {
    let paths = Paths::new(project);
    if !paths.console.exists() {
        bail!("no console log yet; it is created by `bsdt up`");
    }
    if follow {
        let status = std::process::Command::new("tail").arg("-f").arg(&paths.console).status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    std::io::stdout().write_all(&std::fs::read(&paths.console)?)?;
    Ok(())
}

fn cmd_images() -> Result<()> {
    let dir = image::images_dir()?;
    let images = image::list()?;
    if images.is_empty() {
        println!("no cached images in {}", dir.display());
    }
    for (path, size) in images {
        println!("{:>8.1} MiB  {}", size as f64 / (1 << 20) as f64, dir.join(path).display());
    }
    Ok(())
}

fn cmd_man(install: bool) -> Result<()> {
    if !install {
        std::io::stdout().write_all(MAN_PAGE.as_bytes())?;
        return Ok(());
    }

    let exe = std::env::current_exe().context("locating the bsdt binary")?;
    let prefix = exe
        .parent()
        .and_then(|bin| bin.parent())
        .context("the bsdt binary has no parent directory to install under")?;
    let dir = prefix.join("share/man/man1");
    let path = dir.join("bsdt.1");
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, MAN_PAGE))
        .with_context(|| format!("writing {} (run `bsdt man > <path>` to put it somewhere else)", path.display()))?;
    println!("installed {}", path.display());
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let file = cli.file.as_deref();
    match &cli.command {
        Command::Init => return cmd_init(file),
        Command::Images => return cmd_images(),
        Command::Man { install } => return cmd_man(*install),
        _ => {}
    }

    let project = Project::load(file)?;
    match &cli.command {
        Command::Up => cmd_up(&project),
        Command::Down => cmd_down(&project),
        Command::Destroy => cmd_destroy(&project),
        Command::Status => cmd_status(&project),
        Command::Ssh { root } => std::process::exit(cmd_ssh(&project, *root)?),
        Command::Exec { root, no_sync, command } => std::process::exit(cmd_exec(&project, *root, *no_sync, command)?),
        Command::Sync => {
            let (paths, state) = running(&project)?;
            sync(&project, &paths, &state)
        }
        Command::Provision => {
            let (paths, state) = running(&project)?;
            provision(&project, &paths, &state)
        }
        Command::Gui => cmd_gui(&project),
        Command::Key { chords } => cmd_key(&project, chords),
        Command::Type { text, enter } => cmd_type(&project, text, *enter),
        Command::Logs { follow } => cmd_logs(&project, *follow),
        Command::Pull => {
            let path = image::ensure(&os::image(&project.config.vm)?)?;
            println!("{}", path.display());
            Ok(())
        }
        Command::Init | Command::Images | Command::Man { .. } => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The man page is written by hand, so make sure it keeps up with the
    /// CLI: every subcommand and long flag needs at least a mention.
    #[test]
    fn man_page_covers_cli() {
        fn check(cmd: &clap::Command, missing: &mut Vec<String>) {
            for arg in cmd.get_arguments() {
                // mdoc writes `--flag` as `Fl -flag`.
                if let Some(long) = arg.get_long().filter(|long| !MAN_PAGE.contains(&format!("Fl -{long}"))) {
                    missing.push(format!("--{long}"));
                }
            }
            for sub in cmd.get_subcommands() {
                if sub.get_name() != "help" && !MAN_PAGE.contains(&format!("Cm {}", sub.get_name())) {
                    missing.push(sub.get_name().to_string());
                }
                check(sub, missing);
            }
        }

        let mut missing = Vec::new();
        check(&Cli::command(), &mut missing);
        assert!(missing.is_empty(), "not documented in man/bsdt.1: {}", missing.join(", "));
    }
}
