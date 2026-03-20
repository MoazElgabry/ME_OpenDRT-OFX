use crate::catalog;
use crate::models::{
    BundleInstallStamp, InstallRecord, ManagedInstallState, PlatformPackage, PluginOperationResult,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use fs_extra::dir::{copy as copy_dir, CopyOptions};
use plist::Value as PlistValue;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use sysinfo::System;
use tempfile::tempdir;
use walkdir::WalkDir;
use zip::ZipArchive;

pub fn updater_configured() -> bool {
    option_env!("MEPM_UPDATER_PUBKEY").is_some()
}

pub fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unsupported"
    }
}

pub fn current_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86_64" => "x86_64",
        other => other,
    }
}

pub fn install_key(plugin_id: &str, bundle_path: &Path) -> String {
    format!("{}::{}", plugin_id, bundle_path.display())
}

pub fn load_install_state() -> Result<ManagedInstallState> {
    let state_path = install_state_path()?;
    if !state_path.exists() {
        return Ok(ManagedInstallState::default());
    }
    let raw = fs::read_to_string(&state_path)
        .with_context(|| format!("Failed to read {}", state_path.display()))?;
    serde_json::from_str(&raw).context("Failed to parse install state JSON")
}

pub fn read_bundle_install_stamp(bundle_root: &Path) -> Result<Option<BundleInstallStamp>> {
    let stamp_path = bundle_stamp_path(bundle_root);
    if stamp_path.exists() {
        let raw = fs::read_to_string(&stamp_path)
            .with_context(|| format!("Failed to read {}", stamp_path.display()))?;
        let stamp: BundleInstallStamp =
            serde_json::from_str(&raw).with_context(|| format!("Failed to parse {}", stamp_path.display()))?;
        return Ok(Some(stamp));
    }

    let legacy_path = legacy_bundle_stamp_path(bundle_root);
    if !legacy_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&legacy_path)
        .with_context(|| format!("Failed to read {}", legacy_path.display()))?;
    let stamp: BundleInstallStamp =
        serde_json::from_str(&raw).with_context(|| format!("Failed to parse {}", legacy_path.display()))?;
    Ok(Some(stamp))
}

fn save_install_state(state: &ManagedInstallState) -> Result<()> {
    let state_path = install_state_path()?;
    if let Some(parent) = state_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state)?;
    fs::write(&state_path, raw).with_context(|| format!("Failed to write {}", state_path.display()))
}

fn install_state_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| anyhow!("Unable to resolve local application data directory"))?;
    let new_path = base
        .join("Moaz Elgabry Plugins")
        .join("install-state.json");
    let legacy_path = base
        .join("MoazElgabryPluginManager")
        .join("install-state.json");

    if !new_path.exists() && legacy_path.exists() {
        if let Some(parent) = new_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::copy(&legacy_path, &new_path).with_context(|| {
            format!(
                "Failed to migrate install state from {} to {}",
                legacy_path.display(),
                new_path.display()
            )
        })?;
    }

    Ok(new_path)
}

