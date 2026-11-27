# Working on Windows

OurStick is built and tested on Windows in a virtual machine on a Linux PC
(Ubuntu 24.04, KVM). This is how that machine was set up on 2026-09-29, and
how it is used. The helper scripts are in `tools/windows-vm/`.

## The virtual machine

- **Windows 11 Enterprise LTSC 2024 Evaluation** (build 26100), free for 90
  days. It has Edge and WebView2, which Tauri needs, and little else. The
  consumer download page often refuses ("We encountered a problem"); the
  Evaluation Center's link for the English ISO works with plain `curl`:
  `https://go.microsoft.com/fwlink/?linkid=2289029&clcid=0x409&culture=en-us&country=us`.
- **quickemu** from its PPA (`ppa:flexiondotorg/quickemu`) with `swtpm`;
  `ovmf` comes with Ubuntu. `quickget windows 11` makes the configuration and
  the unattended setup, but its ISO and virtio driver downloads failed (the
  virtio ISO came back as a bot check page): take the virtio ISO from
  `https://fedorapeople.org/groups/virt/virtio-win/direct-downloads/archive-virtio/`.
- **The VM's folder must be under `$HOME`** (here `~/vm`): AppArmor lets
  `swtpm` write only there or in `/tmp`, and Windows 11 does not install
  without a TPM. The disk image can live elsewhere through a symbolic link.
- In the `.conf`: `cpu_cores="8"`, `ram="16G"`, `disk_size="100G"`,
  `port_forwards=("2222:22")`.
- **Once Windows is installed, comment out `iso=`** in the `.conf`. quickemu
  decides whether the disk is in use by the size of `disk.qcow2`, and with
  the disk image behind a symbolic link it sees 40 bytes: "Looks unused,
  booting from windows-11.iso", and it presses Enter at "Press any key to
  boot from CD". On 2026-10-01 Windows still booted from its disk (it was
  finishing an update), but the unattended setup could have started.

### Changes to quickget's unattended setup

In `unattended/autounattend.xml`, then `unattended.iso` made again
(`xorriso -as mkisofs -J -l -D -N -joliet-long -relaxed-filenames -V CDROM
-o unattended.iso unattended/`):

- Remove both `<ProductKey>` elements: they carry the Pro key, which stops
  the setup of an Evaluation image.
- Add `Microsoft-Windows-International-Core-WinPE` (en-US) to the
  `windowsPE` pass, or the setup stops at the language page.
- Add a `FirstLogonCommands` entry that runs
  `powershell -NoProfile -ExecutionPolicy Bypass -File F:\setup-ssh.ps1`,
  and copy `tools/windows-vm/setup-ssh.ps1` and your public key (as
  `authorized_key`) into `unattended/`.
- The setup still asks which image to install (Enterprise LTSC or N);
  Enter takes the first.

`setup-ssh.ps1` installs OpenSSH Server, allows only key login (password and
keyboard-interactive off: QEMU's port forward listens on every address of
the host), and keeps the display from turning off.

### Tools

Once, over ssh: `tools/windows-vm/install-tools.ps1` installs Git, Node.js 24
(its hash is checked), the Visual Studio 2022 Build Tools with the C++
workload, and Rust (rustup, stable, MSVC). Restart `sshd` afterwards so that
new sessions see the new `PATH`.

## Daily use

```
cd ~/vm && quickemu --vm windows-11-English-United-States.conf
ssh -p 2222 Quickemu@localhost
```

The VM has no access to the git remote. Send the code as a bundle:

```
git bundle create /tmp/ourstick.bundle main
scp -P 2222 /tmp/ourstick.bundle Quickemu@localhost:
ssh -p 2222 Quickemu@localhost 'cd src\ourstick; git pull ..\..\ourstick.bundle main'
```

(The first time: `git clone -b main ourstick.bundle src\ourstick`, after
`git config --global core.autocrlf false`.) Not in `%USERPROFILE%\ourstick`:
Windows ignores letter case, so that is the stick's own folder
(`%USERPROFILE%\OurStick`), and the app would rightly refuse to take it. Then `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --check` as anywhere. Debug builds are slow in the VM: set
`OURSTICK_TEST_TIME_SCALE=4` for the tests, so their "not too slow" time
bounds give it room (the bounds that show a limit at work stay as they
are; the direct-path test then shows that no limit held it by its relay
bytes alone).

