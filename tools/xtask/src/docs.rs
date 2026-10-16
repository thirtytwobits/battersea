//! Documentation builds and release-archive assembly. No command in this module invokes Node.
use super::{json, output, run, string, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use percent_encoding::percent_decode_str;
use scraper::{Html, Selector};
use semver::Version;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fmt::Write,
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
    process::Command,
};
use url::Url;

const SITE: &str = "target/documentation/site";
const PUBLISHED: &str = "target/documentation/published";
// This bounds unpacked release assets, rather than trusting tar headers or gzip sizes.
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

fn tag_version(tag: &str) -> Result<Version> {
    let version = Version::parse(tag.strip_prefix('v').ok_or("Tag must start with v")?)?;
    if tag != format!("v{version}") {
        return Err(format!("Non-canonical version tag {tag}").into());
    }
    Ok(version)
}

fn version(root: &Path) -> Result<String> {
    let tag = format!("v{}", string(&json(root.join("release.json"))?, "version")?);
    tag_version(&tag)?;
    Ok(tag)
}

fn clean(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    fs::create_dir_all(path)?;
    Ok(())
}

/// Copy only regular files and directories. Pages archives cannot contain links.
fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for item in fs::read_dir(source)? {
        let item = item?;
        let kind = item.file_type()?;
        if kind.is_dir() {
            copy_tree(&item.path(), &target.join(item.file_name()))?;
        } else if kind.is_file() {
            fs::copy(item.path(), target.join(item.file_name()))?;
        } else {
            return Err(format!(
                "Documentation contains a link or special file: {:?}",
                item.path()
            )
            .into());
        }
    }
    Ok(())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(title: &str, body: &str) -> String {
    format!("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>body{{max-width:64rem;margin:3rem auto;padding:0 1.5rem;font:18px/1.6 system-ui,sans-serif}}a{{color:#1559a0}}code{{font-size:.9em}}@media(prefers-color-scheme:dark){{body{{background:#161923;color:#eee}}a{{color:#9fcaff}}}}</style></head><body>{body}</body></html>\n", escape(title))
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(value)?))?;
    Ok(())
}

