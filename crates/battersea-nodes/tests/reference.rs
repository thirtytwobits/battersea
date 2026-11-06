//! Distinguishing cases from contracts/kits.md for the generic kit, on the reference host.
use battersea_kit::conformance::{Divergence, Fixture};
use battersea_kit::{ManualClock, ReferenceHost, ReferenceState};
use battersea_nodes::conformance::{self, register_probe_handlers, ReferenceFixture};
use battersea_runtime::{ExecutionEvent, RegistryBuilder, SchedulerState};
use std::sync::Arc;

#[test]
fn the_generic_kit_passes_its_name_checks() {
    battersea_nodes::KIT.check().unwrap();
    assert!(battersea_nodes::KIT.is_generic());
}

#[test]
fn model_facing_text_is_keyed_by_formatter_block() {
    let text = battersea_nodes::KIT.model_facing_text().unwrap();
    for block in [
        "concatenate.markdown",
        "concatenate.xml",
        "concatenate.plain",
    ] {
        assert!(
            text.contains_key(&format!("formatter_blocks.{block}")),
            "{block} missing from {:?}",
            text.keys().collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn the_suite_passes_on_the_reference_host() {
    conformance::run(&ReferenceFixture).await.unwrap();
}

/// A host that registers the generic handlers but no logic evaluator.
struct NoLogic;
impl Fixture for NoLogic {
    type Host = ReferenceHost<()>;
    fn host(&self, clock: Arc<ManualClock>) -> Self::Host {
        let mut builder = RegistryBuilder::default();
        battersea_nodes::register_handlers(&mut builder).unwrap();
        register_probe_handlers(&mut builder).unwrap();
        ReferenceHost::new(builder.build(), clock)
    }
    fn state(&self, host: &Self::Host, scheduler: SchedulerState) -> ReferenceState<()> {
        host.state(scheduler, ())
    }
    fn events(&self, host: &Self::Host, run_id: &str) -> Vec<ExecutionEvent> {
        host.run_events(run_id)
    }
}

#[tokio::test]
async fn a_host_without_logic_evaluation_fails_with_the_diverging_case_named() {
    let failure = conformance::run(&NoLogic).await.unwrap_err();
    assert_eq!(failure.case, "and-gate", "{failure}");
    assert!(
        matches!(*failure.divergence, Divergence::Outcome { .. }),
        "{failure}"
    );
}