pub async fn apply_plugin_action(
    plugin_id: &str,
    action: &str,
    target_version: Option<&str>,
) -> Result<PluginOperationResult> {
    let resolved = catalog::resolve_plugin(plugin_id, target_version).await?;
    ensure_supported_package(&resolved.package)?;
    ensure_hosts_closed(&resolved.package.host_processes)?;
    let source_spec = parse_package_source_spec(&resolved.package.download_url);
    let installed_at = Utc::now().to_rfc3339();
    let staging_root = tempdir().context("Failed to create staging directory")?;
    let stage_bundle_root = staging_root.path().join(&resolved.package.bundle_name);

    let bundle_root = if resolved.package.package_type == "bundle-dir" {
        let local_bundle = resolve_local_source_path(&source_spec.source)?;
        verify_bundle(&local_bundle, &resolved.package)?;
        stage_bundle_directory(&local_bundle, &stage_bundle_root)?;
        write_bundle_install_stamp(
            &stage_bundle_root,
            plugin_id,
            &resolved.version,
            &resolved.package.bundle_identifier,
            &installed_at,
        )?;
        stage_bundle_root.clone()
    } else {
        let bytes = load_package_bytes(&source_spec.source).await?;
        verify_archive_hash(&bytes, &resolved.package.sha256)?;

        let extracted_root = staging_root.path().join("extract");
        extract_zip(&bytes, &extracted_root)?;

        let extracted_bundle = find_bundle_root(&extracted_root)?
            .ok_or_else(|| anyhow!("Archive did not contain an .ofx.bundle"))?;
        verify_bundle(&extracted_bundle, &resolved.package)?;
        fs::rename(&extracted_bundle, &stage_bundle_root).with_context(|| {
            format!(
                "Failed to stage extracted bundle from {} to {}",
                extracted_bundle.display(),
                stage_bundle_root.display()
            )
        })?;
        write_bundle_install_stamp(
            &stage_bundle_root,
            plugin_id,
            &resolved.version,
            &resolved.package.bundle_identifier,
            &installed_at,
        )?;
        stage_bundle_root.clone()
    };

    let install_root = PathBuf::from(&resolved.package.install_path);
    fs::create_dir_all(&install_root).ok();

    if cfg!(target_os = "windows") {
        privileged_install_windows(
            &bundle_root,
            &install_root,
            &resolved.package.bundle_name,
            source_spec.simulate_fail_after_backup,
        )?;
    } else if cfg!(target_os = "macos") {
        privileged_install_macos(
            &bundle_root,
            &install_root,
            &resolved.package.bundle_name,
            source_spec.simulate_fail_after_backup,
        )?;
    } else {
        bail!("Only macOS and Windows are supported in v1");
    }

    let target_bundle = install_root.join(&resolved.package.bundle_name);
    let mut state = load_install_state().unwrap_or_default();
    state.installs.insert(
        install_key(plugin_id, &target_bundle),
        InstallRecord {
            plugin_id: plugin_id.to_string(),
            bundle_path: target_bundle.display().to_string(),
            installed_version: resolved.version.clone(),
            bundle_identifier: resolved.package.bundle_identifier.clone(),
            installed_at: installed_at.clone(),
        },
    );
    save_install_state(&state)?;

    Ok(PluginOperationResult {
        plugin_id: plugin_id.to_string(),
        action: action.to_string(),
        status: "success".to_string(),
        message: format!(
            "{} {} installed to {}.",
            resolved.manifest.display_name,
            resolved.version,
            install_root.display()
        ),
    })
}

struct PackageSourceSpec {
    source: String,
    simulate_fail_after_backup: bool,
}

fn parse_package_source_spec(raw: &str) -> PackageSourceSpec {
    const FAIL_AFTER_BACKUP_PREFIX: &str = "mepm-test-fail-after-backup::";
    if let Some(source) = raw.strip_prefix(FAIL_AFTER_BACKUP_PREFIX) {
        return PackageSourceSpec {
            source: source.to_string(),
            simulate_fail_after_backup: true,
        };
    }

    PackageSourceSpec {
        source: raw.to_string(),
        simulate_fail_after_backup: false,
    }
}

fn ensure_supported_package(package: &PlatformPackage) -> Result<()> {
    if package.package_type != "zip" && package.package_type != "bundle-dir" {
        bail!("Only zip and bundle-dir plugin packages are supported in v1");
    }
    if package.package_type == "zip" && package.sha256.starts_with("REPLACE_") {
        bail!("The manifest for this plugin still contains a placeholder checksum.");
    }
    Ok(())
}

fn stage_bundle_directory(source_bundle: &Path, staged_bundle: &Path) -> Result<()> {
    if staged_bundle.exists() {
        fs::remove_dir_all(staged_bundle)
            .with_context(|| format!("Failed to remove {}", staged_bundle.display()))?;
    }
    let destination_parent = staged_bundle
        .parent()
        .ok_or_else(|| anyhow!("Staging path was missing a parent directory"))?;
    fs::create_dir_all(destination_parent)
        .with_context(|| format!("Failed to create {}", destination_parent.display()))?;

    let mut options = CopyOptions::new();
    options.copy_inside = false;
    options.overwrite = true;
    copy_dir(source_bundle, destination_parent, &options)
        .with_context(|| format!("Failed to stage {}", source_bundle.display()))?;
    Ok(())
}