pub fn build(root: &Path) -> Result<()> {
    super::bindings::generate(root, true)?;
    let tag = version(root)?;
    let release = json(root.join("release.json"))?;
    let toolchain = json(root.join("docs/toolchain.json"))?;
    let rust = string(&toolchain, "rust")?;
    let mdbook = string(&toolchain, "mdbook")?;
    if output(root, "mdbook", &["--version"])?.trim() != format!("mdbook v{mdbook}") {
        return Err(format!(
            "Install mdbook {mdbook}: cargo install mdbook --version {mdbook} --locked"
        )
        .into());
    }
    let site = root.join(SITE);
    let destination = site.join(&tag);
    fs::create_dir_all(&destination)?;
    let book = root.join("target/documentation/book");
    clean(&book)?;
    copy_tree(&root.join("docs/src"), &book.join("src"))?;
    fs::copy(root.join("docs/book.toml"), book.join("book.toml"))?;
    copy_tree(&root.join("contracts"), &book.join("src/contracts"))?;
    for (source, target) in [
        ("README.md", "overview.md"),
        ("CONTRIBUTING.md", "CONTRIBUTING.md"),
        ("RELEASING.md", "RELEASING.md"),
        ("CHANGELOG.md", "CHANGELOG.md"),
        ("docs/README.md", "documentation.md"),
        ("crates/battersea-runtime/README.md", "runtime.md"),
        ("typescript/packages/editor/README.md", "editor.md"),
        ("examples/editor-server/README.md", "editor-server.md"),
        (
            "typescript/packages/editor/THIRD_PARTY_NOTICES.md",
            "THIRD_PARTY_NOTICES.md",
        ),
        ("provenance.json", "provenance.json"),
        ("LICENSE", "LICENSE"),
        ("NOTICE", "NOTICE"),
    ] {
        fs::copy(root.join(source), book.join("src").join(target))?;
    }
    for name in ["LICENSE", "NOTICE"] {
        fs::write(
            book.join("src").join(format!("{name}.md")),
            format!(
                "# {name}\n\n\u{0060}\u{0060}\u{0060}text\n{}\n\u{0060}\u{0060}\u{0060}\n",
                fs::read_to_string(root.join(name))?
            ),
        )?;
        fs::copy(root.join(name), destination.join(name))?;
    }
    fs::create_dir_all(book.join("src/examples"))?;
    fs::copy(
        root.join("examples/catalogue/src/main.rs"),
        book.join("src/examples/catalogue.rs"),
    )?;
    let provider = super::toml(root.join("crates/battersea-providers/Cargo.toml"))?;
    fs::write(
        book.join("src/provider-features.toml"),
        toml::to_string_pretty(
            &provider
                .get("features")
                .ok_or("Missing provider feature definitions")?,
        )?,
    )?;
    // Only authored guide pages contain this placeholder; contracts remain byte-for-byte copies.
    for name in ["installation.md", "features.md"] {
        let path = book.join("src").join(name);
        fs::write(
            &path,
            fs::read_to_string(&path)?.replace("{{version}}", string(&release, "version")?),
        )?;
    }
    // These source files link to the repository's documentation README; in the book it is a chapter.
    for name in ["overview.md", "CONTRIBUTING.md", "RELEASING.md"] {
        let path = book.join("src").join(name);
        fs::write(
            &path,
            fs::read_to_string(&path)?.replace("(docs/README.md)", "(documentation.md)"),
        )?;
    }
    fs::write(book.join("version.css"),
        format!("body::before{{content:\"Battersea {tag}\";display:block;text-align:center;padding:.4rem;background:var(--quote-bg);font:14px system-ui}}\n"))?;
    let guide = destination.join("guide");
    clean(&guide)?;
    run(
        root,
        "mdbook",
        &[
            "build",
            book.to_str().unwrap(),
            "--dest-dir",
            guide.to_str().unwrap(),
        ],
    )?;

    // Reuse compilation intermediates, but remove old HTML so removed APIs cannot survive a build.
    let cargo_target = root.join("target/documentation/rust-build");
    clean(&cargo_target.join("doc"))?;
    let mut args = vec![
        format!("+{rust}"),
        "doc".into(),
        "--lib".into(),
        "--no-deps".into(),
        "--all-features".into(),
        "--locked".into(),
        "--target-dir".into(),
        cargo_target.to_str().unwrap().into(),
    ];
    let crates = release["crates"]
        .as_object()
        .ok_or("Missing release crate inventory")?;
    for name in crates.keys() {
        args.extend(["-p".into(), name.clone()]);
    }
    if !Command::new("cargo")
        .args(&args)
        .current_dir(root)
        .env("RUSTDOCFLAGS", "-D warnings")
        .status()?
        .success()
    {
        return Err("Rust documentation failed".into());
    }
    clean(&destination.join("rust"))?;
    copy_tree(&cargo_target.join("doc"), &destination.join("rust"))?;
    normalize_rustdoc(root, &destination.join("rust"))?;
    let mut links = String::new();
    for name in crates.keys() {
        write!(
            links,
            "<li><a href=\"{}/index.html\">{}</a></li>",
            name.replace('-', "_"),
            escape(name)
        )?;
    }
    fs::write(destination.join("rust/index.html"), page(&format!("Battersea {tag} Rust API"),
        &format!("<h1>Battersea {tag} Rust API</h1><p><a href=\"../guide/index.html\">Handbook</a> · <a href=\"../typescript/index.html\">TypeScript API</a> · <a href=\"../../versions/index.html\">Versions</a></p><ul>{links}</ul>")))?;
    let schemas = destination.join("schemas");
    clean(&schemas)?;
    for (source, name) in [
        (
            "crates/battersea-flow/schemas/flow.schema.json",
            "flow.schema.json",
        ),
        (
            "crates/battersea-telemetry/schemas/runtime.schema.json",
            "runtime.schema.json",
        ),
        (
            "crates/battersea-pianola/schemas/pianola-roll.schema.json",
            "pianola-roll.schema.json",
        ),
    ] {
        fs::copy(root.join(source), schemas.join(name))?;
    }
    fs::write(destination.join("index.html"), page(&format!("Battersea {tag}"),
        &format!("<h1>Battersea {tag}</h1><ul><li><a href=\"guide/index.html\">Handbook</a></li><li><a href=\"rust/index.html\">Rust API</a></li><li><a href=\"typescript/index.html\">TypeScript API</a></li><li><a href=\"../versions/index.html\">All versions</a></li></ul>")))?;
    let source_commit = output(root, "git", &["rev-parse", "HEAD"])?
        .trim()
        .to_string();
    let dirty = !output(root, "git", &["status", "--porcelain"])?.is_empty();
    let npm = json(root.join("typescript/package.json"))?;
    write_json(
        &destination.join("documentation.json"),
        &json!({
            "version": string(&release, "version")?, "tag": tag, "source_commit": source_commit,
            "source_dirty": dirty, "generators": {
                "rust": rust, "mdbook": mdbook, "typedoc": npm["devDependencies"]["typedoc"]
            }
        }),
    )?;
    let mut versions = BTreeMap::new();
    let prerelease = !tag_version(&tag)?.pre.is_empty();
    versions.insert(tag.clone(), prerelease);
    write_index(&site, &versions, (!prerelease).then_some(tag.as_str()))?;
    println!(
        "Built Rust reference and handbook at {}",
        destination.display()
    );
    println!("Next: (cd typescript && npm run docs), then cargo xtask docs-check");
    Ok(())
}

