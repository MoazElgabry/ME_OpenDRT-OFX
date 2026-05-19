# Shared OpenDRT Core

This folder owns host-neutral `ME_OpenDRT` logic.

Expected contents:

- parameter model and derived parameter computation
- CPU reference implementation
- preset data and preset application helpers
- host-neutral processing orchestration that can be reused by Resolve and Premiere

Rules:

- no OFX descriptors or Adobe command selectors here
- no host UI logic here
- keep behavior deterministic and easy to compare against GPU backends
