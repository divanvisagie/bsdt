# bsdt

Repeatable FreeBSD development VMs from a TOML file, for working on
FreeBSD targets from Linux or macOS. It aims for the same feel as
`docker compose up`: describe the environment in `bsdt.toml`, run
`bsdt up`, and get the same machine every time.

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

## Requirements

- QEMU: `qemu-system-x86_64` or `qemu-system-aarch64` (plus UEFI firmware
  for aarch64), and `qemu-img`
- `ssh`, `ssh-keygen`, `rsync` and `curl`

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

[`docs/index.html`](docs/index.html) is the same page rendered to HTML for
the web. Regenerate it with `make docs` (needs
[mandoc](https://mandoc.bsd.lv)) after editing the man page, and commit
the result.

## Development

Run `make` to list the targets: `build`, `install`, `test`, `lint`,
`docs`, and `publish-check`/`publish` for crates.io releases.

## License

BSD 3-Clause; see [LICENSE](LICENSE).