fn write_index(site: &Path, versions: &BTreeMap<String, bool>, latest: Option<&str>) -> Result<()> {
    fs::create_dir_all(site.join("versions"))?;
    let mut sorted = versions.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|a| std::cmp::Reverse(tag_version(a.0).unwrap()));
    let mut links = String::new();
    for (tag, prerelease) in &sorted {
        write!(
            links,
            "<li><a href=\"../{tag}/index.html\">{tag}</a>{}</li>",
            if **prerelease { " (prerelease)" } else { "" }
        )?;
    }
    fs::write(
        site.join("versions/index.html"),
        page(
            "Battersea documentation versions",
            &format!("<h1>Battersea documentation versions</h1><ul>{links}</ul>"),
        ),
    )?;
    let landing = if let Some(tag) = latest {
        format!("<meta http-equiv=\"refresh\" content=\"0;url={tag}/guide/index.html\"><h1>Battersea documentation</h1><p><a href=\"{tag}/guide/index.html\">Read {tag}</a> · <a href=\"versions/index.html\">All versions</a></p>")
    } else {
        "<h1>Battersea documentation</h1><p><a href=\"versions/index.html\">Browse prerelease documentation</a></p>".into()
    };
    fs::write(
        site.join("index.html"),
        page("Battersea documentation", &landing),
    )?;
    fs::write(site.join(".nojekyll"), "")?;
    write_json(
        &site.join("versions.json"),
        &json!({"latest": latest, "versions":
        sorted.iter().map(|(tag, prerelease)| json!({"tag": tag, "prerelease": prerelease})).collect::<Vec<_>>()}),
    )?;
    Ok(())
}

fn files(path: &Path, result: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            files(&entry.path(), result)?;
        } else if entry.file_type()?.is_file() {
            result.push(entry.path());
        } else {
            return Err(format!("Special file in documentation: {:?}", entry.path()).into());
        }
    }
    Ok(())
}

