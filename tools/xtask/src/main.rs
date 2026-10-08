mod bindings;
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
    /// Check publication decisions and require the tag to match all package versions.
    ReleaseCheck { tag: String },
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
        for dep in node["deps"].as_array().unwrap() {
            let target = packages.iter().find(|p| p["id"] == dep["pkg"]).unwrap();
            let target_name = target["name"].as_str().unwrap();
            if target_name.starts_with("battersea-") && !allowed.iter().any(|v| v == target_name) {
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
            if target_name.starts_with("primrose-") || target_name.starts_with("clerkenwell-") {
                errors.push(format!("{name} reaches product dependency {target_name}"));
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
    for (name, required) in release["crates"].as_object().unwrap() {
        let manifest_path = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == *name)
            .and_then(|package| package["manifest_path"].as_str())
            .ok_or("release crate has no manifest")?;
        let crate_dir = Path::new(manifest_path)
            .parent()
            .ok_or("manifest has no directory")?;
        for asset in ["LICENSE", "NOTICE"] {
            fs::copy(root.join(asset), crate_dir.join(asset))?;
        }
        run(
            root,
            "cargo",
            &["package", "--locked", "--allow-dirty", "-p", name],
        )?;
        let archive = target_dir
            .join("package")
            .join(format!("{name}-{version}.crate"));
        let temp = tempfile::tempdir()?;
        run(temp.path(), "tar", &["-xzf", archive.to_str().unwrap()])?;
        let unpacked = temp.path().join(format!("{name}-{version}"));
        for required in required.as_array().unwrap() {
            let required = required.as_str().unwrap();
            if !unpacked.join(required).is_file() {
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
        let consumer = temp.path().join("consumer");
        fs::create_dir_all(consumer.join("src"))?;
        fs::write(consumer.join("Cargo.toml"), format!("[package]\nname = \"package-consumer\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[dependencies]\n{name} = {{ path = {:?} }}\nserde_json = \"1\"\n", unpacked))?;
        fs::write(
            consumer.join("src/main.rs"),
            fs::read_to_string(root.join("examples/catalogue/src/main.rs"))?,
        )?;
        run(
            &consumer,
            "cargo",
            &[
                "run",
                "--offline",
                "--config",
                "resolver.incompatible-rust-versions=\"fallback\"",
            ],
        )?;
    }
    Ok(())
}
fn release_check(root: &Path, tag: &str) -> Result<()> {
    boundaries(root)?;
    let release = json(root.join("release.json"))?;
    if tag != format!("v{}", string(&release, "version")?) {
        return Err("tag does not match the release version".into());
    }
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
    println!("{tag}");
    Ok(())
}
fn main() -> Result<()> {
    let root = root();
    match Cli::parse().command {
        Task::Bindings { check } => bindings::generate(&root, check),
        Task::Boundaries => boundaries(&root),
        Task::Check => {
            run(&root, "cargo", &["fmt", "--all", "--check"])?;
            boundaries(&root)?;
            bindings::generate(&root, true)?;
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
        Task::ReleaseCheck { tag } => release_check(&root, &tag),
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
