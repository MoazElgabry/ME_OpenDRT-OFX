use crate::installer;
use crate::models::{
    CatalogEntry, DashboardState, ManagerSummary, PlatformPackage, PluginCatalogIndex, PluginManifest,
    PluginRelease, PluginStatus, ResolvedPlugin, VersionOption,
};
use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use semver::Version;
use serde::de::DeserializeOwned;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_CATALOG_URL: &str =
    "https://moazelgabry.github.io/MoazElgabryPluginManager/plugins/index.json";

const EMBEDDED_INDEX: &str = include_str!("../../docs/plugins/index.json");
const EMBEDDED_CHROMASPACE: &str = include_str!("../../docs/plugins/chromaspace/stable.json");
const EMBEDDED_ME_OPENDRT: &str = include_str!("../../docs/plugins/me-opendrt/stable.json");

#[derive(Debug, Clone)]
pub struct CatalogBundle {
    pub source: String,
    pub source_label: String,
    pub entries: Vec<CatalogEntry>,
    pub manifests: HashMap<String, PluginManifest>,
}

pub async fn build_dashboard_state() -> Result<DashboardState> {
    let bundle = load_catalog_bundle().await?;
    let install_state = installer::load_install_state()?;
    let manager = ManagerSummary {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: installer::current_platform().to_string(),
        arch: installer::current_arch().to_string(),
        updater_configured: installer::updater_configured(),
        catalog_url: bundle.source_label.clone(),
    };

    let plugins = bundle
        .entries
        .iter()
        .filter_map(|entry| {
            let manifest = bundle.manifests.get(&entry.plugin_id)?;
            let package = select_package(&manifest.platforms).ok()?;
            Some(build_plugin_status(entry, manifest, package, &install_state))
        })
        .collect::<Vec<_>>();

    Ok(DashboardState {
        manager,
        catalog_source: bundle.source,
        plugins,
    })
}

pub async fn resolve_plugin(plugin_id: &str, requested_version: Option<&str>) -> Result<ResolvedPlugin> {
    let bundle = load_catalog_bundle().await?;
    let manifest = bundle
        .manifests
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| anyhow!("Plugin manifest not found for `{plugin_id}`"))?;
    let release = resolve_release(&manifest, requested_version)?;
    let package = select_package(&release.platforms)?;
    Ok(ResolvedPlugin {
        manifest,
        version: release.version,
        release_notes_url: release.release_notes_url,
        package,
    })
}

fn build_plugin_status(
    entry: &CatalogEntry,
    manifest: &PluginManifest,
    package: PlatformPackage,
    install_state: &crate::models::ManagedInstallState,
) -> PluginStatus {
    let target_bundle = PathBuf::from(&package.install_path).join(&package.bundle_name);
    let installed = target_bundle.exists();
    let install_key = installer::install_key(&entry.plugin_id, &target_bundle);
    let record = install_state.installs.get(&install_key);
    let stamp = installer::read_bundle_install_stamp(&target_bundle).ok().flatten();
    let installed_version = stamp
        .as_ref()
        .map(|item| item.installed_version.clone())
        .or_else(|| record.map(|item| item.installed_version.clone()));
    let managed_install = stamp.is_some() || record.is_some();
    let needs_update = installed
        && installed_version
            .as_ref()
            .map(|current| version_cmp(current, &manifest.version) == Ordering::Less)
            .unwrap_or(true);
    let available_versions = version_options(manifest, installed_version.as_deref());

    let status = if !installed {
        "Ready to install".to_string()
    } else if needs_update {
        "Update available".to_string()
    } else if managed_install {
        "Up to date".to_string()
    } else {
        "Unmanaged install".to_string()
    };

    PluginStatus {
        plugin_id: entry.plugin_id.clone(),
        display_name: manifest.display_name.clone(),
        latest_version: manifest.version.clone(),
        installed_version,
        install_path: package.install_path.clone(),
        bundle_name: package.bundle_name.clone(),
        installed,
        managed_install,
        needs_update,
        status,
        release_notes_url: manifest.release_notes_url.clone(),
        available_versions,
    }
}

