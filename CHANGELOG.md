# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

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
