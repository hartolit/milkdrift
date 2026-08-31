#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <authority|retention|runtime|uncertainty|controller|context|peer> [--list]" >&2
  exit 2
fi

shard=$1
mode=${2:-run}
case "$mode" in
  run) list_args=() ;;
  --list) list_args=(--list) ;;
  *) echo "unknown mode: $mode" >&2; exit 2 ;;
esac

files=()
pattern=
test_packages=()
cargo_test_args=()
case "$shard" in
  authority)
    files=(
      crates/authority/src/selection.rs
      crates/authority/src/evaluator.rs
      crates/authority/src/model.rs
      adapters/redb-store/src/peer.rs
    )
    pattern='(Selection.*(matches|is_subset_of)|validate_count|GrantSetEvaluator::evaluate|CapabilityAuthorityScope::is_subset_of|AuthorityBudget::fits_within|within|validate_admission)'
    test_packages=(milkdrift-authority milkdrift-peer-http milkdrift-evidence)
    ;;
  retention)
    files=(
      adapters/redb-store/src/application.rs
    )
    pattern='(commit_application_command|archive_application_command_receipts|archive_oldest_hot_receipts|receipt_accounting_values|ReceiptLocation::status)'
    test_packages=(milkdrift-redb-store milkdrift-evidence)
    ;;
  runtime)
    files=(
      crates/runtime/src/engine/reconciliation.rs
      crates/runtime/src/engine.rs
    )
    pattern='(handle_new_command|replay_if_present|projection_checkpoint_due|plan_revision_adoption|plan_reconciliation_decision|plan_reconciliation_application)'
    test_packages=(milkdrift-runtime)
    cargo_test_args=(--lib --test durable_runtime --test structured_runtime)
    ;;
  uncertainty)
    files=(
      crates/runtime/src/engine/effects.rs
      crates/runtime/src/engine/support.rs
    )
    pattern='(recovery_classification|record_effect_uncertainty)'
    test_packages=(milkdrift-runtime)
    cargo_test_args=(--test structured_runtime)
    ;;
  controller)
    files=(
      crates/control/src/controller.rs
    )
    pattern='(ControllerPolicy::assess|ControllerLifecycleOwner::progress|ControllerLifecycleOwner::assess|bound_outcome)'
    test_packages=(milkdrift-control)
    cargo_test_args=(--lib --test control_service)
    ;;
  context)
    files=(
      crates/runtime/src/context.rs
    )
    pattern='(CausalContextBuilder::build|budget_overflow)'
    test_packages=(milkdrift-runtime)
    cargo_test_args=(--test causal_context)
    ;;
  peer)
    files=(
      adapters/redb-store/src/peer.rs
    )
    pattern='(admit_peer_execution|claim_peer_dispatch|mark_peer_entered|release_peer_claim|mark_peer_uncertain|append_peer_observation|request_peer_cancellation|acknowledge_peer_cancellation|recover_peer_claims|archive_peer_executions|release_active_accounting|validate_record|validate_tombstone)'
    test_packages=(milkdrift-peer-http milkdrift-evidence)
    ;;
  *)
    echo "unknown mutation shard: $shard" >&2
    exit 2
    ;;
esac

output=${CARGO_MUTANTS_OUTPUT:-target/mutation/$shard}
jobs=${CARGO_MUTANTS_JOBS:-2}
mkdir -p "$(dirname "$output")"
arguments=(
  --workspace
  --output "$output"
  --jobs "$jobs"
  --re "$pattern"
  --baseline run
  --build-timeout 180
  --no-shuffle
)
for file in "${files[@]}"; do
  arguments+=(--file "$file")
done
for package in "${test_packages[@]}"; do
  arguments+=(--test-package "$package")
done
for cargo_test_arg in "${cargo_test_args[@]}"; do
  arguments+=(--cargo-test-arg "$cargo_test_arg")
done

if [[ "$mode" == "--list" ]]; then
  exec cargo mutants "${arguments[@]}" "${list_args[@]}"
fi

set +e
cargo mutants "${arguments[@]}"
mutation_status=$?
set -e
if [[ $mutation_status -eq 0 ]]; then
  exit 0
fi
# cargo-mutants 27.1.0 uses status 2 for completed campaigns with missed/timeout
# outcomes. Tool crashes, interruptions, and setup failures use other statuses and
# must never be converted into a successful classified-survivor lane.
if [[ $mutation_status -ne 2 ]]; then
  exit "$mutation_status"
fi
if node scripts/check-mutation-classifications.mjs \
  "$output/mutants.out" .cargo/mutation-classifications.json; then
  exit 0
fi
exit "$mutation_status"