/// Fix two presentation gaps in the pinned rustdoc output, without changing source or APIs.
fn normalize_rustdoc(root: &Path, directory: &Path) -> Result<()> {
    let lockfile = super::toml(root.join("Cargo.lock"))?;
    let tracing = lockfile["package"]
        .as_array()
        .ok_or("Missing locked packages")?
        .iter()
        .find(|package| package["name"].as_str() == Some("tracing"))
        .and_then(|package| package["version"].as_str())
        .ok_or("Missing tracing version")?;
    let base = Url::parse("https://documentation.invalid/")?;
    let scripts = Selector::parse("script[src]").unwrap();
    let mut paths = Vec::new();
    files(directory, &mut paths)?;
    for path in paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
    {
        let text = fs::read_to_string(path)?;
        // rustdoc inlines these relative links from tracing's blanket implementations, but
        // resolves neither against the external crate. Keep them pointing to the locked source.
        let fixed = text.replace("href=\"dispatcher#setting-the-default-subscriber\"", &format!(
            "href=\"https://docs.rs/tracing/{tracing}/tracing/dispatcher/index.html#setting-the-default-subscriber\""))
            .replace("href=\"super::Subscriber\"", &format!(
                "href=\"https://docs.rs/tracing/{tracing}/tracing/trait.Subscriber.html\""));
        if fixed != text {
            fs::write(path, &fixed)?;
        }
        let page = base.join(&path.strip_prefix(directory)?.to_string_lossy())?;
        let html = Html::parse_document(&fixed);
        for script in html.select(&scripts) {
            let url = page.join(script.value().attr("src").unwrap())?;
            let relative = url.path().trim_start_matches('/');
            // rustdoc references an implementor file even when no crate implements the trait.
            // Supply the empty registry it would have emitted for an empty implementation set.
            if relative.starts_with("trait.impl/") && relative.ends_with(".js") {
                let target = directory.join(relative);
                if !target.exists() {
                    fs::create_dir_all(target.parent().unwrap())?;
                    fs::write(target, "if(window.register_implementors){window.register_implementors({});}else{window.pending_implementors={};}\n")?;
                }
            }
        }
    }
    Ok(())
}

