//! Helm on kind clusters through the app-managed helm binary. Repos and the
//! cache live under the app's tools dir, never the user's ~/.config/helm.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::k8s_tools::{helm_scratch_dir, run_tool, run_tool_with, Tool};
use crate::state::AppState;

const QUICK: Duration = Duration::from_secs(60);
const LONG: Duration = Duration::from_secs(660);

fn check_name(kind: &str, n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 53
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !n.starts_with('-')
        && !n.ends_with('-');
    if ok { Ok(()) } else { Err(format!("invalid {kind} '{n}'")) }
}

/// Chart reference: `repo/chart`, `oci://…`, `https://…tgz`. Never a flag.
pub(crate) fn check_chart(c: &str) -> Result<(), String> {
    let ok = !c.is_empty()
        && !c.starts_with('-')
        && c.len() <= 512
        && !c.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        && (c.starts_with("oci://") || c.starts_with("https://") || c.starts_with("http://") || c.contains('/'));
    if ok { Ok(()) } else { Err(format!("invalid chart reference '{c}' (use repo/chart or oci://…)")) }
}

fn check_version(v: &str) -> Result<(), String> {
    let ok = !v.starts_with('-')
        && v.len() <= 64
        && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+~^<>=*, ".contains(c));
    if ok { Ok(()) } else { Err(format!("invalid version '{v}'")) }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// A private values file that is removed when dropped.
struct ValuesFile(PathBuf);
impl ValuesFile {
    fn new(yaml: &str) -> Result<Self, String> {
        // Reject non-YAML early with a readable error.
        yaml_rust2::YamlLoader::load_from_str(yaml).map_err(|e| format!("values are not valid YAML: {e}"))?;
        let dir = helm_scratch_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let path = dir.join(format!("values-{}-{nanos}.yaml", std::process::id()));
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| e.to_string())?;
            f.write_all(yaml.as_bytes()).map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&path, yaml).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for ValuesFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn parse_json(out: &str) -> Result<serde_json::Value, String> {
    let t = out.trim();
    if t.is_empty() {
        return Ok(serde_json::Value::Array(vec![]));
    }
    serde_json::from_str(t).map_err(|e| format!("unexpected helm output: {e}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmRelease {
    pub name: String,
    pub namespace: String,
    pub revision: String,
    pub updated: String,
    pub status: String,
    pub chart: String,
    pub app_version: String,
}

#[tauri::command]
pub async fn helm_list_releases(state: State<'_, Arc<AppState>>, cluster: String) -> Result<Vec<HelmRelease>, String> {
    helm_list_releases_core(&state, &cluster).await
}

pub(crate) async fn helm_list_releases_core(state: &AppState, cluster: &str) -> Result<Vec<HelmRelease>, String> {
    let out = run_tool(state, Tool::Helm, cluster, &s(&["list", "-A", "-a", "-o", "json"]), QUICK).await?;
    serde_json::from_value(parse_json(&out)?).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelmReleaseDetail {
    pub values: String,
    pub all_values: String,
    pub manifest: String,
    pub notes: String,
    pub history: serde_json::Value,
}

#[tauri::command]
pub async fn helm_release_detail(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    name: String,
) -> Result<HelmReleaseDetail, String> {
    helm_release_detail_core(&state, &cluster, &namespace, &name).await
}

pub(crate) async fn helm_release_detail_core(
    state: &AppState,
    cluster: &str,
    namespace: &str,
    name: &str,
) -> Result<HelmReleaseDetail, String> {
    check_name("namespace", namespace)?;
    check_name("release name", name)?;
    let get = |what: &'static str, extra: &'static [&'static str]| {
        let mut a = s(&["get", what, name, "-n", namespace]);
        a.extend(s(extra));
        async move { run_tool(state, Tool::Helm, cluster, &a, QUICK).await }
    };
    let (values, all_values, manifest, notes) = tokio::join!(
        get("values", &["-o", "yaml"]),
        get("values", &["-a", "-o", "yaml"]),
        get("manifest", &[]),
        get("notes", &[]),
    );
    let history = run_tool(state, Tool::Helm, cluster, &s(&["history", name, "-n", namespace, "-o", "json"]), QUICK).await?;
    Ok(HelmReleaseDetail {
        values: values?,
        all_values: all_values?,
        manifest: manifest?,
        notes: notes.unwrap_or_default(),
        history: parse_json(&history)?,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelmInstallRequest {
    pub cluster: String,
    pub namespace: String,
    pub name: String,
    pub chart: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub values: Option<String>,
    /// Upgrade an existing release (`helm upgrade --install`) instead of a fresh install.
    #[serde(default)]
    pub upgrade: bool,
    /// Keep the release's current values and merge `values` on top.
    #[serde(default)]
    pub reuse_values: bool,
}

pub(crate) fn install_args(req: &HelmInstallRequest, values_file: Option<&str>) -> Result<Vec<String>, String> {
    check_name("namespace", &req.namespace)?;
    check_name("release name", &req.name)?;
    check_chart(&req.chart)?;
    let mut a = if req.upgrade {
        s(&["upgrade", "--install", &req.name, &req.chart])
    } else {
        s(&["install", &req.name, &req.chart])
    };
    a.extend(s(&["-n", &req.namespace, "--create-namespace", "--wait", "--timeout", "10m", "-o", "json"]));
    if let Some(v) = req.version.as_deref().filter(|v| !v.trim().is_empty()) {
        check_version(v.trim())?;
        a.extend(s(&["--version", v.trim()]));
    }
    if req.upgrade && req.reuse_values {
        a.push("--reuse-values".into());
    }
    if let Some(f) = values_file {
        a.extend(s(&["-f", f]));
    }
    Ok(a)
}

#[tauri::command]
pub async fn helm_install(state: State<'_, Arc<AppState>>, req: HelmInstallRequest) -> Result<String, String> {
    helm_install_core(&state, req).await
}

pub(crate) async fn helm_install_core(state: &AppState, req: HelmInstallRequest) -> Result<String, String> {
    super::kind::check_cluster_name(&req.cluster)?;
    let vf = match req.values.as_deref().filter(|v| !v.trim().is_empty()) {
        Some(v) => Some(ValuesFile::new(v)?),
        None => None,
    };
    let args = install_args(&req, vf.as_ref().map(|f| f.0.to_str().unwrap_or_default()))?;
    let out = run_tool(state, Tool::Helm, &req.cluster, &args, LONG).await?;
    let v = parse_json(&out)?;
    Ok(format!(
        "{} {} revision {} ({})",
        req.name,
        if req.upgrade { "upgraded" } else { "installed" },
        v["version"],
        v["info"]["status"].as_str().unwrap_or("unknown")
    ))
}

#[tauri::command]
pub async fn helm_rollback(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    name: String,
    revision: u32,
) -> Result<(), String> {
    helm_rollback_core(&state, &cluster, &namespace, &name, revision).await
}

pub(crate) async fn helm_rollback_core(
    state: &AppState,
    cluster: &str,
    namespace: &str,
    name: &str,
    revision: u32,
) -> Result<(), String> {
    check_name("namespace", namespace)?;
    check_name("release name", name)?;
    let rev = revision.to_string();
    run_tool(state, Tool::Helm, cluster, &s(&["rollback", name, &rev, "-n", namespace, "--wait", "--timeout", "10m"]), LONG)
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn helm_uninstall(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    name: String,
) -> Result<(), String> {
    helm_uninstall_core(&state, &cluster, &namespace, &name).await
}

pub(crate) async fn helm_uninstall_core(state: &AppState, cluster: &str, namespace: &str, name: &str) -> Result<(), String> {
    check_name("namespace", namespace)?;
    check_name("release name", name)?;
    run_tool(state, Tool::Helm, cluster, &s(&["uninstall", name, "-n", namespace, "--wait", "--timeout", "5m"]), LONG)
        .await
        .map(|_| ())
}

// ------------------------------------------------------------------ repos

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmRepo {
    pub name: String,
    pub url: String,
}

#[tauri::command]
pub async fn helm_repo_list() -> Result<Vec<HelmRepo>, String> {
    match run_tool_with(Tool::Helm, None, &s(&["repo", "list", "-o", "json"]), QUICK).await {
        Ok(out) => serde_json::from_value(parse_json(&out)?).map_err(|e| e.to_string()),
        // `helm repo list` fails when no repositories exist yet.
        Err(e) if e.contains("no repositories") => Ok(vec![]),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn helm_repo_add(name: String, url: String) -> Result<(), String> {
    helm_repo_add_core(&name, &url).await
}

pub(crate) async fn helm_repo_add_core(name: &str, url: &str) -> Result<(), String> {
    check_name("repository name", name)?;
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.chars().any(|c| c.is_whitespace()) {
        return Err("Repository URL must be http(s)".into());
    }
    run_tool_with(Tool::Helm, None, &s(&["repo", "add", name, url, "--force-update"]), QUICK).await?;
    Ok(())
}

#[tauri::command]
pub async fn helm_repo_remove(name: String) -> Result<(), String> {
    check_name("repository name", &name)?;
    run_tool_with(Tool::Helm, None, &s(&["repo", "remove", &name]), QUICK).await.map(|_| ())
}

#[tauri::command]
pub async fn helm_repo_update() -> Result<String, String> {
    run_tool_with(Tool::Helm, None, &s(&["repo", "update"]), Duration::from_secs(180)).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmChart {
    pub name: String,
    pub version: String,
    pub app_version: String,
    pub description: String,
}

#[tauri::command]
pub async fn helm_search(query: String, versions: Option<bool>) -> Result<Vec<HelmChart>, String> {
    helm_search_core(&query, versions.unwrap_or(false)).await
}

pub(crate) async fn helm_search_core(query: &str, versions: bool) -> Result<Vec<HelmChart>, String> {
    let q = query.trim();
    if q.starts_with('-') || q.len() > 200 {
        return Err("invalid search query".into());
    }
    let mut a = s(&["search", "repo"]);
    if !q.is_empty() {
        a.push(q.to_string());
    }
    if versions {
        a.push("--versions".into());
    }
    a.extend(s(&["-o", "json"]));
    let out = run_tool_with(Tool::Helm, None, &a, QUICK).await?;
    serde_json::from_value(parse_json(&out)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn helm_show_values(chart: String, version: Option<String>) -> Result<String, String> {
    check_chart(&chart)?;
    let mut a = s(&["show", "values", &chart]);
    if let Some(v) = version.as_deref().filter(|v| !v.trim().is_empty()) {
        check_version(v.trim())?;
        a.extend(s(&["--version", v.trim()]));
    }
    run_tool_with(Tool::Helm, None, &a, Duration::from_secs(120)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> HelmInstallRequest {
        HelmInstallRequest {
            cluster: "dev".into(),
            namespace: "web".into(),
            name: "nginx".into(),
            chart: "bitnami/nginx".into(),
            version: Some("18.1.0".into()),
            values: None,
            upgrade: false,
            reuse_values: true,
        }
    }

    #[test]
    fn install_args_are_built_safely() {
        let a = install_args(&req(), Some("/x/v.yaml")).unwrap();
        assert_eq!(&a[..3], &["install", "nginx", "bitnami/nginx"]);
        assert!(a.windows(2).any(|w| w == ["--version", "18.1.0"]));
        assert!(a.windows(2).any(|w| w == ["-f", "/x/v.yaml"]));
        assert!(!a.contains(&"--reuse-values".to_string()), "reuse only applies to upgrades");

        let mut r = req();
        r.upgrade = true;
        let a = install_args(&r, None).unwrap();
        assert_eq!(&a[..4], &["upgrade", "--install", "nginx", "bitnami/nginx"]);
        assert!(a.contains(&"--reuse-values".to_string()));
    }

    #[test]
    fn injection_is_rejected() {
        let mut r = req();
        r.chart = "--post-renderer=/bin/sh".into();
        assert!(install_args(&r, None).is_err());
        let mut r = req();
        r.version = Some("--set=x".into());
        assert!(install_args(&r, None).is_err());
        let mut r = req();
        r.name = "Bad Name".into();
        assert!(install_args(&r, None).is_err());
        assert!(check_chart("oci://registry-1.docker.io/bitnamicharts/nginx").is_ok());
        assert!(check_chart("nginx").is_err());
        assert!(check_version(">=1.2.0 <2").is_ok());
    }

    #[test]
    fn values_file_is_validated_and_cleaned_up() {
        assert!(ValuesFile::new("a: [").is_err());
        let p = {
            let f = ValuesFile::new("replicaCount: 2\n").unwrap();
            assert!(f.0.is_file());
            f.0.clone()
        };
        assert!(!p.exists());
    }
}
