//! The dispatch state machine for a single drop attempt.
//!
//! The event loop used to inline this decision as a `bool` plus two `if` blocks, which made
//! the interesting cases untestable and hid an ordering bug: the "trigger fired" alert was
//! dispatched *before* checking whether any transaction had actually been prepared, so a
//! drop that never left the machine still reported success.
//!
//! The states are deliberately explicit:
//!
//! ```text
//!                 trigger
//!   Armed ──────────────────────┬──────────────┐
//!     ▲                         │              │
//!     │  abort (no txs)         ▼              ▼
//!     └───────────────────── Aborted        Fired
//! ```
//!
//! `Armed` is the only state in which a trigger may act. `Aborted` returns to `Armed`
//! because a missing batch is a transient condition (gas above the ceiling, a signing
//! failure); the next base-fee update rebuilds it. `Fired` is terminal for the run.

use crate::sniper::SnipeTrigger;

/// What the event loop should do with a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerDecision {
    /// Not something that dispatches (telemetry); ignore.
    Telemetry,
    /// A dispatch trigger arrived while already fired or unarmed; ignore it.
    Ignored,
    /// A dispatch trigger arrived but there is no prepared batch; stay armed.
    AbortedEmpty,
    /// Dispatch the prepared batch.
    Dispatch,
}

/// Tracks whether this run has already dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DispatchState {
    #[default]
    Armed,
    Fired,
}

/// Classifies an incoming trigger against the current state.
///
/// `has_prepared_batch` must reflect the number of *currently valid* pre-signed
/// transactions. A batch is invalidated whenever the gas ceiling is breached, because the
/// pre-signed bytes embed a fee that would now be rejected.
pub fn classify_trigger(
    state: DispatchState,
    trigger: &SnipeTrigger,
    has_prepared_batch: bool,
) -> TriggerDecision {
    if matches!(trigger, SnipeTrigger::BaseFeeUpdated { .. }) {
        return TriggerDecision::Telemetry;
    }
    if state == DispatchState::Fired {
        return TriggerDecision::Ignored;
    }
    if !has_prepared_batch {
        return TriggerDecision::AbortedEmpty;
    }
    TriggerDecision::Dispatch
}

/// Applies a decision to the state, returning the next state.
pub fn apply_decision(state: DispatchState, decision: TriggerDecision) -> DispatchState {
    match (state, decision) {
        (DispatchState::Armed, TriggerDecision::Dispatch) => DispatchState::Fired,
        // An aborted attempt stays armed so the next base-fee update can rebuild the batch.
        (DispatchState::Armed, TriggerDecision::AbortedEmpty) => DispatchState::Armed,
        (current, _) => current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countdown() -> SnipeTrigger {
        SnipeTrigger::CountdownReached { target_unix: 1_700_000_000 }
    }

    fn base_fee() -> SnipeTrigger {
        SnipeTrigger::BaseFeeUpdated { base_fee_wei: 10_000_000_000 }
    }

    #[test]
    fn base_fee_updates_never_dispatch() {
        // A base-fee update is telemetry: it refreshes the pre-signed batch, it does not fire.
        assert_eq!(
            classify_trigger(DispatchState::Armed, &base_fee(), true),
            TriggerDecision::Telemetry
        );
        assert_eq!(
            classify_trigger(DispatchState::Armed, &base_fee(), false),
            TriggerDecision::Telemetry
        );
    }

    #[test]
    fn armed_with_batch_dispatches() {
        let d = classify_trigger(DispatchState::Armed, &countdown(), true);
        assert_eq!(d, TriggerDecision::Dispatch);
        assert_eq!(apply_decision(DispatchState::Armed, d), DispatchState::Fired);
    }

    #[test]
    fn armed_without_batch_aborts_and_stays_armed() {
        let d = classify_trigger(DispatchState::Armed, &countdown(), false);
        assert_eq!(d, TriggerDecision::AbortedEmpty);
        // Crucially, not Fired: the batch can be rebuilt by the next base-fee update.
        assert_eq!(apply_decision(DispatchState::Armed, d), DispatchState::Armed);
    }

    #[test]
    fn fired_ignores_further_triggers() {
        let d = classify_trigger(DispatchState::Fired, &countdown(), true);
        assert_eq!(d, TriggerDecision::Ignored);
        assert_eq!(apply_decision(DispatchState::Fired, d), DispatchState::Fired);
    }

    #[test]
    fn fired_state_survives_an_abort_decision() {
        // Defensive: an explicit abort must never un-fire a run that already dispatched,
        // otherwise a second mint could be sent from the same session.
        assert_eq!(
            apply_decision(DispatchState::Fired, TriggerDecision::AbortedEmpty),
            DispatchState::Fired
        );
    }

    #[test]
    fn abort_then_recover_is_a_valid_sequence() {
        // Sequence: trigger too early (no batch) -> base fee settles -> trigger again.
        let mut state = DispatchState::Armed;

        let d = classify_trigger(state, &countdown(), false);
        state = apply_decision(state, d);
        assert_eq!(state, DispatchState::Armed);

        // Base-fee update rebuilt the batch.
        assert_eq!(
            classify_trigger(state, &base_fee(), true),
            TriggerDecision::Telemetry
        );

        let d = classify_trigger(state, &countdown(), true);
        assert_eq!(d, TriggerDecision::Dispatch);
        state = apply_decision(state, d);
        assert_eq!(state, DispatchState::Fired);
    }
}
