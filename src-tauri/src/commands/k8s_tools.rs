//! App-managed `kubectl` and `helm` binaries, pinned and checksum-verified
//! like the kind binary. They run on the host against the app's isolated
//! kubeconfig (port-forwards must bind host ports; Helm needs a real client).
//! Nothing is put on `PATH`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

pub(crate) const KUBECTL_VERSION: &str = "v1.37.2";
pub(crate) const HELM_VERSION: &str = "v3.22.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Kubectl,
    Helm,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Kubectl => "kubectl",
            Tool::Helm => "helm",
        }
    }
    fn version(self) -> &'static str {
        match self {
            Tool::Kubectl => KUBECTL_VERSION,
            Tool::Helm => HELM_VERSION,
        }
    }
}

fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Download URL and pinned sha256 of the release artifact.
pub(crate) fn artifact(tool: Tool, arch: &str) -> Option<(String, &'static str)> {
    if std::env::consts::OS != "linux" && !cfg!(test) {
        return None;
    }
    let sha = match (tool, arch) {
        (Tool::Kubectl, "amd64") => "c04487523e7aa7d1493f7808174c899db93d7a9cf57531420cd76b9bd415f8f4",
        (Tool::Kubectl, "arm64") => "fcc16f63186659d9722014b7e02d5fdd2d4e669cb8c0ab66365d7d3b8ba03509",
        (Tool::Helm, "amd64") => "896cf63684a42a723d61c980369d6e588330927deb13676ca0d85006dd6da0ea",
        (Tool::Helm, "arm64") => "4c29e286b279d170fa048ddfc3e4e83dcd069ca9c52102766ba672d7149e694d",
        _ => return None,
    };
    let url = match tool {
        Tool::Kubectl => format!("https://dl.k8s.io/release/{KUBECTL_VERSION}/bin/linux/{arch}/kubectl"),
        Tool::Helm => format!("https://get.helm.sh/helm-{HELM_VERSION}-linux-{arch}.tar.gz"),
    };
    Some((url, sha))
}

fn tools_root() -> PathBuf {
    let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    data.join(crate::migrate::APP_DIR).join("versions").join("k8s-tools")
}

pub(crate) fn tool_path(tool: Tool) -> PathBuf {
    tools_root().join(tool.version()).join(tool.name())
}

