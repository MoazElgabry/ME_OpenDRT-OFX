# Resolve / OFX Host Subtree

This folder owns the Resolve/OpenFX-specific host layer for `ME_OpenDRT`.

- OFX descriptors, groups, pages, and host actions live here.
- Shared parameter logic, presets, and CPU processing belong in `core/`.
- Shared viewer protocol/helpers belong in `shared/viewer/`.
- GPU backend kernels may remain under `src/` while they are shared across hosts.

The current Resolve product identity and bundle layout remain unchanged by this separation.
