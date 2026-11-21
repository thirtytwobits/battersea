//! Copyright (c) Scott A Dixon
//!
//! The repository gate and its regression subset.
//!
//! Test binaries are built once and run concurrently. The static steps (feature checks, Clippy
//! and rustdoc) run beside them in a second Cargo target directory, so neither lane waits for
//! the other's build lock.

use crate::{bindings, boundaries, scan_prompts, Result};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

const PROVIDER_FEATURES: [&str; 6] = [
    "openai",
    "anthropic",
    "google",
    "runway",
    "replicate",
    "mock",
];

struct TestBinary {
    name: String,
    package: PathBuf,
    executable: PathBuf,
}
struct Outcome {
    name: String,
    passed: bool,
    took: Duration,
    output: String,
}

/// Every gate step. The coverage is that of the sequential gate this replaces.
pub fn check(root: &Path) -> Result<()> {
    let started = Instant::now();
    preflight(root)?;
    let outcomes = thread::scope(|scope| {
        let lane = scope.spawn(|| static_lane(root));
        let mut outcomes = tests(root, true);
        outcomes.extend(lane.join().expect("static lane panicked"));
        outcomes
    });
    conclude(outcomes, started)
}

/// The preflight checks and every test binary: no doc-tests, feature checks, Clippy or rustdoc.
pub fn regression(root: &Path) -> Result<()> {
    let started = Instant::now();
    preflight(root)?;
    conclude(tests(root, false), started)
}

fn preflight(root: &Path) -> Result<()> {
    crate::run(root, "cargo", &["fmt", "--all", "--check"])?;
    boundaries(root)?;
    bindings::generate(root, true)?;
    scan_prompts(root)
}

fn conclude(outcomes: Vec<Outcome>, started: Instant) -> Result<()> {
    let failed: Vec<&Outcome> = outcomes.iter().filter(|o| !o.passed).collect();
    for outcome in &failed {
        eprintln!("\n---- {} ----\n{}", outcome.name, outcome.output);
    }
    eprintln!(
        "{} steps, {} failed, {:.0}s",
        outcomes.len(),
        failed.len(),
        started.elapsed().as_secs_f64()
    );
    if failed.is_empty() {
        Ok(())
    } else {
        let names: Vec<&str> = failed.iter().map(|o| o.name.as_str()).collect();
        Err(format!("failed: {}", names.join(", ")).into())
    }
}

fn report(outcome: Outcome) -> Outcome {
    eprintln!(
        "{} {:7.1}s {}",
        if outcome.passed { "  ok" } else { "FAIL" },
        outcome.took.as_secs_f64(),
        outcome.name
    );
    outcome
}

/// Runs one Cargo command with its output captured.
fn cargo(root: &Path, target: Option<&Path>, name: &str, args: &[&str]) -> Outcome {
    let started = Instant::now();
    let mut command = crate::command("cargo");
    command.args(args).current_dir(root);
    if let Some(target) = target {
        command.env("CARGO_TARGET_DIR", target);
    }
    let (passed, output) = match command.output() {
        Ok(out) => (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ),
        Err(error) => (false, error.to_string()),
    };
    report(Outcome {
        name: name.to_owned(),
        passed,
        took: started.elapsed(),
        output,
    })
}

