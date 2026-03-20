param(
  [Parameter(Mandatory = $true)]
  [ValidateSet("older", "current", "bad-checksum", "rollback-fail")]
  [string]$Mode
)

$root = Split-Path -Parent $PSScriptRoot
$devDir = Join-Path $root "docs\plugins\dev"
$target = Join-Path $devDir "me-opendrt.local.json"

$source = switch ($Mode) {
  "older" { Join-Path $devDir "me-opendrt.older.local.json" }
  "current" { Join-Path $devDir "me-opendrt.1.2.11.local.json" }
  "bad-checksum" { Join-Path $devDir "me-opendrt.bad-checksum.local.json" }
  "rollback-fail" { Join-Path $devDir "me-opendrt.rollback-fail.local.json" }
}

Copy-Item -LiteralPath $source -Destination $target -Force
Write-Host "Active ME_OpenDRT dev manifest set to: $Mode"
Write-Host "Target file: $target"