### Packages

```
cd apps\app
npm.cmd ci
npm.cmd run tauri build
```

`npm.cmd`, not `npm`: PowerShell's default execution policy refuses
`npm.ps1`. Tauri fetches WiX and NSIS itself and makes both installers,
unsigned (SmartScreen warns of an unknown publisher; for a signed one see below), under
`target\release\bundle\`: `msi\OurStick_<version>_x64_en-US.msi` and
`nsis\OurStick_<version>_x64-setup.exe` (per user, no admin rights).

The NSIS installer puts the app in `%LOCALAPPDATA%\OurStick`, the folder that
also holds OurStick's data (database, blob cache). Its uninstaller removes
the files it installed and then the folder only if it is empty, so the data
stays, and with it every shared link; its "Delete the application data"
box removes `%LOCALAPPDATA%\com.ourstick.OurStick`, which OurStick does not use.

### A signed installer (releases)

The code-signing key (a HARICA certificate in Georgios Kakavelakis's name)
is on a hardware token at the Linux host, never in the VM. Tauri's
`signCommand`, in `apps/app/src-tauri/tauri.windows-sign.conf.json`, runs
`tools/windows-sign/sign-in-vm.ps1` for each file it signs: the app's
`.exe`, the five NSIS plugins, the uninstaller (NSIS's `!uninstfinalize`)
and the installer. That script puts the file in a queue in the VM's home
folder and waits; `tools/windows-sign/sign-on-host.sh`, which you run in
a terminal at the host, takes it over ssh, signs it with osslsigncode
(the token's PIN asked by osslsigncode, once per file) and puts it back.
The VM takes the file only if Windows calls its signature Valid, from that
signer, with a timestamp. The host keeps signed copies by hash, so the
plugins, the same in every build, ask for the PIN only the first time: a
release asks for three PINs.

At the host, with the token plugged in (the settings file, outside the
repository, names osslsigncode 2.16+, pkcs11-provider 1.3.0+, the token's
module, the certificate chain, the key's `id=` and the timestamp server):

```
bash tools/windows-sign/sign-on-host.sh <settings file>
```

In the VM, only the NSIS installer (the one the site offers):

```
cd apps\app
npm.cmd run tauri build -- --bundles nsis --config src-tauri/tauri.windows-sign.conf.json
```

Over ssh, PowerShell 5.1 counts Tauri's lines on stderr as errors and
ends with exit code 1 even when the build ended in `Finished 1 bundle`;
the signatures speak for themselves:
`Get-AuthenticodeSignature` on the installer, and after `setup.exe /S`
on `%LOCALAPPDATA%\OurStick\ourstick-app.exe` and `uninstall.exe`. A
silent install gives up (exit code 2) while OurStick is open: close it
first. A release build needs some 30 GB free in the VM; `target\debug`
of the tests can go.

### What needs the desktop session

An ssh session with key login has no Windows logon session, so some things
only work on the VM's desktop:

- **The Credential Manager** answers `ERROR_NO_SUCH_LOGON_SESSION` over ssh.
  The keyring test runs through `tools/windows-vm/desktop.ps1`:
  `powershell -ExecutionPolicy Bypass -File desktop.ps1 "cargo test -p ourstick-core -- --ignored keyring"`
  (the VM's execution policy refuses unsigned scripts otherwise).
- **`subst` letters** belong to the logon session that made them: one made
  over ssh does not show in Explorer.
- **Screenshots:** QEMU's `screendump` sees nothing once Windows has loaded
  the virtio GPU driver ("no surface"). `desktop.ps1` with
  `winshot.ps1` saves one to `%USERPROFILE%\winshot.png`.

- **The stick's entry in File Explorer's pane** (ADR 0023) is per user:
  `reg query HKCU\Software\OurStick\NavigationPane /s` lists the entries
  of each data folder, by class id; each is under
  `HKCU\Software\Classes\CLSID\{…}` (and `WOW6432Node\CLSID`) and
  `HKCU\…\Explorer\Desktop\NameSpace\{…}`. `explorer shell:::{…}`
  opens one.

If the desktop is locked (after the host slept, say), unlock it with QEMU's
monitor: `sendkey` the local password, then `ret`. The Evaluation expires
90 days after install; the watermark says when.
