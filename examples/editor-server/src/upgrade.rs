//! The example host owns revision fencing and recoverable document replacement.
use crate::{io, key, runtime::codes, App, Error};
use axum::{
    extract::{Path, State},
    Json,
};
use battersea_flow::{
    document::serialize_canonical_document,
    execution::{upgrade_v1_document, upgrade_v2_document},
    FlowDocument, FlowExecutionLimits,
};
use battersea_runtime::ExecutionError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::Path as FsPath,
};

#[derive(Serialize)]
pub struct Plan {
    pub revision: String,
    pub document: FlowDocument,
}
#[derive(Deserialize)]
pub struct Request {
    pub expected_revision: String,
}
#[derive(Serialize)]
pub struct Receipt {
    pub flow: FlowDocument,
    pub original: String,
    pub source_revision: String,
}

fn prepare(app: &App, name: &str) -> Result<(Plan, Vec<u8>), Error> {
    let source = fs::read(
        app.runtime
            .root
            .join("flows")
            .join(format!("{}.json", key(name)?)),
    )
    .map_err(io)?;
    let limits = FlowExecutionLimits::default();
    let invalid = |error: &dyn std::fmt::Display| {
        Error::invalid_request(codes::INVALID_DOCUMENT, error.to_string())
    };
    let source_text = std::str::from_utf8(&source).map_err(|error| invalid(&error))?;
    let version = serde_json::from_str::<serde_json::Value>(source_text)
        .map_err(|error| invalid(&error))?["version"]
        .as_u64();
    let document = if version == Some(2) {
        upgrade_v2_document(source_text, app.runtime.catalog.definitions())
    } else {
        upgrade_v1_document(
            source_text,
            app.runtime.catalog.definitions(),
            limits.clone(),
        )
    }
    .map_err(|message| Error::invalid_request(codes::UPGRADE_REFUSED, message))?;
    if document.flow_key != name {
        return Err(Error::invalid_request(
            codes::FLOW_KEY_MISMATCH,
            "Flow key does not match its stored path.",
        ));
    }
    let document = battersea_flow::document::canonicalize_document(&document);
    let validation = app.runtime.catalog.validate(&document);
    if !validation.valid {
        return Err(Error::invalid_request(
            codes::INVALID_FLOW,
            format!("{:?}", validation.issues),
        ));
    }
    // The fence pins the bytes, catalogue and selected policy, not just the format number.
    let definitions: BTreeMap<_, _> = app.runtime.catalog.definitions().iter().collect();
    let revision = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(source.as_slice(), definitions, limits)).map_err(io)?)
    );
    Ok((Plan { revision, document }, source))
}

/// The upgrade of the stored flow `name`, without applying it.
pub fn plan(app: &App, name: &str) -> Result<Plan, Error> {
    let _guard = app.writes.lock().unwrap();
    Ok(prepare(app, name)?.0)
}

pub async fn inspect(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<Plan>, Error> {
    Ok(Json(plan(&app, &name)?))
}

fn write_new(path: &FsPath, bytes: &[u8]) -> Result<(), Error> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn sync_dir(path: &FsPath) -> Result<(), Error> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(io)
}

pub async fn apply(
    State(app): State<App>,
    Path(name): Path<String>,
    Json(request): Json<Request>,
) -> Result<Json<Receipt>, Error> {
    Ok(Json(apply_at(&app, &name, &request.expected_revision)?))
}

