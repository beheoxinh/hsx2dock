use std::path::Path;
use std::sync::Arc;
use tauri::State;

use crate::docker::compose;
use crate::state::AppState;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ComposeProjectInfo {
    pub name: String,
    pub path: String,
    pub services: Vec<compose::ComposeService>,
}

#[tauri::command]
pub async fn list_compose_projects(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ComposeProjectInfo>, String> {
    list_compose_projects_core(&state).await
}

/// App-free core of [`list_compose_projects`], shared with the MCP server.
pub(crate) async fn list_compose_projects_core(
    state: &AppState,
) -> Result<Vec<ComposeProjectInfo>, String> {
    let configured = state.config.read().await.compose_file.clone();
    let mut projects = Vec::new();
    if let Ok(f) = compose::resolve_compose_file(&configured) {
        if let Ok(services) = compose::ps(&f).await {
            let name = f
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();
            projects.push(ComposeProjectInfo {
                name,
                path: f.display().to_string(),
                services,
            });
        }
    }
    // Projects started from anywhere (the app's compose groups, a terminal)
    // are known by their container labels; the configured file used to be
    // the only source, so every other project was invisible here.
    if let Ok(client) = state.get_client().await {
        if let Ok(containers) = client.list_containers(true).await {
            for p in projects_from_containers(&containers) {
                if projects.iter().any(|q| q.name == p.name) {
                    continue;
                }
                let path = compose_config_file(&client, &p.services).await;
                projects.push(ComposeProjectInfo { path, ..p });
            }
        }
    }
    Ok(projects)
}

/// Groups containers carrying a compose project label into projects, one
/// service row per container, sorted by project then service.
fn projects_from_containers(
    containers: &[crate::docker::models::ContainerItem],
) -> Vec<ComposeProjectInfo> {
    let mut by_project: std::collections::BTreeMap<String, Vec<compose::ComposeService>> =
        Default::default();
    for c in containers {
        let Some(project) = c.compose_project.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let health = ["unhealthy", "healthy", "health: starting"]
            .into_iter()
            .find(|h| c.status.contains(&format!("({h})")))
            .map(|h| h.trim_start_matches("health: ").to_string())
            .unwrap_or_default();
        by_project.entry(project.clone()).or_default().push(compose::ComposeService {
            id: c.id.clone(),
            name: c.name.clone(),
            service: c.compose_service.clone().unwrap_or_default(),
            image: c.image.clone(),
            state: c.state.clone(),
            health,
            ports: c.ports.clone(),
        });
    }
    by_project
        .into_iter()
        .map(|(name, mut services)| {
            services.sort_by(|a, b| a.service.cmp(&b.service));
            ComposeProjectInfo { name, path: String::new(), services }
        })
        .collect()
}

/// The compose file a project was started from, read from one of its
/// containers (`com.docker.compose.project.config_files`, first entry).
async fn compose_config_file(
    client: &crate::docker::client::DockerClient,
    services: &[compose::ComposeService],
) -> String {
    let Some(first) = services.first() else {
        return String::new();
    };
    client
        .docker()
        .inspect_container(&first.id, None)
        .await
        .ok()
        .and_then(|d| d.config?.labels?.get("com.docker.compose.project.config_files").cloned())
        .and_then(|files| files.split(',').next().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::models::ContainerItem;

    fn item(name: &str, project: Option<&str>, service: &str, status: &str) -> ContainerItem {
        ContainerItem {
            id: format!("id-{name}"),
            name: name.into(),
            state: "running".into(),
            status: status.into(),
            compose_project: project.map(str::to_string),
            compose_service: Some(service.into()),
            ..Default::default()
        }
    }

    #[test]
    fn label_projects_group_containers_and_read_health() {
        let list = vec![
            item("b-api", Some("shop"), "api", "Up 2 minutes (healthy)"),
            item("lone", None, "x", "Up"),
            item("b-db", Some("shop"), "db", "Up 1 minute (health: starting)"),
            item("c-web", Some("blog"), "web", "Up 3 minutes (unhealthy)"),
        ];
        let projects = projects_from_containers(&list);
        let names: Vec<_> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["blog", "shop"]);
        assert_eq!(projects[0].services[0].health, "unhealthy");
        let shop: Vec<_> = projects[1].services.iter().map(|s| (s.service.as_str(), s.health.as_str())).collect();
        assert_eq!(shop, [("api", "healthy"), ("db", "starting")]);
    }
}

#[tauri::command]
pub async fn compose_up(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_lifecycle_core(&file_path, true).await
}

/// App-free core of [`compose_up`]/[`compose_down`], shared with the MCP server.
pub(crate) async fn compose_lifecycle_core(file_path: &str, up: bool) -> Result<String, String> {
    let path = Path::new(file_path);
    if up {
        compose::up(path).await
    } else {
        compose::down(path).await
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compose_down(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_lifecycle_core(&file_path, false).await
}

#[tauri::command]
pub async fn compose_logs(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
    tail: String,
) -> Result<String, String> {
    let tail_n: usize = tail.parse().unwrap_or(100);
    compose_logs_core(&file_path, tail_n).await
}

/// App-free core of [`compose_logs`], shared with the MCP server.
pub(crate) async fn compose_logs_core(file_path: &str, tail: usize) -> Result<String, String> {
    compose::logs(Path::new(file_path), tail)
        .await
        .map_err(|e| e.to_string())
}

/// Lifecycle for a compose project addressed by name only (`compose -p X
/// start|stop|restart`) — this is what the Containers page's project headers
/// use, and it needs no compose file because the containers carry the
/// project's labels.
#[tauri::command]
pub async fn compose_project_action(project: String, action: String) -> Result<String, String> {
    compose_project_action_core(&project, &action).await
}

/// App-free core of [`compose_project_action`], shared with the MCP server.
pub(crate) async fn compose_project_action_core(
    project: &str,
    action: &str,
) -> Result<String, String> {
    compose::project_action(project, action)
        .await
        .map_err(|e| format!("{e:#}"))
}