fn write_bundle_install_stamp(
    bundle_root: &Path,
    plugin_id: &str,
    installed_version: &str,
    bundle_identifier: &str,
    installed_at: &str,
) -> Result<()> {
    let resources_dir = bundle_root.join("Contents").join("Resources");
    fs::create_dir_all(&resources_dir)
        .with_context(|| format!("Failed to create {}", resources_dir.display()))?;
    let stamp = BundleInstallStamp {
        plugin_id: plugin_id.to_string(),
        installed_version: installed_version.to_string(),
        bundle_identifier: bundle_identifier.to_string(),
        installed_at: installed_at.to_string(),
    };
    let raw = serde_json::to_string_pretty(&stamp)?;
    let stamp_path = bundle_stamp_path(bundle_root);
    fs::write(&stamp_path, raw).with_context(|| format!("Failed to write {}", stamp_path.display()))
}

async fn load_package_bytes(source: &str) -> Result<Vec<u8>> {
    if let Ok(local_path) = resolve_local_source_path(source) {
        return fs::read(&local_path)
            .with_context(|| format!("Failed to read {}", local_path.display()));
    }

    let client = Client::builder()
        .user_agent("MoazElgabryPlugins/0.1.0")
        .build()
        .context("Failed to create download client")?;
    let bytes = client
        .get(source)
        .send()
        .await
        .with_context(|| format!("Failed to download {source}"))?
        .error_for_status()
        .with_context(|| format!("Unexpected response while downloading {source}"))?
        .bytes()
        .await
        .context("Failed to read downloaded plugin archive")?;
    Ok(bytes.to_vec())
}

fn ensure_hosts_closed(host_processes: &[String]) -> Result<()> {
    let system = System::new_all();
    let mut running = Vec::new();

    for process in system.processes().values() {
        let process_name = process.name().to_string_lossy().to_lowercase();
        if host_processes
            .iter()
            .any(|candidate| process_name.contains(&candidate.to_lowercase()))
        {
            running.push(process.name().to_string_lossy().to_string());
        }
    }

    if running.is_empty() {
        return Ok(());
    }

    running.sort();
    running.dedup();
    bail!(
        "Close the running host applications before installing: {}",
        running.join(", ")
    )
}

fn verify_archive_hash(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    bail!(
        "Checksum mismatch. Expected {}, downloaded {}",
        expected,
        actual
    )
}

fn extract_zip(bytes: &[u8], destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    let reader = Cursor::new(bytes.to_vec());
    let mut archive = ZipArchive::new(reader).context("Downloaded archive was not a valid ZIP")?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let relative = match file.enclosed_name() {
            Some(path) => path.to_path_buf(),
            None => continue,
        };
        let output = destination.join(relative);
        if file.name().ends_with('/') {
            fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut writer = fs::File::create(&output)?;
        std::io::copy(&mut file, &mut writer)?;
    }

    Ok(())
}

fn find_bundle_root(root: &Path) -> Result<Option<PathBuf>> {
    for entry in WalkDir::new(root).min_depth(1).max_depth(6) {
        let entry = entry?;
        if entry.file_type().is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".ofx.bundle")
        {
            return Ok(Some(entry.into_path()));
        }
    }
    Ok(None)
}

fn verify_bundle(bundle_root: &Path, package: &PlatformPackage) -> Result<()> {
    let name = bundle_root
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("Bundle path was missing a file name"))?;
    if name != package.bundle_name {
        bail!(
            "Downloaded bundle name mismatch. Expected {}, found {}",
            package.bundle_name,
            name
        );
    }

    let plist_path = bundle_root.join("Contents").join("Info.plist");
    if !plist_path.exists() {
        if package.platform == "windows" {
            let expected_binary = package
                .bundle_name
                .strip_suffix(".ofx.bundle")
                .unwrap_or(&package.bundle_name);
            let win64_binary = bundle_root
                .join("Contents")
                .join("Win64")
                .join(format!("{expected_binary}.ofx"));
            if win64_binary.exists() {
                return Ok(());
            }
            bail!(
                "Windows bundle did not contain {}",
                win64_binary.display()
            );
        }
        bail!("Bundle did not contain Contents/Info.plist");
    }
    let plist = PlistValue::from_file(&plist_path)
        .with_context(|| format!("Failed to parse {}", plist_path.display()))?;
    let dictionary = plist
        .as_dictionary()
        .ok_or_else(|| anyhow!("Info.plist root was not a dictionary"))?;

    let bundle_identifier = dictionary
        .get("CFBundleIdentifier")
        .and_then(|value| value.as_string())
        .ok_or_else(|| anyhow!("CFBundleIdentifier was missing from Info.plist"))?;

    if bundle_identifier != package.bundle_identifier {
        bail!(
            "Bundle identifier mismatch. Expected {}, found {}",
            package.bundle_identifier,
            bundle_identifier
        );
    }

    Ok(())
}

