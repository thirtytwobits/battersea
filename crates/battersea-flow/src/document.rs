//! Version-first document inspection, canonicalisation and explicit upgrades.
use crate::FlowDocument;
use anyhow::{bail, ensure, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const FLOW_DOCUMENT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq)]
pub enum DocumentInspection {
    Supported(Box<FlowDocument>),
    Incompatible {
        version: u64,
        flow_key: String,
        title: String,
        raw: Value,
    },
}

/// Inspect the version before interpreting executable fields. Never rewrites the input.
pub fn inspect_document(source: &str) -> Result<DocumentInspection> {
    let raw: Value = serde_json::from_str(source)?;
    let version = raw
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("A flow requires an unsigned document version"))?;
    if version != u64::from(FLOW_DOCUMENT_VERSION) {
        return Ok(DocumentInspection::Incompatible {
            version,
            flow_key: raw
                .get("flow_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            title: raw
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            raw,
        });
    }
    Ok(DocumentInspection::Supported(serde_json::from_value(raw)?))
}

pub fn load_document(source: &str) -> Result<FlowDocument> {
    match inspect_document(source)? {
        DocumentInspection::Supported(document) => Ok(*document),
        DocumentInspection::Incompatible { version, .. } => bail!("Unsupported flow document version {version}; supported version is {FLOW_DOCUMENT_VERSION}"),
    }
}

pub fn canonicalize_document(flow: &FlowDocument) -> FlowDocument {
    let mut canonical = flow.clone();
    canonical
        .nodes
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical
        .edges
        .sort_by(|left, right| left.id.cmp(&right.id));
    canonical
}

#[derive(Serialize)]
#[serde(untagged)]
enum CanonicalValue {
    Object(BTreeMap<String, CanonicalValue>),
    Array(Vec<CanonicalValue>),
    Scalar(Value),
}
impl From<Value> for CanonicalValue {
    fn from(value: Value) -> Self {
        match value {
            Value::Object(values) => Self::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into()))
                    .collect(),
            ),
            Value::Array(values) => Self::Array(values.into_iter().map(Into::into).collect()),
            value => Self::Scalar(value),
        }
    }
}

pub fn serialize_canonical_document(flow: &FlowDocument) -> Result<String> {
    ensure!(
        flow.version == FLOW_DOCUMENT_VERSION,
        "Unsupported flow document version {}",
        flow.version
    );
    let value = CanonicalValue::from(serde_json::to_value(canonicalize_document(flow))?);
    Ok(serde_json::to_string_pretty(&value)?)
}

type Upgrade = dyn Fn(Value) -> Result<Value> + Send + Sync;
/// Explicit format upgrades. Registration is separate from loading and inspection.
#[derive(Default)]
pub struct Upgrades {
    steps: HashMap<u64, (u64, Box<Upgrade>)>,
}
impl Upgrades {
    pub fn register(
        &mut self,
        from: u64,
        to: u64,
        upgrade: impl Fn(Value) -> Result<Value> + Send + Sync + 'static,
    ) -> Result<()> {
        ensure!(from != to, "An upgrade must change the version");
        ensure!(
            !self.steps.contains_key(&from),
            "An upgrade from version {from} is already registered"
        );
        self.steps.insert(from, (to, Box::new(upgrade)));
        Ok(())
    }
    /// Produce upgraded bytes only when explicitly called. The caller owns persistence.
    pub fn to_supported_version(&self, source: &str) -> Result<String> {
        if matches!(inspect_document(source)?, DocumentInspection::Supported(_)) {
            return Ok(source.into());
        }
        let mut raw: Value = serde_json::from_str(source)?;
        let mut visited = HashSet::new();
        loop {
            let version = raw
                .get("version")
                .and_then(Value::as_u64)
                .ok_or_else(|| anyhow::anyhow!("An upgrade must produce a document version"))?;
            if version == u64::from(FLOW_DOCUMENT_VERSION) {
                let flow: FlowDocument = serde_json::from_value(raw)?;
                return serialize_canonical_document(&flow);
            }
            ensure!(
                visited.insert(version),
                "Flow upgrade cycle at version {version}"
            );
            let (target, upgrade) = self.steps.get(&version).ok_or_else(|| {
                anyhow::anyhow!("No explicit flow upgrade from version {version} is registered")
            })?;
            raw = upgrade(raw)?;
            ensure!(
                raw.get("version").and_then(Value::as_u64) == Some(*target),
                "An upgrade must produce its registered target version {target}"
            );
        }
    }
}

impl From<FlowDocument> for DocumentInspection {
    fn from(document: FlowDocument) -> Self {
        if document.version == FLOW_DOCUMENT_VERSION {
            Self::Supported(Box::new(document))
        } else {
            Self::Incompatible {
                version: u64::from(document.version),
                flow_key: document.flow_key.clone(),
                title: document.title.clone(),
                raw: serde_json::to_value(document).expect("FlowDocument is JSON-serialisable"),
            }
        }
    }
}
impl DocumentInspection {
    pub fn supported(&self) -> Result<&FlowDocument> {
        match self {
            Self::Supported(document) => {
                ensure!(document.version == FLOW_DOCUMENT_VERSION,
                    "Unsupported flow document version {}", document.version);
                Ok(document)
            },
            Self::Incompatible { version, .. } => bail!("Unsupported flow document version {version}; supported version is {FLOW_DOCUMENT_VERSION}"),
        }
    }
    pub fn flow_key(&self) -> &str {
        match self {
            Self::Supported(document) => &document.flow_key,
            Self::Incompatible { flow_key, .. } => flow_key,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Self::Supported(document) => &document.title,
            Self::Incompatible { title, .. } => title,
        }
    }
}
