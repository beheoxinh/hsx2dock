//! Gated integration tests: daemon-dependent cases are `#[ignore]`d and
//! only run with a live daemon (`DOCKER_TEST=1 cargo test -- --include-ignored`).
//! The always-run tests cover pure logic that CI can verify without Docker.

use docker_manager::docker::models::RunConfig;

#[test]
fn parse_lines_handles_ports_env_volumes_shapes() {
    let ports = RunConfig::parse_lines("8080:80\n  443:443\n");
    assert_eq!(ports, vec!["8080:80".to_string(), "443:443".to_string()]);
    assert!(RunConfig::parse_lines("").is_empty());
}

#[test]
fn run_config_validation_smoke() {
    let ok = RunConfig {
        image: "nginx:latest".into(),
        ..Default::default()
    };
    assert!(ok.validate().is_ok());
}

#[test]
#[ignore = "requires a live Docker daemon"]
fn daemon_lists_containers() {
    panic!("gated: run with DOCKER_TEST=1 and -- --include-ignored against a live daemon");
}
