# Testing bsdt by hand

`make test` covers parsing, URLs, quoting and the seed disk, but most of
what bsdt does only happens against a real VM. This is the checklist for
that. Run it before merging `develop` into `master`, and on each host you
care about (Linux amd64 with KVM, macOS on Apple Silicon).

## Setup

You need QEMU (`qemu-system-x86_64` or `qemu-system-aarch64`, plus
`qemu-img`), `ssh`, `rsync` and `curl`. On Apple Silicon, Homebrew's
`qemu` includes the aarch64 UEFI firmware.

There are two ways to run your working copy:

- **Without installing**: `make try` builds a debug binary and runs
  `bsdt up` in `examples/hello-c`. To use that binary for everything else:

  ```sh
  export PATH="$PWD/target/debug:$PATH"
  cd examples/hello-c
  ```

- **Installed**: `make install` puts this checkout's `bsdt` and man page
  in `~/.cargo`, replacing any copy from crates.io. Reinstall from crates.io
  afterwards with `cargo install bsdt` if you want the release back.

Each example keeps its VM in its own `.bsdt/` directory, which ignores
itself in git. `make try-down` and `make try-destroy` stop and delete the
example VM. Add `EXAMPLE=hello-rust` or `EXAMPLE=gui` to any try target to use
another example.

The first run downloads a FreeBSD image of about 650 MB. Later runs reuse
it from `~/.cache/bsdt/images` (`~/Library/Caches/bsdt/images` on macOS).

## The examples

- **`examples/hello-c`** is the smoke test. Nothing needs installing, so a
  full `destroy` and `up` cycle takes under a minute. It forwards
  `127.0.0.1:18080` to port 8080 in the guest, runs a root and a user
  provision command, and excludes the built `hello` binary from syncing.
- **`examples/gui`** is `gui = true` and nothing else: a sway desktop in a
  VNC viewer window.
- **`examples/hello-rust`** is the real use case: install `rust`, edit on
  the host, and build on FreeBSD. The first `up` takes a few minutes while
  the package installs.

## Checklist

Run these from `examples/hello-c` unless noted. The expected result is
under each step.

### First boot

1. `make try-destroy && make try` (from the repo root)
   - `ready: freebsd 15.1 at /home/bsdt/hello-c` in well under a minute
     once the image is cached.
   - The output shows `sysrc` setting `hello_greeting` and `cc` building
     `hello`.
   - No `waiting for boot to finish` hang and no reboot. `bsdt logs`
     shows a single boot.
2. `bsdt status`
   - `running (pid …)`, an `ssh 127.0.0.1:<port>` line, and
     `port 127.0.0.1:18080 -> 8080`.
3. `bsdt up` again
   - `already running`, and nothing else happens.

### exec, ssh and sync

4. `bsdt exec -- make run`
   - `hello from FreeBSD 15.1-RELEASE on amd64` (`arm64` on Apple
     Silicon).
5. Edit the greeting in `hello.c`, then `bsdt exec -- make run`.
   - `cc` rebuilds, and the new greeting is printed. Undo the edit.
6. `bsdt exec -- sh -c 'exit 7'; echo $?`
   - `7`.
7. `bsdt exec --root -- whoami`
   - `root`.
8. `mkdir sub && cd sub && bsdt exec -- pwd`
   - `/home/bsdt/hello-c/sub`. Then `cd .. && rmdir sub`.
9. `touch extra && bsdt sync && bsdt exec --no-sync -- ls`, then
   `rm extra && bsdt exec -- ls`
   - `extra` appears in the guest, then disappears.
   - `hello` (built in the guest, excluded from sync) is still there.
10. `bsdt ssh`
    - A shell in `/home/bsdt/hello-c` as `bsdt`. `exit` to leave.
      `bsdt ssh --root` gives a root shell.

### Ports

11. `bsdt exec -- make serve` in one terminal, `curl 127.0.0.1:18080` in
    another.
    - curl prints the hello line. Stop the server with ctrl-c.

### Lifecycle

12. `bsdt down`
    - `stopped` within a few seconds. `bsdt status` says `stopped`.