static INSTALL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Path of the tool, downloading and verifying it on first use.
pub(crate) async fn ensure_tool(tool: Tool) -> Result<PathBuf, String> {
    let path = tool_path(tool);
    if path.is_file() {
        return Ok(path);
    }
    let _g = INSTALL.lock().await;
    if path.is_file() {
        return Ok(path);
    }
    let arch = arch().ok_or("this CPU architecture is not supported")?;
    let (url, sha) = artifact(tool, arch).ok_or("this platform is not supported")?;
    let dir = path.parent().ok_or("bad tool path")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let staging = dir.join(format!(".{}.download", tool.name()));
    crate::docker::engine::download(&url, &staging, |_, _| {})
        .await
        .map_err(|e| format!("{e:#}"))?;
    let got = crate::docker::engine::sha256_file(&staging).map_err(|e| e.to_string())?;
    if got != sha {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("{} checksum mismatch (got {got})", tool.name()));
    }
    match tool {
        Tool::Kubectl => std::fs::rename(&staging, &path).map_err(|e| e.to_string())?,
        Tool::Helm => {
            let s = staging.clone();
            let target = path.clone();
            tokio::task::spawn_blocking(move || extract_helm(&s, &target))
                .await
                .map_err(|e| e.to_string())??;
            let _ = std::fs::remove_file(&staging);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// Pulls `linux-<arch>/helm` out of the release tarball.
fn extract_helm(archive: &Path, target: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path().map_err(|e| e.to_string())?.to_path_buf();
        if p.file_name().and_then(|n| n.to_str()) == Some("helm") && p.components().count() == 2 {
            let tmp = target.with_extension("tmp");
            let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            return std::fs::rename(&tmp, target).map_err(|e| e.to_string());
        }
    }
    Err("helm binary not found in the release archive".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub version: String,
    pub installed: bool,
    pub path: String,
}

#[tauri::command]
pub fn k8s_tools_status() -> Vec<ToolStatus> {
    [Tool::Kubectl, Tool::Helm]
        .into_iter()
        .map(|t| ToolStatus {
            name: t.name().into(),
            version: t.version().into(),
            installed: tool_path(t).is_file(),
            path: tool_path(t).display().to_string(),
        })
        .collect()
}

#[tauri::command]
pub async fn install_k8s_tools() -> Result<Vec<ToolStatus>, String> {
    ensure_tool(Tool::Kubectl).await?;
    ensure_tool(Tool::Helm).await?;
    Ok(k8s_tools_status())
}

/// A per-cluster kubeconfig the host tools use: `kind get kubeconfig` output
/// (API on 127.0.0.1:<port>), written privately next to the app kubeconfig.
pub(crate) async fn cluster_kubeconfig_file(
    state: &crate::state::AppState,
    cluster: &str,
) -> Result<PathBuf, String> {
    super::kind::check_cluster_name(cluster)?;
    let paths = &state.kind;
    if !paths.is_installed() {
        return Err("kind is not installed yet".into());
    }
    let host = super::kind::docker_host_for(state);
    let mut cmd = super::kind::kind_command(paths, &host);
    cmd.args(["get", "kubeconfig", "--name", cluster]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "kind get kubeconfig timed out".to_string())?
        .map_err(|e| format!("run kind get kubeconfig: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "kind get kubeconfig failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !text.contains("apiVersion") {
        return Err(format!("cluster {cluster} returned an empty kubeconfig"));
    }
    if text.contains("server: https://0.0.0.0:") || !text.contains("server: https://127.0.0.1:") {
        // Still usable, but tell the caller what is going on if it fails.
        tracing::debug!("cluster {cluster} kubeconfig has a non-loopback API address");
    }
    let dir = state
        .kind
        .kubeconfig
        .parent()
        .ok_or("kind data directory is unavailable")?
        .join("host-kubeconfigs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{cluster}.yaml"));
    write_private(&path, &text)?;
    Ok(path)
}

/// Removes the host kubeconfig of a deleted cluster (it holds admin keys).
pub(crate) fn forget_cluster_kubeconfig(state: &crate::state::AppState, cluster: &str) {
    if super::kind::check_cluster_name(cluster).is_err() {
        return;
    }
    if let Some(dir) = state.kind.kubeconfig.parent() {
        let _ = std::fs::remove_file(dir.join("host-kubeconfigs").join(format!("{cluster}.yaml")));
    }
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = path.with_extension("tmp");
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| e.to_string())?;
        f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

/// Runs a host tool against a cluster, returning stdout (stderr on failure).
pub(crate) async fn run_tool(
    state: &crate::state::AppState,
    tool: Tool,
    cluster: &str,
    args: &[String],
    timeout: std::time::Duration,
) -> Result<String, String> {
    let kubeconfig = cluster_kubeconfig_file(state, cluster).await?;
    run_tool_with(tool, Some(&kubeconfig), args, timeout).await
}

/// Runs a host tool with an optional kubeconfig (Helm repo commands need none).
pub(crate) async fn run_tool_with(
    tool: Tool,
    kubeconfig: Option<&Path>,
    args: &[String],
    timeout: std::time::Duration,
) -> Result<String, String> {
    let bin = ensure_tool(tool).await?;
    let mut cmd = Command::new(bin);
    // An empty kubeconfig path keeps the user's ~/.kube/config out of reach.
    let empty = tools_root().join("no-kubeconfig");
    cmd.args(args)
        .env("KUBECONFIG", kubeconfig.unwrap_or(&empty))
        .env("HELM_CONFIG_HOME", tools_root().join("helm-config"))
        .env("HELM_CACHE_HOME", tools_root().join("helm-cache"))
        .env("HELM_DATA_HOME", tools_root().join("helm-data"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("{} timed out after {}s", tool.name(), timeout.as_secs()))?
        .map_err(|e| format!("{}: {e}", tool.name()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("{} exited with {}", tool.name(), out.status) } else { err })
    }
}

pub(crate) fn helm_scratch_dir() -> PathBuf {
    tools_root().join("helm-scratch")
}

/// Env for a long-running tool process (port-forward).
pub(crate) async fn tool_command(
    state: &crate::state::AppState,
    tool: Tool,
    cluster: &str,
) -> Result<Command, String> {
    let bin = ensure_tool(tool).await?;
    let kubeconfig = cluster_kubeconfig_file(state, cluster).await?;
    let mut cmd = Command::new(bin);
    cmd.env("KUBECONFIG", kubeconfig);
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifacts_are_pinned_per_arch() {
        let (u, s) = artifact(Tool::Kubectl, "amd64").unwrap();
        assert!(u.ends_with("/v1.37.2/bin/linux/amd64/kubectl"));
        assert_eq!(s.len(), 64);
        let (u, _) = artifact(Tool::Helm, "arm64").unwrap();
        assert!(u.ends_with("helm-v3.22.1-linux-arm64.tar.gz"));
        assert!(artifact(Tool::Helm, "riscv").is_none());
    }
}
