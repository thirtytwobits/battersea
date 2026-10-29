mod bindings;
mod docs;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::{
    collections::HashSet,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Parser)]
#[command(about = "Battersea repository validation and packaging")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}
#[derive(Subcommand)]
enum Task {
    /// Format, check dependencies/versions, test and build Rust documentation. Needs no Node.
    Check,
    /// Check dependency direction and synchronised Cargo/npm versions.
    Boundaries,
    /// Scan AI-facing source against declared prompt roots.
    ScanPrompts,
    /// Generate JSON Schema, TypeScript types and shared fixtures from the Rust contract.
    Bindings {
        /// Refuse stale generated files without writing them.
        #[arg(long)]
        check: bool,
    },
    /// Verify public Rust APIs with the pinned cargo-public-api and nightly toolchain.
    Api {
        /// Accept and write an intentional API change.
        #[arg(long)]
        update: bool,
    },
    /// Build crate archives and compile an independent consumer of each archive.
    Package,
    /// Check the repository, licence and security contact against the package metadata.
    ReleaseCheck,
    /// Build the handbook and public Rust reference. Needs mdBook, but no Node.
    Docs,
    /// Check the assembled documentation, including links and anchors.
    DocsCheck {
        #[arg(long, default_value = "target/documentation/site")]
        site: PathBuf,
    },
    /// Check and archive this version's documentation for a release.
    DocsArchive,
    /// Assemble GitHub Pages from checksummed, published release archives. Needs gh.
    DocsSite,
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn json(path: impl AsRef<Path>) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
fn toml(path: impl AsRef<Path>) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}
fn run(root: &Path, command: &str, args: &[&str]) -> Result<()> {
    if !Command::new(command)
        .args(args)
        .current_dir(root)
        .status()?
        .success()
    {
        return Err(format!("{command} {} failed", args.join(" ")).into());
    }
    Ok(())
}
fn output(root: &Path, command: &str, args: &[&str]) -> Result<String> {
    let out = Command::new(command)
        .args(args)
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()?;
    if !out.status.success() {
        return Err(format!("{command} {} failed", args.join(" ")).into());
    }
    Ok(String::from_utf8(out.stdout)?)
}
fn reject(errors: Vec<String>) -> Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n").into())
    }
}
fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{name} must be confirmed in release.json").into())
}

