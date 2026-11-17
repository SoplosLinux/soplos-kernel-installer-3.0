# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [3.0.0] - 2026-11-17

First stable release.

## [3.0.0-6-beta] - 2026-11-14

### Fixed
- The x86-64 level used to pick the XanMod package, the other kernels of the list and the `march` of a build was read from the flags of the processor, while the window showed the level the dynamic linker reports (`ld.so --help`). They could differ: an Intel N150 in a Proxmox virtual machine, which runs v3 kernels, showed v3 and got v2. The level now comes from the dynamic linker everywhere, and from the flags only when it gives no answer.

## [3.0.0-5-beta] - 2026-11-08

### Fixed
- When the compilation of a kernel failed, the message showed only the last four lines of the output, which are the `make` of the packages saying that it stopped. It now also shows the first lines that say what went wrong (`error:`, `fatal error`, `undefined reference`, `No rule to make target`), before the last lines. The whole output stays in `~/kernel_build/build.log`.

## [3.0.0-4-beta] - 2026-11-08

### Fixed
- The patches of a recipe were not applied all or none. GNU `patch` applies the hunks that fit and leaves the others in a `.rej` file when one does not fit, so a patch that failed left files with some of its changes, and the patches applied before it stayed applied. Now each patch is tried with `--dry-run` and applied only if it fits whole; a patch that is in the tree already counts as applied only if all of it is there. If one patch does not apply, or the build is cancelled half way, the ones applied before come out again in reverse order and the tree is as it was; if they cannot come out, the sources are deleted, so a kernel with some of its patches is never built or reused.
- The BORE patch of firelzrd was looked for in a folder layout that the repository no longer has (`patches/stable/linux-6.18-bore/`), so for a line like 6.18 it was never found. It is now also looked for in the layout it has now, loose files with the point release in the name (`0001-linux6.18.48-bore-7.0.0.patch`); the one made for the newest point release is tried first, and the first that applies is kept. The order stays: Soplos, firelzrd (both layouts), CachyOS.

### Added
- How long each kernel of a batch takes. The kernel in course shows its own time next to the stage, the finished ones show what each took, and the summary of a finished queue shows the total.
- A history of how long each kernel took to build, kept in `~/.local/share/soplos-kernel-installer/build-times.json` (the last 500), for the kernels of a batch and for the ones built on their own, counting from the download to the packages. The Batch page lists the last ones, the newest first, with the date, the kernel, the version and the time.

## [3.0.0-3-beta] - 2026-11-08

### Fixed
- A kernel with the Rust Binder (6.18 and newer) failed with `ANDROID_BINDER_IPC_RUST was dropped by olddefconfig` when the `rustc` of the PATH was the one of rustup, which has no `rust-src`: the kernel takes `rustc` from the PATH, the check of the tools only looked at the package `rust-src` of Debian, and the build failed late. The check now counts Rust as there when the package of Debian is, or when the `rustc` of the PATH finds the source of the standard library, and the version it shows is the one the build uses. When the `rustc` of the PATH has no source and the one of `/usr/bin` has it, `/usr/bin` goes first in the PATH of every `make` of the build, and the log says so.
- A batch was not known as a batch by the window: the Build page drew the stages of a single kernel, with Install among them, and the Batch page said another build was in course. The window now asks the program how the build goes every two seconds, knowing it is a batch or not.
- The output of a build did not go down to its last line. It follows it now, unless it was scrolled up to read. It is as tall as the room left in the window, so the page has no scroll of its own and the two no longer fight; the wheel does not pass from the output to the page.
- The Batch page showed the result of the last queue instead of the page. The page of the batch is always there. A queue that ended or failed says how, once, on top of it, and it goes when the page is left; a queue that was cancelled says nothing.
- The kernels of a batch were listed in two columns that mixed the levels. They are in four groups, Kernels V1 to Kernels V4.

### Added
- What a build costs the computer, as in the 2.x program: the time it has been running, the load of the CPU (and the load of 1, 5 and 15 minutes), the memory, the disk of the folder of builds and the temperature of the CPU, read every two seconds, in the Build and the Batch pages.
- The follow-up of a batch in the Build page too, with the stage, the output and what is finished, as in the Batch page. A strip along the bottom of the other pages says which kernel is being built and what the build says now; a click leads to the Build page.
- Cancelling a build, in the Build page or in a batch, asks whether to delete the folder of the builds (`~/kernel_build`) or keep it, as the 2.x program did. It is deleted when the build has stopped.
- Ctrl+Tab and Ctrl+Shift+Tab move to the next and the previous page.

### Changed
- The Batch page is in four boxes, Kernel version (with the revision), Destination folder, Options and Kernels, and the button that starts the queue is outside of them, on the right.
- The message of a build that is not of the Batch page says what it is doing and has a button to see the details.

## [3.0.0-2-beta] - 2026-11-07

### Added
- The package installs, only on a computer that has an NVIDIA graphics card, the kernel hook of the nvidia-patches project (`00-soplos-nvidia-patches`) in `/etc/kernel/postinst.d` and `/etc/kernel/header_postinst.d`, and runs it once. The hook runs before the DKMS hook of Debian when a kernel or its headers are installed: it downloads the latest patches (checked against the sha256 of their MANIFEST, with the ones downloaded before as a fallback) and applies them to the NVIDIA sources, so a kernel that arrives from a terminal builds the module. It never makes the installation of a kernel fail. The same hook is put in place by Soplos Welcome, and the package removes it when it is removed, unless Soplos Welcome is still installed. On a computer with no NVIDIA graphics card nothing is left in `/etc` or `/var`.

## [3.0.0-1-beta] - 2026-11-07