/// Resolve links as a browser at the actual project and version prefix would.
pub fn check(site: &Path) -> Result<()> {
    let site = site.canonicalize()?;
    let index = json(site.join("versions.json"))?;
    for entry in index["versions"]
        .as_array()
        .ok_or("No documentation versions")?
    {
        let tag = string(entry, "tag")?;
        tag_version(tag)?;
        let directory = site.join(tag);
        let metadata = json(directory.join("documentation.json"))?;
        if metadata["tag"] != tag || metadata["version"] != tag.trim_start_matches('v') {
            return Err(format!("Documentation identity mismatch for {tag}").into());
        }
        for required in [
            "index.html",
            "guide/index.html",
            "rust/index.html",
            "rust/battersea_flow/struct.FlowDocument.html",
            "typescript/index.html",
            "schemas/flow.schema.json",
            "schemas/runtime.schema.json",
            "schemas/pianola-roll.schema.json",
            "LICENSE",
            "NOTICE",
        ] {
            if !directory.join(required).is_file() {
                return Err(format!("Missing {tag}/{required}").into());
            }
        }
    }
    let base = Url::parse("https://documentation.invalid/battersea/")?;
    let mut paths = Vec::new();
    files(&site, &mut paths)?;
    let selector = Selector::parse("[href], [src], [id], a[name]").unwrap();
    let mut pages = BTreeMap::new();
    for path in paths
        .iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
    {
        let html = Html::parse_document(&fs::read_to_string(path)?);
        let mut ids = HashSet::new();
        let mut links = Vec::new();
        for element in html.select(&selector) {
            for attribute in ["id", "name"] {
                if let Some(id) = element.value().attr(attribute) {
                    ids.insert(id.to_string());
                }
            }
            for attribute in ["href", "src"] {
                if let Some(link) = element.value().attr(attribute) {
                    links.push(link.to_string());
                }
            }
        }
        pages.insert(path.strip_prefix(&site)?.to_path_buf(), (ids, links));
    }
    let mut errors = Vec::new();
    for (path, (_, links)) in &pages {
        let page_url = base.join(&path.to_string_lossy())?;
        for link in links {
            let target = match page_url.join(link) {
                Ok(target) => target,
                Err(_) => {
                    errors.push(format!("{}: invalid URL {link}", path.display()));
                    continue;
                }
            };
            if target.origin() != base.origin() {
                continue;
            }
            let Some(relative) = target.path().strip_prefix(base.path()) else {
                errors.push(format!(
                    "{}: link escapes project prefix: {link}",
                    path.display()
                ));
                continue;
            };
            let mut relative = PathBuf::from(percent_decode_str(relative).decode_utf8()?.as_ref());
            if relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                errors.push(format!("{}: unsafe encoded path: {link}", path.display()));
                continue;
            }
            if site.join(&relative).is_dir() {
                relative.push("index.html");
            }
            if !site.join(&relative).is_file() {
                errors.push(format!("{}: missing {link}", path.display()));
            } else if let Some(fragment) = target.fragment().filter(|f| !f.is_empty()) {
                // Rustdoc's line ranges are applied by JavaScript to source pages.
                let decoded = percent_decode_str(fragment).decode_utf8()?;
                let first = decoded.split('-').next().unwrap();
                let source_range = relative.starts_with("src")
                    || relative.components().any(|c| c.as_os_str() == "src");
                let valid = pages.get(&relative).is_some_and(|(ids, _)| {
                    ids.contains(fragment)
                        || ids.contains(decoded.as_ref())
                        || (source_range && ids.contains(first))
                });
                if !valid && pages.contains_key(&relative) {
                    errors.push(format!("{}: missing anchor {link}", path.display()));
                }
            }
        }
    }
    if !errors.is_empty() {
        let count = errors.len();
        let mut messages = HashSet::new();
        errors.retain(|error| messages.insert(error.split_once(": ").unwrap().1.to_string()));
        errors.truncate(40);
        return Err(format!("{count} documentation link errors:\n{}", errors.join("\n")).into());
    }
    println!(
        "Checked {} HTML pages under /battersea/ (including links, assets and anchors)",
        pages.len()
    );
    Ok(())
}

fn append_archive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    source: &Path,
    name: &Path,
) -> Result<()> {
    let mut paths = Vec::new();
    files(source, &mut paths)?;
    paths.sort();
    for path in paths {
        let bytes = fs::read(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        builder.append_data(
            &mut header,
            name.join(path.strip_prefix(source)?),
            Cursor::new(bytes),
        )?;
    }
    Ok(())
}

pub fn archive(root: &Path) -> Result<()> {
    let site = root.join(SITE);
    check(&site)?;
    let tag = version(root)?;
    let destination = root.join(format!("target/documentation/battersea-docs-{tag}.tar.gz"));
    let gzip = GzEncoder::new(fs::File::create(&destination)?, Compression::default());
    let mut builder = tar::Builder::new(gzip);
    append_archive(&mut builder, &site.join(&tag), Path::new(&tag))?;
    builder.into_inner()?.finish()?;
    println!("Created {}", destination.display());
    Ok(())
}

fn verify_identity(metadata: &Value, tag: &str, commit: &str) -> Result<()> {
    if metadata["tag"] != tag
        || metadata["version"] != tag.trim_start_matches('v')
        || metadata["source_commit"] != commit
        || metadata["source_dirty"] != false
    {
        return Err(
            format!("Documentation does not match the clean source commit for {tag}").into(),
        );
    }
    Ok(())
}

fn verify_checksum(bytes: &[u8], checksums: &str, asset: &str) -> Result<()> {
    let expected = checksums
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let hash = fields.next()?;
            let name = fields.next()?.trim_start_matches('*');
            (name == asset).then_some(hash)
        })
        .collect::<Vec<_>>();
    let actual = format!("{:x}", Sha256::digest(bytes));
    if expected.len() != 1 || expected[0] != actual {
        return Err(format!("Missing, duplicate or incorrect SHA-256 checksum for {asset}").into());
    }
    Ok(())
}