/// Inspect resolved dependency identities, including renamed and transitive dependencies.
fn dependency_errors(metadata: &Value, policy: &Value, root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let packages = metadata["packages"].as_array().unwrap();
    let nodes = metadata["resolve"]["nodes"].as_array().unwrap();
    for package in packages
        .iter()
        .filter(|p| p["name"].as_str().unwrap().starts_with("battersea-"))
    {
        let name = package["name"].as_str().unwrap();
        let Some(allowed) = policy[name].as_array() else {
            errors.push(format!("{name} needs a dependency policy"));
            continue;
        };
        let node = nodes.iter().find(|n| n["id"] == package["id"]).unwrap();
        let allows = |target: &str| allowed.iter().any(|v| v == target);
        for dep in node["deps"].as_array().unwrap() {
            let target = packages.iter().find(|p| p["id"] == dep["pkg"]).unwrap();
            let target_name = target["name"].as_str().unwrap();
            if target_name.starts_with("battersea-") && !allows(target_name) {
                errors.push(format!("{name} cannot depend on {target_name}"));
            }
        }
        let mut pending = vec![package["id"].as_str().unwrap()];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let target = packages.iter().find(|p| p["id"] == id).unwrap();
            let target_name = target["name"].as_str().unwrap();
            if target_name == "rmcp" {
                errors.push(format!("{name} reaches rmcp outside Clapham's MCP crates"));
            }
            if target_name.starts_with("primrose-") || target_name.starts_with("clerkenwell-") {
                errors.push(format!("{name} reaches product dependency {target_name}"));
            }
            if target_name.starts_with("walthamstow-") {
                errors.push(format!("{name} reaches London piece {target_name}"));
            }
            if target_name.starts_with("clapham") && !allows(target_name) {
                errors.push(format!(
                    "{name} reaches {target_name} without a dependency policy"
                ));
            }
            // Clapham's MCP crates own rmcp; a crate allowed one of them reaches rmcp through it.
            if target_name.starts_with("clapham-mcp-") && allows(target_name) {
                continue;
            }
            if target["source"].is_null()
                && !Path::new(target["manifest_path"].as_str().unwrap()).starts_with(root)
            {
                errors.push(format!(
                    "{name} reaches a path dependency outside this repository: {target_name}"
                ));
            }
            if let Some(node) = nodes.iter().find(|n| n["id"] == id) {
                pending.extend(
                    node["deps"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|d| d["pkg"].as_str().unwrap()),
                );
            }
        }
    }
    errors
}
fn boundaries(root: &Path) -> Result<()> {
    if root.join("package.json").exists() {
        return Err("npm belongs in typescript/".into());
    }
    let release = json(root.join("release.json"))?;
    let version = string(&release, "version")?;
    let workspace = toml(root.join("Cargo.toml"))?;
    if workspace["workspace"]["package"]["version"].as_str() != Some(version) {
        return Err("Cargo and release.json versions differ".into());
    }
    let metadata: Value = serde_json::from_str(&output(
        root,
        "cargo",
        &["metadata", "--format-version", "1", "--locked"],
    )?)?;
    let mut errors = dependency_errors(
        &metadata,
        &json(root.join("contracts/dependencies.json"))?,
        root,
    );
    let crates = release["crates"]
        .as_object()
        .ok_or("release.json needs crates")?;
    let packages = metadata["packages"].as_array().unwrap();
    for name in crates.keys() {
        if !packages
            .iter()
            .any(|p| p["name"] == *name && p["version"] == version)
        {
            errors.push(format!("{name} is absent or has another version"));
        }
    }
    for package in packages
        .iter()
        .filter(|p| p["name"].as_str().unwrap().starts_with("battersea-"))
    {
        if !crates.contains_key(package["name"].as_str().unwrap()) {
            errors.push(format!(
                "{} missing from release inventory",
                package["name"]
            ));
        }
    }
    for entry in fs::read_dir(root.join("typescript/packages"))? {
        let path = entry?.path().join("package.json");
        if !path.exists() {
            continue;
        }
        let manifest = json(&path)?;
        let name = string(&manifest, "name")?;
        if manifest["version"] != version || release["npm"][name].is_null() {
            errors.push(format!(
                "{name} is missing from the release inventory or has another version"
            ));
        }
    }
    if json(root.join("typescript/package.json"))?["version"] != version {
        errors.push("TypeScript workspace version differs".into());
    }
    reject(errors)
}
fn api(root: &Path, update: bool) -> Result<()> {
    let settings = json(root.join("api/toolchain.json"))?;
    let version = string(&settings, "cargo-public-api")?;
    if output(root, "cargo", &["public-api", "--version"])?.trim()
        != format!("cargo-public-api {version}")
    {
        return Err(format!("Install cargo-public-api {version} with --locked").into());
    }
    let toolchain = format!("+{}", string(&settings, "rustdoc")?);
    for name in json(root.join("release.json"))?["crates"]
        .as_object()
        .unwrap()
        .keys()
    {
        let value = output(
            root,
            "cargo",
            &[
                &toolchain,
                "public-api",
                "--color",
                "never",
                "--all-features",
                "-p",
                name,
            ],
        )?;
        let file = root.join("api").join(format!("{name}.txt"));
        if update {
            fs::write(file, value)?;
        } else if fs::read_to_string(&file)? != value {
            return Err(format!(
                "{} is stale; review and run cargo xtask api --update",
                file.display()
            )
            .into());
        }
    }
    Ok(())
}
fn package(root: &Path) -> Result<()> {
    boundaries(root)?;
    let release = json(root.join("release.json"))?;
    let version = string(&release, "version")?;
    let metadata: Value = serde_json::from_str(&output(
        root,
        "cargo",
        &["metadata", "--no-deps", "--format-version", "1", "--locked"],
    )?)?;
    let target_dir = PathBuf::from(
        metadata["target_directory"]
            .as_str()
            .ok_or("Cargo returned no target directory")?,
    );
    let crates = release["crates"].as_object().unwrap().clone();
    let mut directories = std::collections::BTreeMap::new();
    for name in crates.keys() {
        let manifest = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == *name)
            .and_then(|p| p["manifest_path"].as_str())
            .ok_or("release crate has no manifest")?;
        directories.insert(
            name.clone(),
            Path::new(manifest)
                .parent()
                .ok_or("manifest has no directory")?
                .to_path_buf(),
        );
    }
    // Git dependencies, such as Clapham, are pinned by the lockfile. Archives name them by
    // version; consumers resolve those versions to the locked checkouts.
    let resolved: Value = serde_json::from_str(&output(
        root,
        "cargo",
        &["metadata", "--format-version", "1", "--locked"],
    )?)?;
    let mut git = std::collections::BTreeMap::new();
    for dependency in resolved["packages"].as_array().unwrap() {
        if dependency["source"]
            .as_str()
            .is_some_and(|source| source.starts_with("git+"))
        {
            git.insert(
                dependency["name"].as_str().unwrap().to_string(),
                Path::new(dependency["manifest_path"].as_str().unwrap())
                    .parent()
                    .ok_or("manifest has no directory")?
                    .to_path_buf(),
            );
        }
    }
    let unpack_root = tempfile::tempdir()?;
    for (name, required) in &crates {
        let directory = &directories[name];
        for asset in ["LICENSE", "NOTICE"] {
            fs::copy(root.join(asset), directory.join(asset))?;
        }
        let mut args = vec![
            "package".to_string(),
            "--allow-dirty".into(),
            "--no-verify".into(),
            "-p".into(),
            name.clone(),
        ];
        // Packages are distributed together. Resolve their versioned dependencies, Clapham's
        // included, locally while creating archives, then verify only the unpacked archives.
        // Cargo records the unused Clapham patches in the workspace lockfile, which is restored.
        for (dependency, path) in directories.iter().chain(&git) {
            args.extend([
                "--config".into(),
                format!("patch.crates-io.{dependency}.path={path:?}"),
            ]);
        }
        let lockfile = fs::read(root.join("Cargo.lock"))?;
        let packaged = run(
            root,
            "cargo",
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        fs::write(root.join("Cargo.lock"), lockfile)?;
        packaged?;
        let archive = target_dir
            .join("package")
            .join(format!("{name}-{version}.crate"));
        run(
            unpack_root.path(),
            "tar",
            &["-xzf", archive.to_str().unwrap()],
        )?;
        let unpacked = unpack_root.path().join(format!("{name}-{version}"));
        for required in required.as_array().unwrap() {
            if !unpacked.join(required.as_str().unwrap()).is_file() {
                return Err(format!("{name} package omits {required}").into());
            }
        }
        for asset in ["LICENSE", "NOTICE"] {
            if fs::read(unpacked.join(asset))? != fs::read(root.join(asset))? {
                return Err(format!("{name} package changed {asset}").into());
            }
        }
        if toml(unpacked.join("Cargo.toml"))?["package"]["license"].as_str()
            != release["license"].as_str()
        {
            return Err(format!("{name} package licence differs from release.json").into());
        }
    }
    for example in [
        "catalogue",
        "custom-node",
        "backend",
        "declarative-providers",
        "editor-server",
        "mcp",
        "ops",
    ] {
        let consumer = unpack_root.path().join(example);
        fs::create_dir_all(consumer.join("src"))?;
        let tokio_features = if matches!(example, "mcp" | "ops") {
            r#"["macros", "rt-multi-thread", "net", "time", "io-util", "io-std", "process", "signal"]"#
        } else {
            r#"["macros", "rt-multi-thread", "net", "time"]"#
        };
        let mut manifest = format!("[package]\nname = \"package-{example}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[dependencies]\nserde_json = \"1\"\nfutures-util = \"0.3\"\nasync-trait = \"0.1\"\ntokio = {{ version = \"1\", features = {tokio_features} }}\ntokio-util = \"0.7\"\n");
        if example == "editor-server" {
            manifest.push_str("sha2 = \"0.10\"\naxum = { version = \"0.8\", features = [\"json\"] }\nclap = { version = \"4\", features = [\"derive\", \"env\"] }\nserde = { version = \"1\", features = [\"derive\"] }\nuuid = { version = \"1\", features = [\"v4\"] }\ntempfile = \"3\"\n");
        }
        if example == "mcp" {
            manifest.push_str("axum = { version = \"0.8\", features = [\"json\"] }\nclap = { version = \"4\", features = [\"derive\"] }\ntempfile = \"3\"\n");
        }
        for name in crates.keys() {
            manifest.push_str(&format!(
                "{name} = {{ path = {:?} }}\n",
                unpack_root.path().join(format!("{name}-{version}"))
            ));
        }
        if example == "ops" {
            manifest.push_str("axum = \"0.8\"\n");
        }
        if matches!(example, "backend" | "ops") {
            manifest.push_str(
                "serde = { version = \"1\", features = [\"derive\"] }\nschemars = \"1\"\n",
            );
        }
        if matches!(example, "backend" | "editor-server" | "mcp" | "ops") {
            for (name, path) in &git {
                if name == "clapham"
                    || (example == "mcp" && name == "clapham-mcp-client")
                    || (example == "ops" && name != "clapham-mcp-client")
                    || (example == "editor-server"
                        && matches!(name.as_str(), "clapham-cli" | "clapham-mcp-server"))
                {
                    let features = if name == "clapham-mcp-client" {
                        ", features = [\"stdio\", \"http\"]"
                    } else {
                        ""
                    };
                    manifest.push_str(&format!("{name} = {{ path = {path:?}{features} }}\n"));
                }
            }
        }
        if example == "editor-server" {
            manifest
                .push_str("[[bin]]\nname = \"editor-server-example\"\npath = \"src/main.rs\"\n");
        }
        if matches!(example, "mcp" | "ops") {
            manifest.push_str(&format!("[lib]\nname = \"{example}_example\"\n[[bin]]\nname = \"{example}-example\"\npath = \"src/main.rs\"\n"));
        }
        manifest.push_str("[patch.crates-io]\n");
        for name in crates.keys() {
            manifest.push_str(&format!(
                "{name} = {{ path = {:?} }}\n",
                unpack_root.path().join(format!("{name}-{version}"))
            ));
        }
        for (name, path) in &git {
            manifest.push_str(&format!("{name} = {{ path = {path:?} }}\n"));
        }
        fs::write(consumer.join("Cargo.toml"), manifest)?;
        fs::write(
            consumer.join("src/main.rs"),
            fs::read_to_string(root.join(format!("examples/{example}/src/main.rs")))?,
        )?;
        if example == "mcp" {
            for module in [
                "lib.rs",
                "host.rs",
                "fixture.rs",
                "background_fixture.rs",
                "journal.rs",
                "conversation_journal.rs",
            ] {
                fs::copy(
                    root.join("examples/mcp/src").join(module),
                    consumer.join("src").join(module),
                )?;
            }
            fs::create_dir_all(consumer.join("tests"))?;
            fs::copy(
                root.join("examples/mcp/tests/contracts.rs"),
                consumer.join("tests/contracts.rs"),
            )?;
            for asset in ["LICENSE", "NOTICE"] {
                fs::copy(root.join("examples/mcp").join(asset), consumer.join(asset))?;
            }
        }
        if example == "ops" {
            fs::copy(
                root.join("examples/ops/src/lib.rs"),
                consumer.join("src/lib.rs"),
            )?;
            for directory in ["tests", "fixtures"] {
                fs::create_dir_all(consumer.join(directory))?;
            }
            for file in [
                "tests/conformance.rs",
                "tests/contracts.rs",
                "fixtures/example.json",
                "LICENSE",
                "NOTICE",
            ] {
                fs::copy(root.join("examples/ops").join(file), consumer.join(file))?;
            }
        }
        if example == "declarative-providers" {
            fs::copy(
                root.join("examples/declarative-providers/providers.yaml"),
                consumer.join("providers.yaml"),
            )?;
        }
        if example == "editor-server" {
            for module in ["ops.rs", "runtime.rs", "upgrade.rs"] {
                fs::copy(
                    root.join("examples/editor-server/src").join(module),
                    consumer.join("src").join(module),
                )?;
            }
            fs::create_dir_all(consumer.join("tests"))?;
            fs::copy(
                root.join("examples/editor-server/tests/cli.rs"),
                consumer.join("tests/cli.rs"),
            )?;
            fs::create_dir_all(consumer.join("fixtures"))?;
            for name in ["nodes.json", "example.json"] {
                fs::write(
                    consumer.join("fixtures").join(name),
                    fs::read(root.join("examples/editor-server/fixtures").join(name))?,
                )?;
            }
        }
        run(
            &consumer,
            "cargo",
            &[
                if matches!(example, "editor-server" | "mcp" | "ops") {
                    "test"
                } else {
                    "run"
                },
                "--offline",
                "--target-dir",
                unpack_root.path().join("consumer-target").to_str().unwrap(),
                "--config",
                "resolver.incompatible-rust-versions=\"fallback\"",
            ],
        )?;
    }
    Ok(())
}
fn release_check(root: &Path) -> Result<()> {
    boundaries(root)?;
    let release = json(root.join("release.json"))?;
    let repository = string(&release, "repository")?;
    let license = string(&release, "license")?;
    let security = string(&release, "security_contact")?;
    if !repository.starts_with("https://github.com/") || !security.starts_with("https://") {
        return Err("repository and security contact must be HTTPS URLs".into());
    }
    let manifest = toml(root.join("Cargo.toml"))?;
    let package = &manifest["workspace"]["package"];
    if package.get("license").and_then(toml::Value::as_str) != Some(license)
        || package.get("repository").and_then(toml::Value::as_str) != Some(repository)
    {
        return Err("Cargo metadata must carry the confirmed repository and licence".into());
    }
    if !root.join("LICENSE").is_file() {
        return Err("apply the confirmed licence to LICENSE before publication".into());
    }
    for entry in fs::read_dir(root.join("typescript/packages"))? {
        let path = entry?.path().join("package.json");
        if path.exists() && json(path)?["license"] != license {
            return Err("npm metadata must carry the confirmed licence".into());
        }
    }
    Ok(())
}
fn scan_prompts(root: &Path) -> Result<()> {
    let policy = battersea_guard::PromptPolicy::new(
        &["crates/battersea-nodes/src/nodes.yaml"],
        &[
            r"^crates/(battersea-model|battersea-tools|battersea-ops|battersea-providers|battersea-config|battersea-nodes)/src/.*\.(rs|yaml)$",
        ],
    )?;
    fn visit(
        root: &Path,
        directory: &Path,
        policy: &battersea_guard::PromptPolicy,
        violations: &mut Vec<battersea_guard::Violation>,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                visit(root, &path, policy, violations)?;
            } else {
                let name = path
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if policy.is_candidate(&name) {
                    violations.extend(battersea_guard::scan_content(
                        policy,
                        &name,
                        &fs::read_to_string(&path)?,
                        &[],
                    ));
                }
            }
        }
        Ok(())
    }
    let mut violations = Vec::new();
    visit(root, &root.join("crates"), &policy, &mut violations)?;
    if violations.is_empty() {
        Ok(())
    } else {
        Err(battersea_guard::render_report(&policy, &violations).into())
    }
}