### Fixed
- Installing or updating a kernel of Soplos from the Kernels page did not patch the sources of the NVIDIA driver, as the 2.x program did. The module could fail to build, and the patches did not stay in `/usr/src` for the kernels that arrive later from a terminal. The patches are applied before `apt-get install` and before `apt-get install --only-upgrade`, when the computer has an NVIDIA graphics card.

### Added
- The package patches the sources of the NVIDIA driver when it is installed or updated (`postinst`, which runs `soplos-kernel-installer --patch-nvidia` as root and never makes the installation fail).
- Four screenshots of the program, in English, in `src-tauri/resources/screenshots/` and in the README: Build, Kernels, Installed and Secure Boot.

## [3.0.0-beta] - 2026-11-06

First beta, for the development team. Everything of the alpha is there: the pages Build, Kernels, Installed, Secure Boot and Batch. It has not yet been tested with a complete kernel build on real hardware, nor with Secure Boot on a real firmware: report anything that fails.

### Added
- Keyboard shortcuts: F1 for About, Ctrl+1 to Ctrl+5 for the pages, Ctrl+Shift+D to show or hide Batch and Ctrl+Q to quit. The sidebar shows the shortcut of each page.
- The package: launcher in 8 languages, AppStream metadata, manual page, copyright file, the polkit action and a script that removes the signing hook and the copy of the key when the program is removed.
- The right button of the mouse is off in the installed program, as in the other 3.0 programs. It stays on in development, where the inspector is reached with it.

## [3.0.0-alpha] - 2026-11-06

### Added
- First skeleton of Soplos Kernel Installer 3.0 on Tauri 2, Svelte 4 and Tailwind 3.
- Frameless card window with the sidebar, light and dark theme that follows the desktop, and the interface in 8 languages (en, es, de, fr, it, pt, ro, ru).
- Pages of the 2.x version, still empty: Build, Soplos kernels, Installed, Secure Boot and Batch (hidden, shown with Ctrl+Shift+D).
- Hero image on every page, with the title of the page and the kernel that is running now. The pages have no top bar.
- Soplos kernels page: add, refresh and remove the repository; list the kernels with filters by profile and level; install, update and remove them. The architecture level of the computer is shown highlighted.
- Kernels page also installs and removes the kernels of Debian (standard and real-time), XanMod (main, real-time, edge and LTS, chosen by the level of the processor) and Liquorix. The repositories of XanMod and Liquorix are added by the program and taken away again if apt cannot read them.
- Build page as an assistant: what the computer is for, which kernel (the newest stable one is recommended; the long-term ones; and, if the user wants to help to test, the release candidates), what this computer is like (architecture level, cores, free space, tools, Rust, NVIDIA, signing), and a recipe that can be edited before the build. The list of kernels comes from the JSON of kernel.org. The build runs in the background, in `~/kernel_build`: it checks the download against the hash of kernel.org, applies the patches (the rebases of Soplos first; a patch that does not apply is not accepted), configures the kernel with the checks of the 2.x program, compiles it, and installs it asking for the password only for the tools and for the install. The build goes on when the page is left, and can be cancelled. Rust is part of the kernels that have the Rust Binder (6.18 and newer); the older ones are built without Binder and without Rust. NVIDIA: its driver is patched before the install, and what DKMS did with its module is reported at the end.
- Batch page (hidden, Ctrl+Shift+D): builds a whole release of Soplos kernels one after another, 18 in all (4 variants for each of the 4 architecture levels: standard, BORE with NTSYNC, RT and Zen, and X3D for V3 and V4; BORE alone and NTSYNC alone are not built any more). The destination folder is chosen with the file chooser and remembered. Each one starts from fresh sources, comes with a metapackage that pulls its image and headers, and is saved in `<destination>/<version>/V<level>/`, with a `revN` folder for a recompile. A queue can be resumed, skipping what is already in the destination. If one fails the queue stops and its sources and log are kept. A release candidate, a long-term or an end-of-life kernel gets its channel in the package name, so it never replaces a stable one. The download is kept between kernels, checked against its hash, and the sources are made again each time.
- Fixed from the 2.x program: a patch that was already applied counted as a failure, the automatic profile took every graphics card for an AMD, and the download was not checked.
- Secure Boot page as a step by step guide: check the computer, create the key, make the firmware trust it, trust the key of the DKMS drivers when there are any, sign the installed kernels and the bootloaders, and turn Secure Boot on. The password of the firmware goes to the administrator step through its standard input, never through a file. Removing the key and resetting the key database are under Advanced. The guide detects the bootloader of the boot partition and the one the firmware used, tells who signed every kernel and the GRUB (Debian, the key of the user or nobody), shows the version of GRUB and checks the whole chain before Secure Boot is turned on. The three bootloaders are covered: GRUB and systemd-boot through shim and the key of the user (MOK, or the database of the firmware when systemd-boot is started directly), and Limine through the database of the firmware, with the hash of its configuration enrolled and a hash on every file it loads. A bootloader is told by the content of its binary, not by its name. A section shows the computer (model, firmware, UEFI or legacy, setup mode, TPM) and the bootloaders found. A summary at the top shows the state of Secure Boot, of the key of the user and of the key of the drivers, and the keys the firmware trusts. Automatic signing: apt signs again, after every update, the kernels and the GRUB that were left without a signature, with a copy of the key that only root can read. The Advanced options sit in the steps they belong to. The guide opens at its first step.
- Installed page offers to revoke the key of the user when the last kernel signed with it is removed, but only if nothing else is signed with it (a bootloader signed with it that stayed would not start without it).
- Installed page: the kernels found in /boot with their profile, patches and date, and removing one (never the one that is running). It reads the history file of the 2.x program.
- Administrator steps run as named operations under one polkit action, with their output followed line by line.