fn privileged_install_windows(
    source_bundle: &Path,
    install_root: &Path,
    bundle_name: &str,
    simulate_fail_after_backup: bool,
) -> Result<()> {
    let token = format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_millis()
    );
    let script_dir = std::env::temp_dir().join("Moaz Elgabry Plugins");
    fs::create_dir_all(&script_dir)
        .with_context(|| format!("Failed to create {}", script_dir.display()))?;
    let script_path = script_dir.join(format!("install-plugin-{token}.ps1"));
    let log_path = script_dir.join(format!("install-plugin-{token}.log"));
    let script = format!(
        r#"$ErrorActionPreference = "Stop"
$SourceBundle = '{source}'
$InstallRoot = '{install_root}'
$BundleName = '{bundle_name}'
$LogPath = '{log}'
$SimulateFailureAfterBackup = {simulate_fail}
$target = Join-Path $InstallRoot $BundleName
$backup = Join-Path $InstallRoot ($BundleName + ".manager-backup")

function Write-InstallLog([string]$Message) {{
  $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
  Add-Content -Path $LogPath -Value "[$timestamp] $Message"
}}

try {{
  Set-Content -Path $LogPath -Value ""
  Write-InstallLog "Starting Windows plugin install"
  Write-InstallLog "SourceBundle=$SourceBundle"
  Write-InstallLog "InstallRoot=$InstallRoot"
  Write-InstallLog "BundleName=$BundleName"

  if (Test-Path $backup) {{ Remove-Item $backup -Recurse -Force }}
  if (!(Test-Path $InstallRoot)) {{ New-Item -ItemType Directory -Path $InstallRoot | Out-Null }}
  if (Test-Path $target) {{
    Write-InstallLog "Moving existing bundle to backup"
    Move-Item -LiteralPath $target -Destination $backup -Force
  }}
  if ($SimulateFailureAfterBackup -eq 1) {{
    Write-InstallLog "Simulating failure after backup for rollback test"
    throw "Simulated install failure after backup"
  }}
  Write-InstallLog "Copying new bundle"
  Copy-Item -LiteralPath $SourceBundle -Destination $InstallRoot -Recurse -Force
  if (Test-Path $backup) {{ Remove-Item $backup -Recurse -Force }}
  Write-InstallLog "Install completed successfully"
  exit 0
}}
catch {{
  Write-InstallLog ("ERROR: " + $_.Exception.Message)
  if ($_.ScriptStackTrace) {{
    Write-InstallLog ("STACK: " + $_.ScriptStackTrace)
  }}
  if (Test-Path $target) {{ Remove-Item $target -Recurse -Force }}
  if (Test-Path $backup) {{ Move-Item -LiteralPath $backup -Destination $target -Force }}
  throw
}}
"#,
        source = escape_ps(&source_bundle.display().to_string()),
        install_root = escape_ps(&install_root.display().to_string()),
        bundle_name = escape_ps(bundle_name),
        log = escape_ps(&log_path.display().to_string()),
        simulate_fail = if simulate_fail_after_backup { 1 } else { 0 }
    );
    fs::write(&script_path, script)
        .with_context(|| format!("Failed to write {}", script_path.display()))?;

    let outer_command = format!(
        "$ErrorActionPreference='Stop'; Set-Content -Path '{}' -Value ''; Add-Content -Path '{}' -Value '[outer] launching elevated installer'; try {{ $p = Start-Process -FilePath 'powershell.exe' -Verb RunAs -Wait -PassThru -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File','{}'); Add-Content -Path '{}' -Value ('[outer] elevated installer exit code: ' + $p.ExitCode); exit $p.ExitCode }} catch {{ Add-Content -Path '{}' -Value ('[outer] ERROR: ' + $_.Exception.Message); if ($_.ScriptStackTrace) {{ Add-Content -Path '{}' -Value ('[outer] STACK: ' + $_.ScriptStackTrace) }} exit 1 }}",
        escape_ps(&log_path.display().to_string()),
        escape_ps(&log_path.display().to_string()),
        escape_ps(&script_path.display().to_string()),
        escape_ps(&log_path.display().to_string()),
        escape_ps(&log_path.display().to_string()),
        escape_ps(&log_path.display().to_string())
    );

    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &outer_command])
        .status()
        .context("Failed to start elevated PowerShell installer")?;

    if status.success() {
        let _ = fs::remove_file(&script_path);
        let _ = fs::remove_file(&log_path);
        return Ok(());
    }

    let details = fs::read_to_string(&log_path).unwrap_or_default();
    let _ = fs::remove_file(&script_path);
    if details.trim().is_empty() {
        bail!(
            "Windows installation was cancelled or failed with exit code {:?}. Log file: {}",
            status.code(),
            log_path.display()
        );
    }

    bail!(
        "Windows installation failed with exit code {:?}. Log file: {}. Details: {}",
        status.code(),
        log_path.display(),
        details.trim()
    )
}

