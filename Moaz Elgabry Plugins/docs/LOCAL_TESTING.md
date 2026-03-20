# Local Testing

`Moaz Elgabry Plugins` can use the local dev feed from:

- `docs/plugins/dev/index.json`

The active local `ME_OpenDRT` manifest is:

- `docs/plugins/dev/me-opendrt.local.json`

Available `ME_OpenDRT` local test variants:

- `docs/plugins/dev/me-opendrt.older.local.json`
  - Windows zip package for `1.1.0-rc1`
- `docs/plugins/dev/me-opendrt.1.2.11.local.json`
  - current local bundle directory for `1.2.11`
- `docs/plugins/dev/me-opendrt.bad-checksum.local.json`
  - same older zip with an intentionally wrong checksum
- `docs/plugins/dev/me-opendrt.rollback-fail.local.json`
  - current local bundle with a deliberate failure after backup to verify rollback

## Switching the active ME_OpenDRT manifest

From the manager root:

```powershell
.\tools\switch-me-opendrt-manifest.ps1 older
.\tools\switch-me-opendrt-manifest.ps1 current
.\tools\switch-me-opendrt-manifest.ps1 bad-checksum
.\tools\switch-me-opendrt-manifest.ps1 rollback-fail
```

Because the dev feed is now read from disk at refresh time, you can switch manifests and then just click:

- `Refresh Plugin Catalog`

No rebuild or app restart should be needed for manifest swaps anymore.

## Suggested test flow

### Update test

1. Set the older manifest:
   - `.\tools\switch-me-opendrt-manifest.ps1 older`
2. Refresh the catalog.
3. Install or reinstall `ME_OpenDRT`.
4. Set the current manifest:
   - `.\tools\switch-me-opendrt-manifest.ps1 current`
5. Refresh the catalog.
6. Confirm `Update available` appears.
7. Run the update and verify `1.2.11`.

### Checksum failure test

1. Set the bad checksum manifest:
   - `.\tools\switch-me-opendrt-manifest.ps1 bad-checksum`
2. Refresh the catalog.
3. Trigger install/update.
4. Confirm the visible alert and activity log show a checksum mismatch.

### Host-running block test

1. Launch a supported host.
2. Trigger install/update.
3. Confirm the visible alert mentions the running host process.

### Rollback test

1. Install the older package:
   - `.\tools\switch-me-opendrt-manifest.ps1 older`
2. Refresh and install `ME_OpenDRT`.
3. Switch to the rollback-fail manifest:
   - `.\tools\switch-me-opendrt-manifest.ps1 rollback-fail`
4. Refresh the catalog.
5. Trigger update.
6. Confirm the install fails visibly.
7. Refresh again and confirm the installed version is still the previous one rather than being lost or half-replaced.
