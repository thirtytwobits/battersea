//! Contract cases from contracts/recovery.md; the store can fail after applying a write.
use battersea_runtime::recovery::*;
use battersea_runtime::RunIdentity;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Store {
    value: Mutex<Option<JournalRecord>>,
    fail_after_write: Mutex<bool>,
}
impl JournalStore for Store {
    fn load(&self, _: &str) -> Result<Option<JournalRecord>, RecoveryError> {
        Ok(self.value.lock().unwrap().clone())
    }
    fn compare_exchange(
        &self,
        _: &str,
        expected: Option<u64>,
        next: &JournalRecord,
    ) -> Result<(), RecoveryError> {
        let mut slot = self.value.lock().unwrap();
        if slot.as_ref().map(|value| value.revision) != expected {
            return Err(RecoveryError::Conflict);
        }
        *slot = Some(next.clone());
        if std::mem::take(&mut *self.fail_after_write.lock().unwrap()) {
            return Err(RecoveryError::Storage("lost acknowledgement".into()));
        }
        Ok(())
    }
}
fn identity() -> RunIdentity {
    RunIdentity {
        run_id: "fixture-run".into(),
        flow_key: "fixture-flow".into(),
        graph_revision: "graph-revision".into(),
        catalogue_revision: "catalogue-revision".into(),
        configuration_revision: "configuration-revision".into(),
    }
}
fn state() -> VersionedState {
    VersionedState {
        version: 1,
        value: json!({"next_node": "second"}),
    }
}
fn open(store: Arc<Store>) -> Journal {
    Journal::accept(store, identity(), state()).unwrap()
}

#[test]
fn inspection_does_not_create_a_record() {
    let store = Store::default();
    assert!(store.load(&identity().run_id).unwrap().is_none());
    assert!(store.value.lock().unwrap().is_none());
}

#[test]
fn an_intent_blocks_resume_until_explicit_resolution() {
    let store = Arc::new(Store::default());
    let mut journal = open(store.clone());
    let before = journal.record().checkpoint.clone();
    let effect = journal
        .begin_effect("provider", "generate", json!({"model":"fixture"}))
        .unwrap();
    assert!(matches!(
        journal.require_resumable(),
        Err(RecoveryError::UnresolvedEffects(_))
    ));
    assert_eq!(journal.record().checkpoint, before);
    let mut resumed = Journal::open(store, &identity().run_id).unwrap();
    resumed
        .resolve_not_applied(
            &effect,
            "operator",
            "Provider confirmed no request was accepted.",
        )
        .unwrap();
    resumed.require_resumable().unwrap();
    assert_eq!(resumed.record().checkpoint, before);
    assert!(!resumed.record().resolutions.is_empty());
}

#[test]
fn result_and_checkpoint_are_published_together() {
    let store = Arc::new(Store::default());
    let mut journal = open(store.clone());
    let effect = journal
        .begin_effect("provider", "generate", json!({}))
        .unwrap();
    let result = json!({"provider_job":"retained-id"});
    let next = VersionedState {
        version: 1,
        value: json!({"next_node":"writer", "output":"retained"}),
    };
    journal
        .checkpoint(next.clone(), vec![(effect.clone(), result.clone())])
        .unwrap();
    let reopened = Journal::open(store, &identity().run_id).unwrap();
    reopened.require_resumable().unwrap();
    assert_eq!(reopened.record().checkpoint, next);
    assert_eq!(reopened.record().effects[&effect].result(), Some(&result));
}

#[test]
fn lost_write_acknowledgement_cannot_authorise_repeating_an_effect() {
    let store = Arc::new(Store::default());
    let mut journal = open(store.clone());
    *store.fail_after_write.lock().unwrap() = true;
    assert!(journal
        .begin_effect("provider", "generate", json!({}))
        .is_err());
    assert!(matches!(
        journal.begin_effect("provider", "generate", json!({})),
        Err(RecoveryError::Poisoned)
    ));
    let reopened = Journal::open(store, &identity().run_id).unwrap();
    assert!(matches!(
        reopened.require_resumable(),
        Err(RecoveryError::UnresolvedEffects(_))
    ));
}

#[test]
fn concurrent_recovery_uses_revision_fencing() {
    let store = Arc::new(Store::default());
    let mut first = open(store.clone());
    let mut second = Journal::open(store, &identity().run_id).unwrap();
    first.claim_resume(first.record().revision).unwrap();
    assert!(matches!(
        second.claim_resume(second.record().revision),
        Err(RecoveryError::Conflict)
    ));
    assert!(matches!(
        second.begin_effect("provider", "generate", json!({})),
        Err(RecoveryError::Poisoned)
    ));
}

#[test]
fn host_commit_requires_reconciliation_after_lost_acknowledgement() {
    let store = Arc::new(Store::default());
    let mut journal = open(store.clone());
    journal.execution_complete(state()).unwrap();
    journal
        .begin_host_commit(json!({"destination":"history"}))
        .unwrap();
    let reopened = Journal::open(store.clone(), &identity().run_id).unwrap();
    assert!(matches!(
        reopened.require_resumable(),
        Err(RecoveryError::CommitPending)
    ));
    *store.fail_after_write.lock().unwrap() = true;
    assert!(journal
        .host_committed(json!({"record":"history-result"}))
        .is_err());
    let reopened = Journal::open(store, &identity().run_id).unwrap();
    assert!(matches!(
        reopened.record().phase,
        RecoveryPhase::Committed { .. }
    ));
    assert!(matches!(
        reopened.require_resumable(),
        Err(RecoveryError::Terminal)
    ));
}

#[test]
fn unknown_versions_and_changed_pins_are_rejected_without_writes() {
    let store = Arc::new(Store::default());
    let journal = open(store.clone());
    let mut other = identity();
    other.configuration_revision.push_str("-changed");
    assert!(journal.validate_identity(&other).is_err());
    let original_revision = journal.record().revision;
    store.value.lock().unwrap().as_mut().unwrap().version += 1;
    assert!(matches!(
        Journal::open(store.clone(), &identity().run_id),
        Err(RecoveryError::Incompatible(_))
    ));
    assert_eq!(
        store.value.lock().unwrap().as_ref().unwrap().revision,
        original_revision
    );
}

#[test]
fn completed_effects_and_terminal_outcomes_cannot_be_replaced() {
    let store = Arc::new(Store::default());
    let mut journal = open(store);
    let effect = journal
        .begin_effect("provider", "generate", json!({}))
        .unwrap();
    journal
        .checkpoint(
            state(),
            vec![(effect.clone(), json!({"output":"retained"}))],
        )
        .unwrap();
    assert!(journal
        .resolve_not_applied(&effect, "operator", "Cannot change known result")
        .is_err());
    journal
        .abandon("operator", "Discard remaining work")
        .unwrap();
    assert!(matches!(
        journal.require_resumable(),
        Err(RecoveryError::Terminal)
    ));
    assert!(journal.execution_complete(state()).is_err());
}
