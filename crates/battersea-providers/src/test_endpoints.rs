//! Endpoint fixtures shared by Rust tests. Live listeners request an OS-assigned port.
#![allow(dead_code)]
pub const TEST_HTTP_URL: &str = "http://127.0.0.1:18417";
pub const TEST_BIND_ADDRESS: &str = "127.0.0.1:0";

pub fn test_http_url() -> String {
    TEST_HTTP_URL.to_owned()
}

pub fn test_ws_url() -> String {
    format!("{}/ws", TEST_HTTP_URL.replacen("http:", "ws:", 1))
}

pub fn configure_test_server<'a>(
    command: &'a mut std::process::Command,
    root: &std::path::Path,
    state: &std::path::Path,
) -> &'a mut std::process::Command {
    let explicit: Vec<_> = command.get_envs().map(|(key, _)| key.to_owned()).collect();
    for key in explicit {
        if key.to_string_lossy().starts_with("PRIMROSE_ENGINE_") {
            command.env_remove(key);
        }
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PRIMROSE_ENGINE_") {
            command.env_remove(key);
        }
    }
    command.env_remove("PRIMROSE_ENGINE_URL");
    let bind: std::net::SocketAddr = TEST_BIND_ADDRESS.parse().unwrap();
    command
        .env("PRIMROSE_ENGINE_ROOT", root)
        .env("PRIMROSE_ENGINE_DATA_DIR", state)
        .env("PRIMROSE_ENGINE_HOST", bind.ip().to_string())
        .env("PRIMROSE_ENGINE_PORT", bind.port().to_string())
}
