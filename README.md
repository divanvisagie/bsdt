# bsdt

Repeatable FreeBSD development VMs from a TOML file, for working on
FreeBSD targets from Linux or macOS. It aims for the same feel as
`docker compose up`: describe the environment in `bsdt.toml`, run
`bsdt up`, and get the same machine every time.

Website, install guide and manual: <https://bsdt.divanv.com>

bsdt boots the official FreeBSD `BASIC-CLOUDINIT` VM image with QEMU.
The image is downloaded once, checked against the release's
`CHECKSUM.SHA256`, and cached. Each project gets a copy-on-write overlay
disk on top of it, so throwing a VM away and starting fresh takes seconds.
On first boot bsdt sets up SSH access through FreeBSD's own `nuageinit`,
installs your packages, copies the project in with rsync, and runs your
provision commands.

NetBSD and OpenBSD are planned next, in that order.

## Usage

```sh
bsdt init                  # write a starter bsdt.toml
bsdt up                    # boot (and on first use create + provision) the VM
bsdt exec -- cargo test    # sync the project, then run a command in the guest
bsdt ssh                   # shell in the synced directory (--root for root)
bsdt sync                  # copy the project into the guest
bsdt provision             # reinstall packages and rerun provision commands
bsdt status                # is it running, and on which ports
bsdt logs [-F]             # the guest's serial console

bsdt gui                   # reopen the desktop window (gui = true)
bsdt key alt+return        # press keys on the VM's keyboard
bsdt type --enter 'ls'     # type text on it
bsdt down                  # shut down, keeping the disk
bsdt destroy               # delete the VM's disk and state

bsdt pull                  # download the base image without booting
bsdt images                # list cached base images
bsdt man [--install]       # print or install the bsdt(1) man page
```

`exec` and `ssh` work in the guest directory matching your current
directory, so running `bsdt exec -- make` from `src/` runs it in the
guest's copy of `src/`.

## bsdt.toml

```toml
[vm]
os = "freebsd"
version = "15.1"
# arch = "auto"         # auto (match the host), amd64 or aarch64
memory = "4G"
ports = ["8080"]        # or "HOST:GUEST"; bound to 127.0.0.1
# update = true        # install security updates on first boot (slower)
# audio = true         # a sound card that plays on the host

[packages]
install = ["rust", "git"]

[sync]
exclude = ["target"]    # not copied, and not deleted in the guest

[provision]
root = ["sysrc nginx_enable=YES"]
run = ["cargo fetch"]   # as the bsdt user, in the synced directory
```

Only `vm.os` and `vm.version` are required. The man page documents every
key. Use `-f other.toml` to give one project several VMs, for example a
second one on an older release.

## Desktop

```toml
[vm]
os = "freebsd"
version = "15.1"
gui = true
```

is enough for a sway desktop in a window. The first `bsdt up` installs
sway, seatd, wayvnc and the foot terminal, and every `up` starts them and
opens a VNC viewer on the host. Every setting under `[gui]` has a default;
see the man page to use your own desktop, port, resolution, modifier key
or full screen. Sway uses Alt as its modifier by default (`alt+return` for
a terminal), because the host desktop usually keeps Super for itself.

FreeBSD has no driver for QEMU's virtual graphics cards, so sway draws to
a headless output in software and wayvnc shares it. Input comes from the
VM's emulated USB keyboard and tablet through `/dev/input`, so
`bsdt key` and `bsdt type` can drive the desktop from scripts, and tools
that read input devices directly see those keys. `input = true` gives you
the keyboard and tablet without a desktop.

`audio = true` under `[vm]` adds a sound card that plays through the
host's sound server. It works with or without the desktop; programs that
use ALSA rather than OSS also need the `alsa-plugins` package.

On Linux, install a VNC viewer such as TigerVNC (`apt install
tigervnc-viewer`). On macOS, bsdt opens TigerVNC Viewer
(`brew install --cask tigervnc-viewer`). The built-in Screen Sharing can't
connect, because it requires a password and the desktop has none.

## Requirements

- QEMU: `qemu-system-x86_64` or `qemu-system-aarch64` (plus UEFI firmware
  for aarch64), and `qemu-img`
- `ssh`, `ssh-keygen`, `rsync` and `curl`
- For `gui = true`, a VNC viewer
- For `audio = true`, a QEMU with an audio backend for your sound server
  (on Debian and Ubuntu, `qemu-system-gui` adds PipeWire and PulseAudio)

Guests that match the host architecture use KVM on Linux and the
Hypervisor framework on macOS, so an Apple Silicon Mac gets a fast aarch64
FreeBSD guest. Other architectures fall back to emulation, which works but
is slow.

## How it works

- **Images** are cached in `~/.cache/bsdt/images` (Linux) or
  `~/Library/Caches/bsdt/images` (macOS). Set `BSDT_CACHE_DIR` to put them
  somewhere else.
- **Per-VM state** lives in `.bsdt/<file stem>/` beside `bsdt.toml`: the
  overlay disk, an SSH key pair generated for this VM, the console log and
  QEMU's pid file. The directory has its own `.gitignore`.
- **First boot**: bsdt writes a tiny FAT disk labelled `CIDATA` with
  cloud-config user data. `nuageinit` reads it to create a `bsdt` user and
  allow the VM's key to log in as `bsdt` and as root. No ISO tools are
  needed on the host.
- **Access** is SSH on a port forwarded from `127.0.0.1`. Syncing is a
  one-way `rsync --delete` from the host.

## Man page

`cargo install` only installs the binary, so the man page is built into it
instead. `bsdt man --install` writes it to `../share/man/man1/` relative
to the binary, which is `~/.cargo/share/man/man1/bsdt.1`. man-db searches
there automatically for anything in `~/.cargo/bin` on your `PATH`, so
`man bsdt` works with no `MANPATH` changes. `make install` does both
steps. The page's source is [`man/bsdt.1`](man/bsdt.1), written by hand in
mdoc. A test checks that it mentions every subcommand and flag.

The project website lives in [`docs/`](docs/) and is served by GitHub
Pages. `make docs` rebuilds it (needs [mandoc](https://mandoc.bsd.lv)):
the home page comes from [`templates/index.html`](templates/index.html),
and the manual page and license are rendered into the same layout. Run it
after editing the man page or templates, and commit the result.

## Development

Run `make` to list the targets: `build`, `install`, `test`, `lint`,
`docs`, and `publish-check`/`publish` for crates.io releases.

To try a change against a real VM, `make try` builds a debug binary and
boots [`examples/hello-c`](examples/hello-c), a C program that needs
nothing installed. `EXAMPLE=hello-rust` boots
[`examples/hello-rust`](examples/hello-rust) instead, and `EXAMPLE=gui`
boots [`examples/gui`](examples/gui), a sway desktop. `make try-down` and
`make try-destroy` stop and delete the VM. [TESTING.md](TESTING.md) is the
checklist to run by hand before merging `develop` into `master`.

## License

BSD 3-Clause; see [LICENSE](LICENSE).