fn version_cmp(left: &str, right: &str) -> Ordering {
    match (Version::parse(left), Version::parse(right)) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

pub fn select_package(packages: &[PlatformPackage]) -> Result<PlatformPackage> {
    let platform = installer::current_platform();
    let arch = installer::current_arch();

    let mut exact = None;
    let mut universal = None;

    for package in packages {
        if package.platform != platform {
            continue;
        }
        if package.arch == arch {
            exact = Some(package.clone());
            break;
        }
        if package.arch == "universal" {
            universal = Some(package.clone());
        }
    }

    exact
        .or(universal)
        .ok_or_else(|| anyhow!("No supported package found for {} / {}", platform, arch))
}

fn version_options(manifest: &PluginManifest, installed_version: Option<&str>) -> Vec<VersionOption> {
    let mut releases = collect_releases(manifest);
    releases.retain(|release| select_package(&release.platforms).is_ok());
    releases.sort_by(|left, right| version_cmp(&right.version, &left.version));

    let mut options = Vec::new();
    for release in releases {
        let version = release.version.clone();
        let is_current_latest = version == manifest.version;
        let is_installed = installed_version == Some(version.as_str());
        let label = if is_current_latest && is_installed {
            format!("{} (Latest installed)", version)
        } else if is_current_latest {
            format!("{} (Latest)", version)
        } else if is_installed {
            format!("{} (Installed)", version)
        } else {
            version.clone()
        };
        let action_label = match installed_version {
            Some(current) if current == version => "Reinstall this version".to_string(),
            Some(current) => match version_cmp(&version, current) {
                Ordering::Greater => "Install selected upgrade".to_string(),
                Ordering::Less => "Roll back to selected".to_string(),
                Ordering::Equal => "Install selected".to_string(),
            },
            None => "Install selected".to_string(),
        };
        options.push(VersionOption {
            version,
            label,
            release_date: release.release_date,
            is_current_latest,
            is_installed,
            action_label,
        });
    }
    options
}

fn collect_releases(manifest: &PluginManifest) -> Vec<PluginRelease> {
    let mut releases = Vec::with_capacity(1 + manifest.available_versions.len());
    releases.push(PluginRelease {
        version: manifest.version.clone(),
        release_date: manifest.release_date.clone(),
        release_notes_url: manifest.release_notes_url.clone(),
        platforms: manifest.platforms.clone(),
    });

    for release in &manifest.available_versions {
        if releases.iter().any(|item| item.version == release.version) {
            continue;
        }
        releases.push(release.clone());
    }
    releases
}

fn resolve_release(manifest: &PluginManifest, requested_version: Option<&str>) -> Result<PluginRelease> {
    let releases = collect_releases(manifest);
    if let Some(version) = requested_version {
        return releases
            .into_iter()
            .find(|release| release.version == version)
            .ok_or_else(|| anyhow!("Plugin version `{version}` was not found in the manifest"));
    }

    Ok(releases
        .into_iter()
        .find(|release| release.version == manifest.version)
        .ok_or_else(|| anyhow!("Latest plugin version was not found in the manifest"))?)
}

async fn load_catalog_bundle() -> Result<CatalogBundle> {
    if cfg!(debug_assertions) {
        if let Some(bundle) = load_local_dev_catalog()? {
            return Ok(bundle);
        }
    }

    let client = Client::builder()
        .user_agent("MoazElgabryPlugins/0.1.0")
        .build()
        .context("Failed to create HTTP client")?;

    let (index, source) = match fetch_json::<PluginCatalogIndex>(&client, DEFAULT_CATALOG_URL).await {
        Ok(index) => (index, "remote".to_string()),
        Err(_) => (serde_json::from_str(EMBEDDED_INDEX)?, "embedded".to_string()),
    };

    let mut manifests = HashMap::new();
    for entry in &index.plugins {
        let manifest = match fetch_json::<PluginManifest>(&client, &entry.manifest_url).await {
            Ok(value) => value,
            Err(_) => embedded_manifest(&entry.plugin_id)?,
        };
        manifests.insert(entry.plugin_id.clone(), manifest);
    }

    Ok(CatalogBundle {
        source,
        source_label: DEFAULT_CATALOG_URL.to_string(),
        entries: index.plugins,
        manifests,
    })
}

fn load_local_dev_catalog() -> Result<Option<CatalogBundle>> {
    let manager_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow!("Unable to resolve manager root"))?;
    let dev_index_path = manager_root.join("docs").join("plugins").join("dev").join("index.json");
    if !dev_index_path.exists() {
        return Ok(None);
    }

    let raw_index = fs::read_to_string(&dev_index_path)
        .with_context(|| format!("Failed to read {}", dev_index_path.display()))?;
    let expanded_index = expand_tokens(&raw_index)?;
    let index: PluginCatalogIndex = serde_json::from_str(&expanded_index)
        .with_context(|| format!("Failed to parse {}", dev_index_path.display()))?;
    let mut manifests = HashMap::new();
    for entry in &index.plugins {
        let manifest_path = resolve_manifest_path(&entry.manifest_url, manager_root)?;
        let raw_manifest = fs::read_to_string(&manifest_path)
            .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
        let expanded = expand_tokens(&raw_manifest)?;
        let manifest: PluginManifest = serde_json::from_str(&expanded)
            .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;
        manifests.insert(entry.plugin_id.clone(), manifest);
    }

    Ok(Some(CatalogBundle {
        source: "local-dev".to_string(),
        source_label: dev_index_path.display().to_string(),
        entries: index.plugins,
        manifests,
    }))
}