/// Feature checks, Clippy and rustdoc, in order, stopping at the first failure.
fn static_lane(root: &Path) -> Vec<Outcome> {
    // A directory of its own inside the target directory in use.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| root.join("target"), PathBuf::from)
        .join("static");
    let base = [
        "check",
        "-p",
        "battersea-providers",
        "--no-default-features",
        "--locked",
    ];
    let mut steps: Vec<(String, Vec<&str>)> =
        vec![("check providers, no features".to_owned(), base.to_vec())];
    for feature in PROVIDER_FEATURES {
        let mut args = base.to_vec();
        args.extend(["--features", feature]);
        steps.push((format!("check providers, {feature}"), args));
    }
    steps.push((
        "clippy".to_owned(),
        vec![
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    ));
    steps.push((
        "doc".to_owned(),
        vec!["doc", "--workspace", "--no-deps", "--locked"],
    ));
    let mut outcomes = Vec::new();
    for (name, args) in steps {
        let outcome = cargo(root, Some(&target), &name, &args);
        let passed = outcome.passed;
        outcomes.push(outcome);
        if !passed {
            break;
        }
    }
    outcomes
}

/// Builds the test binaries, then runs them concurrently, beside the doc-tests when asked.
fn tests(root: &Path, doc_tests: bool) -> Vec<Outcome> {
    let started = Instant::now();
    let binaries = match build(root) {
        Ok(binaries) => binaries,
        Err(error) => {
            return vec![report(Outcome {
                name: "build tests".to_owned(),
                passed: false,
                took: started.elapsed(),
                output: error.to_string(),
            })];
        }
    };
    eprintln!(
        "  ok {:7.1}s build tests ({} binaries)",
        started.elapsed().as_secs_f64(),
        binaries.len()
    );
    thread::scope(|scope| {
        let docs = doc_tests.then(|| {
            scope.spawn(|| {
                cargo(
                    root,
                    None,
                    "doc-tests",
                    &["test", "--workspace", "--all-features", "--locked", "--doc"],
                )
            })
        });
        let mut outcomes = run_binaries(&binaries);
        outcomes.extend(docs.map(|docs| docs.join().expect("doc-tests panicked")));
        outcomes
    })
}

/// Builds every test binary once and returns them from Cargo's artefact messages.
fn build(root: &Path) -> Result<Vec<TestBinary>> {
    let mut child = crate::command("cargo")
        .args([
            "test",
            "--workspace",
            "--all-features",
            "--locked",
            "--no-run",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root)
        .stdout(Stdio::piped())
        .spawn()?;
    let mut binaries = Vec::new();
    for line in BufReader::new(child.stdout.take().expect("piped stdout")).lines() {
        let Ok(message) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if message["reason"] != "compiler-artifact" || message["profile"]["test"] != true {
            continue;
        }
        let (Some(executable), Some(manifest), Some(target)) = (
            message["executable"].as_str(),
            message["manifest_path"].as_str(),
            message["target"]["name"].as_str(),
        ) else {
            continue;
        };
        let package = Path::new(manifest)
            .parent()
            .ok_or("manifest has no directory")?;
        let directory = package.strip_prefix(root).unwrap_or(package);
        binaries.push(TestBinary {
            name: format!("{} {target}", directory.display()),
            package: package.to_owned(),
            executable: executable.into(),
        });
    }
    if !child.wait()?.success() {
        return Err("cargo test --no-run failed".into());
    }
    binaries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(binaries)
}

/// Runs each binary from its package directory, as `cargo test` does.
fn run_binary(binary: &TestBinary, args: &[&str]) -> Outcome {
    let started = Instant::now();
    let (passed, output) = match Command::new(&binary.executable)
        .args(args)
        .current_dir(&binary.package)
        .env("CARGO_MANIFEST_DIR", &binary.package)
        .output()
    {
        Ok(out) => (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        ),
        Err(error) => (false, error.to_string()),
    };
    Outcome {
        name: binary.name.clone(),
        passed,
        took: started.elapsed(),
        output,
    }
}

fn run_binaries(binaries: &[TestBinary]) -> Vec<Outcome> {
    // An operating system that assesses an executable on its first launch delays that launch.
    // Listing every binary at once overlaps those delays and counts the tests.
    let started = Instant::now();
    let listed: Vec<Outcome> = thread::scope(|scope| {
        let handles: Vec<_> = binaries
            .iter()
            .map(|binary| scope.spawn(|| run_binary(binary, &["--list", "--format", "terse"])))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("listing panicked"))
            .collect()
    });
    let count: usize = listed
        .iter()
        .map(|outcome| {
            outcome
                .output
                .lines()
                .filter(|line| line.ends_with(": test"))
                .count()
        })
        .sum();
    eprintln!(
        "  ok {:7.1}s list tests ({count} tests)",
        started.elapsed().as_secs_f64()
    );
    let unlisted: Vec<Outcome> = listed.into_iter().filter(|o| !o.passed).collect();
    if !unlisted.is_empty() {
        return unlisted.into_iter().map(report).collect();
    }
    let width = thread::available_parallelism().map_or(4, usize::from);
    let queue = Mutex::new(binaries.iter().collect::<VecDeque<_>>());
    let outcomes = Mutex::new(Vec::new());
    thread::scope(|scope| {
        for _ in 0..width.min(binaries.len()) {
            scope.spawn(|| loop {
                let Some(binary) = queue.lock().expect("queue").pop_front() else {
                    break;
                };
                let outcome = report(run_binary(binary, &[]));
                outcomes.lock().expect("outcomes").push(outcome);
            });
        }
    });
    outcomes.into_inner().expect("outcomes")
}