13. `bsdt up`
    - Boots the existing disk without provisioning again. `hello` is
      still built.
14. `bsdt destroy`
    - Removes `.bsdt/bsdt`. `bsdt status` says `not created`.

### First-boot updates

15. Add `update = true` under `[vm]` in `examples/hello-c/bsdt.toml`, then
    `make try-destroy && make try`.
    - Takes a minute or two longer. `bsdt logs` shows `pkg` upgrading
      `FreeBSD-base` and the guest rebooting, and `up` still finishes.
    - `bsdt exec -- freebsd-version -ku` shows a patch level, such as
      `15.1-RELEASE-p4`.
    - Undo the change and run `make try-destroy`.

### The Rust example

16. `make try EXAMPLE=hello-rust`, then from `examples/hello-rust`:
    `bsdt exec -- cargo run`
    - `hello from freebsd on x86_64` (`aarch64` on Apple Silicon).
17. Edit `src/main.rs`, then `bsdt exec -- cargo run` again.
    - Only an incremental rebuild: the guest's `target/` is kept across
      syncs.
18. `make try-destroy EXAMPLE=hello-rust`

### Desktop

Install a VNC viewer first (TigerVNC on Linux; on macOS try both TigerVNC
Viewer and the built-in Screen Sharing). Then:

19. `make try EXAMPLE=gui`
    - About a minute and a half the first time while the desktop installs.
    - `ready` lists a `gui` line, and a viewer window opens on a sway
      desktop with a foot terminal in `/home/bsdt/gui`.
    - On macOS, note whether Screen Sharing connects. wayvnc has no
      password, which it may refuse.
20. Type into foot in the viewer window, and press super+return there.
    - The text appears, and a second terminal opens.
21. From `examples/gui`: `bsdt type --enter 'echo "typed by bsdt: A_b-C!"'`
    - The line appears in the focused terminal exactly as written.
22. `bsdt key super+return`
    - Another terminal opens.
23. `bsdt key ctrl+bogus`
    - Fails with QEMU rejecting `bogus`. Typing in the viewer afterwards
      is not stuck with Ctrl held.
24. Close the viewer window, then `bsdt gui`
    - It opens again.
25. `bsdt down && bsdt up`
    - The desktop comes back in well under a minute, without
      reinstalling anything.
26. `make try-destroy EXAMPLE=gui`

### Errors

From an empty temporary directory:

27. `bsdt status`
    - `no bsdt.toml in … or its parents (run bsdt init to create one)`.
28. `bsdt init && bsdt init`
    - `wrote bsdt.toml`, then `bsdt.toml already exists`.
29. Set `version = "13.9"`, then `bsdt pull`
    - Fails with a 404 on `CHECKSUM.SHA256` and suggests checking the
      version.
30. Set `os = "openbsd"`, then `bsdt up`
    - `openbsd guests are planned but not supported yet`.

### Man page

31. `bsdt man | man -l -` (`mandoc -a` on macOS: `bsdt man | mandoc -a`)
    - The page renders, with every command listed.

## Platform notes

- **Linux without KVM access** (no read/write on `/dev/kvm`): `up` warns
  that there is no hardware acceleration and boots slowly. Add yourself to
  the `kvm` group to fix it.
- **Apple Silicon**: guests default to aarch64 and use HVF. This has not
  been tested yet, so watch for problems finding the UEFI firmware and
  booting, and report what `bsdt logs` shows. The desktop (steps 19 to
  26) is untested there too.
- **`arch = "amd64"` on Apple Silicon, or `aarch64` on an x86 host**: works
  through emulation, but expect first boot to take several minutes.

## When something fails

- `bsdt logs` (or `bsdt logs -F` while booting) shows the serial console.
  `up` also prints its last lines when it times out.
- The VM's SSH key is `.bsdt/bsdt/id_ed25519`, and the port is in
  `.bsdt/bsdt/state.json`, if you want to connect with plain `ssh`.
- `/var/log/nuageinit.log` in the guest shows what first-boot setup did.
- With `gui = true`, `/tmp/bsdt-sway.log` and `/tmp/bsdt-wayvnc.log` in the
  guest show why the desktop or VNC server did not start.
