# Attribution and license

MemReduct RS is an independent Rust reimplementation inspired by:

- Project: Mem Reduct
- Author: Henry++
- Copyright: (c) 2011–2026 Henry++
- Source: https://github.com/henrypp/memreduct
- Reference commit: 04402de8b6a8ff9d1c5517c675d87c346655323c
- Upstream license: GNU General Public License, version 3

The reference project's memory-list operations and privilege requirements were
studied when implementing the backend. The complete upstream GPLv3 license text
is retained in LICENSE. No upstream branding assets, C implementation files or
submodules are bundled. This repository's Rust implementation is licensed under
GPL-3.0-only and is not affiliated with or endorsed by the upstream author.

Dependencies retain their own licenses; Cargo.lock pins the versions used.
When distributing executables, include this notice, LICENSE and the corresponding
source code. Corresponding source and installers are published together on GitHub Releases.

Additional implementation references studied for v0.2.0:
- IgorMundstein/WinMemoryCleaner: process exclusions, per-process working-set APIs
  and its administrator manifest. No C# code or binary is bundled.
- winsiderss/phnt: native system information layouts (ABI reference only).
- Microsoft Windows SDK documentation: token privileges, EmptyWorkingSet,
  SetSystemFileCacheSize and application manifests.