fn main() -> Result<()> {
    let root = root();
    match Cli::parse().command {
        Task::Docs => docs::build(&root),
        Task::DocsCheck { site } => docs::check(&root.join(site)),
        Task::DocsArchive => docs::archive(&root),
        Task::DocsSite => docs::published_site(&root),
        Task::Bindings { check } => bindings::generate(&root, check),
        Task::Boundaries => boundaries(&root),
        Task::ScanPrompts => scan_prompts(&root),
        Task::Check => {
            run(&root, "cargo", &["fmt", "--all", "--check"])?;
            boundaries(&root)?;
            bindings::generate(&root, true)?;
            scan_prompts(&root)?;
            run(
                &root,
                "cargo",
                &[
                    "check",
                    "-p",
                    "battersea-providers",
                    "--no-default-features",
                    "--locked",
                ],
            )?;
            for feature in [
                "openai",
                "anthropic",
                "google",
                "runway",
                "replicate",
                "mock",
            ] {
                run(
                    &root,
                    "cargo",
                    &[
                        "check",
                        "-p",
                        "battersea-providers",
                        "--no-default-features",
                        "--features",
                        feature,
                        "--locked",
                    ],
                )?;
            }
            run(
                &root,
                "cargo",
                &["test", "--workspace", "--all-features", "--locked"],
            )?;
            run(
                &root,
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--locked",
                    "--",
                    "-D",
                    "warnings",
                ],
            )?;
            run(
                &root,
                "cargo",
                &["doc", "--workspace", "--no-deps", "--locked"],
            )
        }
        Task::Api { update } => api(&root, update),
        Task::Package => package(&root),
        Task::ReleaseCheck => release_check(&root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn metadata(dependency: &str, source: Value, path: &str) -> Value {
        json!({"packages":[
            {"id":"root","name":"battersea-flow","source":null,"manifest_path":"/repo/crates/flow/Cargo.toml"},
            {"id":"helper","name":"helper","source":"registry","manifest_path":"/cache/helper/Cargo.toml"},
            {"id":"dependency","name":dependency,"source":source,"manifest_path":path}
        ],"resolve":{"nodes":[
            {"id":"root","deps":[{"name":"renamed","pkg":"helper"}]},
            {"id":"helper","deps":[{"name":"alias","pkg":"dependency"}]},
            {"id":"dependency","deps":[]}
        ]}})
    }
    #[test]
    fn a_transitive_product_dependency_is_rejected_even_when_renamed() {
        for name in ["primrose-protocol", "clerkenwell-doc"] {
            let errors = dependency_errors(
                &metadata(name, json!("git"), "/cache/Cargo.toml"),
                &json!({"battersea-flow":[]}),
                Path::new("/repo"),
            );
            assert!(errors.iter().any(|e| e.contains(name)));
        }
    }
    #[test]
    fn an_external_path_dependency_is_rejected_but_registry_dependencies_are_allowed() {
        let policy = json!({"battersea-flow":[]});
        assert!(!dependency_errors(
            &metadata("helper-other", Value::Null, "/elsewhere/Cargo.toml"),
            &policy,
            Path::new("/repo")
        )
        .is_empty());
        assert!(dependency_errors(
            &metadata("helper-other", json!("registry"), "/cache/Cargo.toml"),
            &policy,
            Path::new("/repo")
        )
        .is_empty());
    }
    #[test]
    fn the_mcp_sdk_is_reached_only_through_claphams_mcp_crates() {
        assert!(dependency_errors(
            &metadata("rmcp", json!("registry"), "/cache/Cargo.toml"),
            &json!({"battersea-flow":[]}),
            Path::new("/repo")
        )
        .iter()
        .any(|error| error.contains("rmcp")));
        let mut data = metadata("clapham-mcp-client", json!("git"), "/cache/Cargo.toml");
        data["packages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"rmcp","name":"rmcp","source":"registry","manifest_path":"/cache/rmcp/Cargo.toml"}));
        data["resolve"]["nodes"][2]["deps"] = json!([{"name":"rmcp","pkg":"rmcp"}]);
        data["resolve"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"rmcp","deps":[]}));
        assert!(dependency_errors(
            &data,
            &json!({"battersea-flow":["clapham-mcp-client"]}),
            Path::new("/repo")
        )
        .is_empty());
    }
    #[test]
    fn a_london_piece_is_reached_only_where_the_policy_names_it() {
        for (name, policy) in [
            ("clapham", json!({"battersea-flow":[]})),
            (
                "walthamstow-workspace",
                json!({"battersea-flow":["walthamstow-workspace"]}),
            ),
        ] {
            let errors = dependency_errors(
                &metadata(name, json!("git"), "/cache/Cargo.toml"),
                &policy,
                Path::new("/repo"),
            );
            assert!(errors.iter().any(|e| e.contains(name)), "{name}");
        }
        assert!(dependency_errors(
            &metadata("clapham", json!("git"), "/cache/Cargo.toml"),
            &json!({"battersea-flow":["clapham"]}),
            Path::new("/repo")
        )
        .is_empty());
    }

    #[test]
    fn unresolved_publication_values_are_not_strings() {
        for v in [Value::Null, json!(""), json!(false)] {
            assert!(string(&json!({"repository":v}), "repository").is_err());
        }
        let repository = "https://github.com/owner/project";
        assert_eq!(
            string(&json!({"repository":repository}), "repository").unwrap(),
            repository
        );
    }
}