async fn fetch_json<T>(client: &Client, url: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    if let Some(local_path) = resolve_local_path(url) {
        let raw = fs::read_to_string(&local_path)
            .with_context(|| format!("Failed to read {}", local_path.display()))?;
        return serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse JSON from {}", local_path.display()));
    }

    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("Unexpected response while fetching {url}"))?;

    response
        .json::<T>()
        .await
        .with_context(|| format!("Failed to parse JSON from {url}"))
}

fn embedded_manifest(plugin_id: &str) -> Result<PluginManifest> {
    let raw = match plugin_id {
        "chromaspace" => EMBEDDED_CHROMASPACE,
        "me-opendrt" => EMBEDDED_ME_OPENDRT,
        _ => return Err(anyhow!("No embedded manifest available for `{plugin_id}`")),
    };
    serde_json::from_str(raw).with_context(|| format!("Failed to parse embedded manifest for {plugin_id}"))
}

fn expand_tokens(raw: &str) -> Result<String> {
    let manager_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow!("Unable to resolve manager root"))?;
    let me_ofx_root = manager_root
        .parent()
        .ok_or_else(|| anyhow!("Unable to resolve ME_OFX root"))?;
    let git_root = me_ofx_root
        .parent()
        .ok_or_else(|| anyhow!("Unable to resolve GitHub root"))?;
    let ofx_workshop_root = git_root.join("OFX-Workshop");

    let mut expanded = raw.to_string();
    let mappings = [
        ("${MEPM_MANAGER_ROOT}", manager_root.display().to_string()),
        ("${ME_OFX_ROOT}", me_ofx_root.display().to_string()),
        ("${OFX_WORKSHOP_ROOT}", ofx_workshop_root.display().to_string()),
    ];
    for (token, replacement) in mappings {
        expanded = expanded.replace(token, &replacement.replace('\\', "\\\\"));
    }
    Ok(expanded)
}

fn resolve_manifest_path(raw: &str, manager_root: &Path) -> Result<PathBuf> {
    let expanded = expand_tokens(raw)?;
    if let Some(local_path) = resolve_local_path(&expanded) {
        return Ok(local_path);
    }

    Ok(manager_root.join(expanded.replace('/', "\\")))
}

fn resolve_local_path(raw: &str) -> Option<PathBuf> {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return None;
    }
    if let Some(stripped) = raw.strip_prefix("file:///") {
        return Some(PathBuf::from(stripped.replace('/', "\\")));
    }
    if raw.contains(':') || raw.starts_with('\\') || raw.starts_with('/') {
        return Some(PathBuf::from(raw));
    }
    None
}