fn unpack(bytes: &[u8], destination: &Path, tag: &str) -> Result<()> {
    tag_version(tag)?;
    let mut archive = tar::Archive::new(GzDecoder::new(bytes));
    let mut seen = HashSet::new();
    let mut remaining = MAX_ARCHIVE_BYTES;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut components = path.components();
        if components.next() != Some(Component::Normal(tag.as_ref()))
            || components.clone().next().is_none()
            || components.any(|c| !matches!(c, Component::Normal(_)))
            || !entry.header().entry_type().is_file()
            || !seen.insert(path.clone())
        {
            return Err(format!(
                "Unsafe or duplicate documentation archive entry: {}",
                path.display()
            )
            .into());
        }
        remaining = remaining
            .checked_sub(entry.size())
            .ok_or("Documentation archive exceeds size limit")?;
        let target = destination.join(&path);
        fs::create_dir_all(target.parent().unwrap())?;
        let mut out = fs::File::create(target)?;
        std::io::copy(&mut entry, &mut out)?;
    }
    Ok(())
}

fn latest_documented(
    versions: &BTreeMap<String, bool>,
    github_latest: Option<&str>,
) -> Option<String> {
    if let Some(tag) = github_latest.filter(|tag| versions.get(*tag) == Some(&false)) {
        return Some(tag.into());
    }
    versions
        .iter()
        .filter(|(_, prerelease)| !**prerelease)
        .max_by_key(|(tag, _)| tag_version(tag).unwrap())
        .map(|(tag, _)| tag.clone())
}

