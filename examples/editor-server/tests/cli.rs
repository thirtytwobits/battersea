//! The assembled CLI runs the example's operations against its daemon over MCP, under the grant
//! its token carries.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

const BINARY: &str = env!("CARGO_BIN_EXE_editor-server-example");

struct Daemon {
    root: PathBuf,
    process: std::process::Child,
    _directory: tempfile::TempDir,
}
impl Daemon {
    fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("app");
        assert!(Command::new(BINARY)
            .arg("init")
            .arg(&root)
            .status()
            .unwrap()
            .success());
        let process = Command::new(BINARY)
            .arg("serve")
            .arg(&root)
            .args(["--port", "0"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("daemon.json").is_file() {
            assert!(
                Instant::now() < deadline,
                "the daemon wrote no endpoint record"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            root,
            process,
            _directory: directory,
        }
    }
    fn record(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.root.join("daemon.json")).unwrap()).unwrap()
    }
    fn token(&self, subject: &str) -> String {
        let tokens: Value =
            serde_json::from_slice(&std::fs::read(self.root.join("tokens.json")).unwrap()).unwrap();
        tokens
            .as_object()
            .unwrap()
            .iter()
            .find(|(_, grant)| grant["subject"] == subject)
            .map(|(token, _)| token.clone())
            .unwrap()
    }
    fn run(&self, args: &[&str]) -> Output {
        run(&self.root, args)
    }
}
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(BINARY)
        .arg("--root")
        .arg(root)
        .args(args)
        .env_remove("BATTERSEA_ENDPOINT")
        .env_remove("BATTERSEA_TOKEN")
        .output()
        .unwrap()
}

fn lines(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn an_activation_runs_from_the_cli_over_mcp_with_its_events_in_order() {
    let daemon = Daemon::start();
    let output = daemon.run(&[
        "battersea",
        "run",
        "activate",
        "example",
        "text",
        "--parameters-json",
        r#"{"text":"from the command line"}"#,
    ]);
    assert!(output.status.success(), "{output:?}");
    let printed = lines(&output);
    let (outcome, records) = printed.split_last().unwrap();
    assert_eq!(outcome["data"]["status"], "succeeded");
    assert_eq!(outcome["data"]["terminal"], true);
    let run_id = outcome["data"]["run_id"].as_str().unwrap();
    assert_eq!(records[0]["data"]["detail"]["status"], "accepted");
    assert!(records
        .iter()
        .all(|record| record["kind"] == "battersea.run.activate"
            && record["data"]["activation_id"] == run_id));
    let sequences = records
        .iter()
        .map(|record| {
            record["data"]["sequence"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(sequences.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(records.last().unwrap()["data"]["phase"], "succeeded");
    let status = daemon.run(&["battersea", "run", "status", run_id]);
    assert!(status.status.success());
    assert_eq!(lines(&status)[0]["data"]["status"], "succeeded");
}

#[test]
fn each_token_carries_its_own_grant() {
    let daemon = Daemon::start();
    let endpoint = daemon.record()["endpoint"].as_str().unwrap().to_string();
    let agent = daemon.token("agent");
    let as_agent = |args: &[&str]| {
        Command::new(BINARY)
            .args(["--endpoint", &endpoint, "--token", &agent])
            .args(args)
            .output()
            .unwrap()
    };
    let listed = as_agent(&["battersea", "flow", "list"]);
    assert!(listed.status.success(), "{listed:?}");
    // The agent's grant cannot discover the administrative operation, so its CLI has no command.
    let resume = as_agent(&["battersea", "run", "resume", "unknown", "--yes"]);
    assert_eq!(resume.status.code(), Some(2));
    let refusal: Value = serde_json::from_slice(&resume.stderr).unwrap();
    assert_eq!(refusal["code"], "usage");
    let resume = daemon.run(&["battersea", "run", "resume", "unknown", "--yes"]);
    assert_eq!(resume.status.code(), Some(78), "{resume:?}");
    let refusal: Value = serde_json::from_slice(&resume.stderr).unwrap();
    assert_eq!(refusal["code"], "unsupported_host_operation");
    let rejected = Command::new(BINARY)
        .args(["--endpoint", &endpoint, "--token", "not-a-token"])
        .args(["battersea", "flow", "list"])
        .output()
        .unwrap();
    assert_eq!(rejected.status.code(), Some(77));
    let refusal: Value = serde_json::from_slice(&rejected.stderr).unwrap();
    assert_eq!(refusal["code"], "mcp_unauthenticated");
}

#[test]
fn an_unknown_flow_is_refused_with_its_category_exit_code() {
    let daemon = Daemon::start();
    for args in [
        &["battersea", "flow", "get", "missing"][..],
        &["battersea", "run", "activate", "missing", "text"],
        &[
            "battersea",
            "flow",
            "upgrade",
            "missing",
            "--dry-run",
            "--yes",
        ],
        &[
            "battersea",
            "flow",
            "upgrade",
            "missing",
            "--expected_revision",
            "any",
            "--yes",
        ],
    ] {
        let output = daemon.run(args);
        assert_eq!(output.status.code(), Some(66), "{args:?}: {output:?}");
        let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(refusal["code"], "unknown_flow", "{args:?}");
    }
    let output = daemon.run(&["battersea", "run", "status", "unknown"]);
    assert_eq!(output.status.code(), Some(66));
}

#[test]
fn an_unreadable_flow_is_refused_as_invalid() {
    let daemon = Daemon::start();
    for (name, source) in [
        ("malformed", &b"{ not json"[..]),
        ("binary", &b"\xff\xfe"[..]),
        (
            "future",
            &br#"{"version":999,"flow_key":"future","title":"Future","nodes":[]}"#[..],
        ),
    ] {
        std::fs::write(daemon.root.join(format!("flows/{name}.json")), source).unwrap();
        for args in [
            &["battersea", "flow", "get", name][..],
            &["battersea", "run", "activate", name, "text"],
            &["battersea", "flow", "upgrade", name, "--dry-run", "--yes"],
            &[
                "battersea",
                "flow",
                "upgrade",
                name,
                "--expected_revision",
                "any",
                "--yes",
            ],
        ] {
            let output = daemon.run(args);
            assert_eq!(output.status.code(), Some(65), "{args:?}: {output:?}");
            let refusal: Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(refusal["code"], "invalid_flow", "{args:?}");
        }
    }
}