fn privileged_install_macos(
    source_bundle: &Path,
    install_root: &Path,
    bundle_name: &str,
    simulate_fail_after_backup: bool,
) -> Result<()> {
    let script_dir = tempdir().context("Failed to create temp directory for installer script")?;
    let script_path = script_dir.path().join("install-plugin.sh");
    let script = format!(r#"#!/bin/sh
set -e

SOURCE_BUNDLE="$1"
INSTALL_ROOT="$2"
BUNDLE_NAME="$3"
TARGET="$INSTALL_ROOT/$BUNDLE_NAME"
BACKUP="$INSTALL_ROOT/$BUNDLE_NAME.manager-backup"

cleanup_on_error() {{
  status=$?
  if [ "$status" -ne 0 ]; then
    rm -rf "$TARGET"
    if [ -d "$BACKUP" ]; then
      mv "$BACKUP" "$TARGET"
    fi
  fi
  exit "$status"
}}

trap cleanup_on_error EXIT

mkdir -p "$INSTALL_ROOT"
rm -rf "$BACKUP"
if [ -d "$TARGET" ]; then
  mv "$TARGET" "$BACKUP"
fi
if [ "{simulate_fail}" = "1" ]; then
  exit 91
fi
cp -R "$SOURCE_BUNDLE" "$INSTALL_ROOT/"
chmod -R 755 "$TARGET"
chown -R root:wheel "$TARGET"
xattr -dr com.apple.quarantine "$TARGET" || true
codesign --force --deep --sign - "$TARGET"
rm -rf "$BACKUP"
trap - EXIT
"#,
        simulate_fail = if simulate_fail_after_backup { "1" } else { "0" }
    );
    fs::write(&script_path, script)
        .with_context(|| format!("Failed to write {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)?;
    }

    let escaped_script = escape_osascript(&script_path.display().to_string());
    let escaped_source = escape_osascript(&source_bundle.display().to_string());
    let escaped_root = escape_osascript(&install_root.display().to_string());
    let escaped_bundle = escape_osascript(bundle_name);

    let status = Command::new("osascript")
        .args([
            "-e",
            &format!(r#"set scriptPath to "{}""#, escaped_script),
            "-e",
            &format!(r#"set sourceBundle to "{}""#, escaped_source),
            "-e",
            &format!(r#"set installRoot to "{}""#, escaped_root),
            "-e",
            &format!(r#"set bundleName to "{}""#, escaped_bundle),
            "-e",
            r#"do shell script quoted form of scriptPath & " " & quoted form of sourceBundle & " " & quoted form of installRoot & " " & quoted form of bundleName with administrator privileges"#,
        ])
        .status()
        .context("Failed to start elevated macOS installer")?;

    if status.success() {
        return Ok(());
    }

    bail!("macOS installation was cancelled or failed with exit code {:?}", status.code())
}

fn escape_ps(raw: &str) -> String {
    raw.replace('\'', "''")
}

fn escape_osascript(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

fn bundle_stamp_path(bundle_root: &Path) -> PathBuf {
    bundle_root
        .join("Contents")
        .join("Resources")
        .join("moaz-elgabry-plugins.install.json")
}

fn legacy_bundle_stamp_path(bundle_root: &Path) -> PathBuf {
    bundle_root
        .join("Contents")
        .join("Resources")
        .join("moazelgabry-plugin-manager.install.json")
}

fn resolve_local_source_path(raw: &str) -> Result<PathBuf> {
    let path = if let Some(stripped) = raw.strip_prefix("file:///") {
        PathBuf::from(stripped.replace('/', "\\"))
    } else {
        PathBuf::from(raw)
    };

    if path.exists() {
        return Ok(path);
    }

    bail!("Local package source was not found at {}", path.display())
}
