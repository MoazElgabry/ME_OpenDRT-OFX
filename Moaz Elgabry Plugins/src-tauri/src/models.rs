use anyhow::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardState {
    pub manager: ManagerSummary,
    pub catalog_source: String,
    pub plugins: Vec<PluginStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSummary {
    pub app_version: String,
    pub platform: String,
    pub arch: String,
    pub updater_configured: bool,
    pub catalog_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub plugin_id: String,
    pub display_name: String,
    pub latest_version: String,
    pub installed_version: Option<String>,
    pub install_path: String,
    pub bundle_name: String,
    pub installed: bool,
    pub managed_install: bool,
    pub needs_update: bool,
    pub status: String,
    pub release_notes_url: String,
    pub available_versions: Vec<VersionOption>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginOperationResult {
    pub plugin_id: String,
    pub action: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCatalogIndex {
    pub generated_at: String,
    pub plugins: Vec<CatalogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub plugin_id: String,
    pub display_name: String,
    pub manifest_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub plugin_id: String,
    pub display_name: String,
    pub version: String,
    pub release_date: String,
    pub release_notes_url: String,
    pub platforms: Vec<PlatformPackage>,
    #[serde(default)]
    pub available_versions: Vec<PluginRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRelease {
    pub version: String,
    pub release_date: String,
    pub release_notes_url: String,
    pub platforms: Vec<PlatformPackage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionOption {
    pub version: String,
    pub label: String,
    pub release_date: String,
    pub is_current_latest: bool,
    pub is_installed: bool,
    pub action_label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformPackage {
    pub platform: String,
    pub arch: String,
    pub download_url: String,
    pub sha256: String,
    pub package_type: String,
    pub bundle_name: String,
    pub bundle_identifier: String,
    pub install_path: String,
    pub min_manager_version: String,
    pub host_processes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedInstallState {
    pub installs: BTreeMap<String, InstallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRecord {
    pub plugin_id: String,
    pub bundle_path: String,
    pub installed_version: String,
    pub bundle_identifier: String,
    pub installed_at: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub manifest: PluginManifest,
    pub version: String,
    pub release_notes_url: String,
    pub package: PlatformPackage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleInstallStamp {
    pub plugin_id: String,
    pub installed_version: String,
    pub bundle_identifier: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiError {
    pub code: String,
    pub summary: String,
    pub details: String,
}

impl UiError {
    pub fn from_error(operation: &str, error: &Error) -> Self {
        let details = error
            .chain()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .join("\nCaused by: ");
        let message = error.to_string();
        let (code, summary) = classify_error(operation, &message, &details);

        Self {
            code: code.to_string(),
            summary: summary.to_string(),
            details,
        }
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            format!(
                r#"{{"code":"unknown","summary":"{}","details":"{}"}}"#,
                escape_json_string(&self.summary),
                escape_json_string(&self.details)
            )
        })
    }
}

fn classify_error<'a>(operation: &'a str, message: &'a str, details: &'a str) -> (&'a str, &'a str) {
    if details.contains("Checksum mismatch") {
        return (
            "checksum_mismatch",
            "The downloaded plugin package did not match the expected checksum.",
        );
    }

    if details.contains("Close the running host applications before installing") {
        return (
            "host_running",
            "Close any running supported host apps before installing or updating this plugin.",
        );
    }

    if details.contains("Simulated install failure after backup") {
        return (
            "rollback_restored",
            "Installation failed after backup, and the previous plugin version was restored.",
        );
    }

    if details.contains("placeholder checksum") {
        return (
            "placeholder_checksum",
            "This plugin manifest still has a placeholder checksum, so the install was blocked.",
        );
    }

    if details.contains("Local package source was not found") {
        return (
            "package_missing",
            "The plugin package source could not be found on disk.",
        );
    }

    if details.contains("Failed to download") || details.contains("Unexpected response while downloading") {
        return (
            "download_failed",
            "The plugin package could not be downloaded.",
        );
    }

    if details.contains("Plugin version `") && details.contains("was not found in the manifest") {
        return (
            "version_missing",
            "The selected plugin version is no longer available in the manifest.",
        );
    }

    if details.contains("No supported package found for") {
        return (
            "platform_unsupported",
            "This plugin release does not include a package for your current platform.",
        );
    }

    if details.contains("cancelled or failed") {
        return (
            "install_failed",
            "The elevated installer did not complete successfully.",
        );
    }

    if operation == "dashboard" {
        return (
            "catalog_refresh_failed",
            "Couldn't refresh the plugin catalog right now.",
        );
    }

    if message.is_empty() {
        return ("unknown", "The operation failed for an unknown reason.");
    }

    ("operation_failed", "The plugin operation could not be completed.")
}

fn escape_json_string(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