/// Rebuild the complete deployment from durable release assets, never from a previous Pages deploy.
pub fn published_site(root: &Path) -> Result<()> {
    let release = json(root.join("release.json"))?;
    let repository = string(&release, "repository")?
        .strip_prefix("https://github.com/")
        .ok_or("Expected GitHub repository")?;
    let pages: Value = serde_json::from_str(&output(
        root,
        "gh",
        &[
            "api",
            "--paginate",
            "--slurp",
            &format!("repos/{repository}/releases?per_page=100"),
        ],
    )?)?;
    let site = root.join(PUBLISHED);
    clean(&site)?;
    let downloads = tempfile::tempdir()?;
    let mut versions = BTreeMap::new();
    for item in pages
        .as_array()
        .ok_or("Expected release pages")?
        .iter()
        .flat_map(|p| p.as_array().into_iter().flatten())
    {
        if item["draft"] == true {
            continue;
        }
        let tag = string(item, "tag_name")?;
        if tag_version(tag).is_err() {
            continue;
        }
        let asset = format!("battersea-docs-{tag}.tar.gz");
        let assets = item["assets"].as_array().ok_or("Missing release assets")?;
        if !assets.iter().any(|a| a["name"] == asset) {
            // Releases before documentation delivery have no archive to publish.
            continue;
        }
        let directory = downloads.path().join(tag);
        fs::create_dir_all(&directory)?;
        run(
            root,
            "gh",
            &[
                "release",
                "download",
                tag,
                "--repo",
                repository,
                "--pattern",
                &asset,
                "--pattern",
                "SHA256SUMS",
                "--dir",
                directory.to_str().unwrap(),
            ],
        )?;
        let bytes = fs::read(directory.join(&asset))?;
        verify_checksum(
            &bytes,
            &fs::read_to_string(directory.join("SHA256SUMS"))?,
            &asset,
        )?;
        unpack(&bytes, &site, tag)?;
        let metadata = json(site.join(tag).join("documentation.json"))?;
        let commit = output(
            root,
            "gh",
            &[
                "api",
                &format!("repos/{repository}/commits/{tag}"),
                "--jq",
                ".sha",
            ],
        )?;
        verify_identity(&metadata, tag, commit.trim())?;
        versions.insert(
            tag.to_string(),
            item["prerelease"] == true || !tag_version(tag)?.pre.is_empty(),
        );
    }
    if versions.is_empty() {
        return Err("No published releases contain documentation archives".into());
    }
    // The list is sorted by creation time, which does not determine the latest stable release.
    let latest = Command::new("gh")
        .args([
            "api",
            &format!("repos/{repository}/releases/latest"),
            "--jq",
            ".tag_name",
        ])
        .current_dir(root)
        .output()?;
    let github_latest = latest
        .status
        .success()
        .then(|| String::from_utf8_lossy(&latest.stdout).trim().to_string());
    if !latest.status.success() && versions.values().any(|prerelease| !prerelease) {
        return Err(format!(
            "Cannot determine GitHub's latest stable release: {}",
            String::from_utf8_lossy(&latest.stderr)
        )
        .into());
    }
    let latest = latest_documented(&versions, github_latest.as_deref());
    write_index(&site, &versions, latest.as_deref())?;
    check(&site)?;
    println!("Assembled published documentation at {}", site.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_follows_github_without_promoting_prereleases_or_backports() {
        let versions = BTreeMap::from([
            ("v0.12.1".into(), false),
            ("v0.13.0".into(), false),
            ("v0.14.0-rc.1".into(), true),
        ]);
        assert_eq!(
            latest_documented(&versions, Some("v0.13.0")),
            Some("v0.13.0".into())
        );
        assert_eq!(
            latest_documented(&versions, Some("v0.12.1")),
            Some("v0.12.1".into())
        );
        assert_eq!(
            latest_documented(&versions, Some("v0.14.0-rc.1")),
            Some("v0.13.0".into())
        );
        assert_eq!(
            latest_documented(&versions, Some("v0.15.0")),
            Some("v0.13.0".into())
        );
        assert_eq!(
            latest_documented(&BTreeMap::from([("v1.0.0-rc.1".into(), true)]), None),
            None
        );
        assert!(tag_version("v../bad").is_err());
    }

    #[test]
    fn checksums_require_exactly_one_matching_filename_and_digest() {
        let hash = format!("{:x}", Sha256::digest(b"archive"));
        assert!(
            verify_checksum(b"archive", &format!("{hash}  docs.tar.gz\n"), "docs.tar.gz").is_ok()
        );
        assert!(
            verify_checksum(b"changed", &format!("{hash}  docs.tar.gz"), "docs.tar.gz").is_err()
        );
        assert!(
            verify_checksum(b"archive", &format!("{hash}  other.tar.gz"), "docs.tar.gz").is_err()
        );
        assert!(verify_checksum(
            b"archive",
            &format!("{hash}  docs.tar.gz\n{hash}  docs.tar.gz"),
            "docs.tar.gz"
        )
        .is_err());
    }

    #[test]
    fn archives_round_trip_and_reject_links_and_other_version_roots() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("index.html"), "release contents").unwrap();
        let gzip = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(gzip);
        append_archive(&mut builder, &source, Path::new("v1.0.0")).unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        let target = temp.path().join("target");
        unpack(&bytes, &target, "v1.0.0").unwrap();
        assert_eq!(
            fs::read_to_string(target.join("v1.0.0/index.html")).unwrap(),
            "release contents"
        );
        assert!(unpack(&bytes, &temp.path().join("wrong"), "v2.0.0").is_err());

        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("../../outside").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "v1.0.0/escape", Cursor::new([]))
            .unwrap();
        let bytes = builder.into_inner().unwrap().finish().unwrap();
        assert!(unpack(&bytes, &temp.path().join("symlinks"), "v1.0.0").is_err());
    }

    fn fixture(site: &Path, tag: &str) {
        let version = site.join(tag);
        for name in [
            "index.html",
            "guide/index.html",
            "rust/index.html",
            "rust/battersea_flow/struct.FlowDocument.html",
            "typescript/index.html",
            "schemas/flow.schema.json",
            "schemas/runtime.schema.json",
            "schemas/pianola-roll.schema.json",
            "LICENSE",
            "NOTICE",
        ] {
            let path = version.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "<html><body id=\"present\"></body></html>").unwrap();
        }
        write_json(
            &version.join("documentation.json"),
            &json!({
                "version": tag.trim_start_matches('v'), "tag": tag
            }),
        )
        .unwrap();
    }

    #[test]
    fn site_checks_nested_links_anchors_assets_and_project_prefix() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path(), "v1.0.0");
        write_index(
            temp.path(),
            &BTreeMap::from([("v1.0.0".into(), false)]),
            Some("v1.0.0"),
        )
        .unwrap();
        let guide = temp.path().join("v1.0.0/guide/index.html");
        fs::write(&guide, "<a href=\"../typescript/index.html#present\">API</a><a href=\"../../versions/\">Versions</a><div id=\"impl-Borrow%3CT%3E\"></div><a href=\"#impl-Borrow%3CT%3E\">Rustdoc anchor</a>").unwrap();
        check(temp.path()).unwrap();
        for broken in [
            "../typescript/index.html#absent",
            "../schemas/missing.json",
            "/v1.0.0/guide/index.html",
            "../../../outside",
            "%2e%2e%2f%2e%2e%2foutside",
        ] {
            fs::write(&guide, format!("<a href=\"{broken}\">Broken</a>")).unwrap();
            assert!(check(temp.path()).is_err(), "{broken} must fail");
        }
        fs::write(&guide, "<img src=\"missing.png\">").unwrap();
        assert!(check(temp.path()).is_err());
    }

    #[test]
    fn publishing_a_second_version_preserves_first_version_links() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path(), "v1.0.0");
        fixture(temp.path(), "v1.1.0");
        let versions = BTreeMap::from([("v1.0.0".into(), false), ("v1.1.0".into(), false)]);
        write_index(temp.path(), &versions, Some("v1.1.0")).unwrap();
        check(temp.path()).unwrap();
        let index = fs::read_to_string(temp.path().join("versions/index.html")).unwrap();
        assert!(index.contains("../v1.0.0/index.html"));
        assert!(index.contains("../v1.1.0/index.html"));
    }

    #[test]
    fn publication_refuses_dirty_mismatched_or_stale_source_identity() {
        let metadata = json!({"tag": "v1.0.0", "version": "1.0.0", "source_commit": "abc", "source_dirty": false});
        verify_identity(&metadata, "v1.0.0", "abc").unwrap();
        assert!(verify_identity(&metadata, "v1.1.0", "abc").is_err());
        assert!(verify_identity(&metadata, "v1.0.0", "def").is_err());
        let mut dirty = metadata;
        dirty["source_dirty"] = json!(true);
        assert!(verify_identity(&dirty, "v1.0.0", "abc").is_err());
    }

    #[test]
    fn rustdoc_normalization_supplies_empty_implementors_and_locked_external_links() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.lock"),
            "[[package]]\nname = 'tracing'\nversion = '0.1.44'\n",
        )
        .unwrap();
        let doc = temp.path().join("doc/example");
        fs::create_dir_all(&doc).unwrap();
        fs::write(doc.join("trait.Host.html"), r#"<script src="../trait.impl/example/trait.Host.js"></script><a href="dispatcher#setting-the-default-subscriber">Default</a><a href="super::Subscriber">Subscriber</a>"#).unwrap();
        normalize_rustdoc(temp.path(), &temp.path().join("doc")).unwrap();
        assert!(temp
            .path()
            .join("doc/trait.impl/example/trait.Host.js")
            .is_file());
        let text = fs::read_to_string(doc.join("trait.Host.html")).unwrap();
        assert!(text.contains("https://docs.rs/tracing/0.1.44/tracing/dispatcher/index.html#setting-the-default-subscriber"));
        assert!(text.contains("https://docs.rs/tracing/0.1.44/tracing/trait.Subscriber.html"));
    }
}