/// Upgrades the stored flow `name` when its source revision is `expected_revision`, retaining
/// its original.
pub fn apply_at(app: &App, name: &str, expected_revision: &str) -> Result<Receipt, Error> {
    let _guard = app.writes.lock().unwrap();
    let (plan, original) = prepare(app, name)?;
    if plan.revision != expected_revision {
        return Err(Error::invalid_request(
            codes::STALE_UPGRADE_REVISION,
            "The flow or catalogue changed; inspect the upgrade again.",
        ));
    }
    let upgraded = serialize_canonical_document(&plan.document).map_err(io)?;
    let backups = app.runtime.root.join("upgrades");
    fs::create_dir_all(&backups).map_err(io)?;
    sync_dir(&app.runtime.root)?;
    let id = uuid::Uuid::new_v4().to_string();
    let backup = backups.join(&id);
    fs::create_dir(&backup).map_err(io)?;
    write_new(&backup.join("original.json"), &original)?;
    write_new(
        &backup.join("receipt.json"),
        &serde_json::to_vec_pretty(&serde_json::json!({
            "flow_key": name, "source_revision": plan.revision,
            "target_sha256": format!("{:x}", Sha256::digest(upgraded.as_bytes())),
        }))
        .map_err(io)?,
    )?;
    sync_dir(&backup)?;
    sync_dir(&backups)?;
    let path = app.runtime.root.join("flows").join(format!("{name}.json"));
    let pending = path.with_extension(format!("{id}.tmp"));
    write_new(&pending, upgraded.as_bytes())?;
    fs::rename(&pending, &path).map_err(io)?;
    sync_dir(path.parent().unwrap())?;
    Ok(Receipt {
        flow: plan.document,
        original: format!("upgrades/{id}/original.json"),
        source_revision: plan.revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{activate, initialise, list, read, runtime::Application, save, Activation};
    use serde_json::{json, Value};
    use std::{collections::HashMap, sync::Arc};

    fn fixture() -> (tempfile::TempDir, App, Vec<u8>) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        initialise(&root).unwrap();
        let mut old: Value =
            serde_json::from_str(include_str!("../fixtures/example.json")).unwrap();
        old["version"] = json!(1);
        old.as_object_mut().unwrap().remove("execution");
        let bytes = serde_json::to_vec_pretty(&old).unwrap();
        fs::write(root.join("flows/example.json"), &bytes).unwrap();
        let app = App {
            runtime: Arc::new(Application::new(root).unwrap()),
            runs: Arc::default(),
            writes: Arc::default(),
        };
        (temp, app, bytes)
    }

    #[tokio::test]
    async fn v2_inspection_preserves_bytes_and_explicit_upgrade_retains_the_original() {
        let (_temp, app, _) = fixture();
        let path = app.runtime.root.join("flows/example.json");
        let mut old: Value =
            serde_json::from_str(include_str!("../fixtures/example.json")).unwrap();
        old["version"] = json!(2);
        let original = serde_json::to_vec_pretty(&old).unwrap();
        fs::write(&path, &original).unwrap();
        let (plan, inspected) = prepare(&app, "example").unwrap();
        assert_eq!(inspected, original);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(plan.document.version, 3);
        let receipt = apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(
            fs::read(app.runtime.root.join(receipt.original)).unwrap(),
            original
        );
        assert_eq!(read(&app, "example").unwrap().version, 3);
    }

    #[tokio::test]
    async fn reads_and_upgrade_inspection_preserve_v1_until_explicit_fenced_replacement() {
        let (_temp, app, original) = fixture();
        let root = &app.runtime.root;
        let path = root.join("flows/example.json");
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(read(&app, "example").is_err());
        let entries = list(State(app.clone())).await.unwrap().0;
        assert!(entries[0]["unavailable_reason"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(activate(
            State(app.clone()),
            Json(Activation {
                flow_key: "example".into(),
                node_id: "text".into(),
                parameters: HashMap::new()
            })
        )
        .await
        .is_err());
        let plan = inspect(State(app.clone()), Path("example".into()))
            .await
            .unwrap()
            .0;
        // A normal editor save must not substitute for the explicit upgrade command.
        assert!(save(State(app.clone()), Json(plan.document.clone()))
            .await
            .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), before);
        assert!(!root.join("upgrades").exists());
        assert_eq!(fs::read_dir(root.join("runs")).unwrap().count(), 0);

        let receipt = apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(fs::read(root.join(&receipt.original)).unwrap(), original);
        assert_eq!(read(&app, "example").unwrap(), receipt.flow);
        let accepted = activate(
            State(app.clone()),
            Json(Activation {
                flow_key: "example".into(),
                node_id: "text".into(),
                parameters: HashMap::from([("text".into(), json!("upgraded"))]),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(root
            .join(format!("runs/{}.json", accepted["id"].as_str().unwrap()))
            .exists());
    }

    #[tokio::test]
    async fn stale_revision_and_invalid_graph_fail_before_creating_upgrade_files() {
        let (_temp, app, original) = fixture();
        let path = app.runtime.root.join("flows/example.json");
        let plan = inspect(State(app.clone()), Path("example".into()))
            .await
            .unwrap()
            .0;
        let mut changed: Value = serde_json::from_slice(&original).unwrap();
        changed["title"] = json!("Edited after inspection");
        let changed_bytes = serde_json::to_vec(&changed).unwrap();
        fs::write(&path, &changed_bytes).unwrap();
        assert!(apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision
            })
        )
        .await
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), changed_bytes);
        assert!(!app.runtime.root.join("upgrades").exists());
        changed["nodes"][0]["definition_name"] = json!("missing.custom.node");
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        let invalid = fs::read(&path).unwrap();
        assert!(inspect(State(app.clone()), Path("example".into()))
            .await
            .is_err());
        assert_eq!(fs::read(path).unwrap(), invalid);
        assert!(!app.runtime.root.join("upgrades").exists());
    }

    #[tokio::test]
    async fn backup_failure_preserves_original_and_a_revision_can_only_be_applied_once() {
        let (_temp, app, original) = fixture();
        let plan = inspect(State(app.clone()), Path("example".into()))
            .await
            .unwrap()
            .0;
        let blocker = app.runtime.root.join("upgrades");
        fs::write(&blocker, b"not a directory").unwrap();
        assert!(apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision.clone()
            })
        )
        .await
        .is_err());
        assert_eq!(
            fs::read(app.runtime.root.join("flows/example.json")).unwrap(),
            original
        );
        fs::remove_file(blocker).unwrap();
        let _ = apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision.clone(),
            }),
        )
        .await
        .unwrap();
        let committed = fs::read(app.runtime.root.join("flows/example.json")).unwrap();
        assert!(apply(
            State(app.clone()),
            Path("example".into()),
            Json(Request {
                expected_revision: plan.revision
            })
        )
        .await
        .is_err());
        assert_eq!(
            fs::read(app.runtime.root.join("flows/example.json")).unwrap(),
            committed
        );
        assert_eq!(
            fs::read_dir(app.runtime.root.join("upgrades"))
                .unwrap()
                .count(),
            1
        );
    }
}
