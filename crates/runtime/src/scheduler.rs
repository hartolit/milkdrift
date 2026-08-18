use std::{
    cmp::Ordering,
    collections::{BTreeMap, VecDeque},
};

use milkdrift_blueprint::{
    BindingSource, Comparison, Condition, ConditionOperand, PathSegment, PathSelector,
};
use milkdrift_capability::{
    BoundedJson, ErrorClass, IdempotencyBehavior, IdempotencyKey, OperationId, SideEffectClass,
    canonical_json_bytes,
};
use milkdrift_persistence::{MAX_PAGE_SIZE, RunnableIndexEntry};
use milkdrift_workspace::{BranchId, RunId};
use serde_json::{Number, Value};

use crate::RuntimeError;

/// Hard scheduler admission limits. Every value is non-zero.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerLimits {
    global: u32,
    per_run: u32,
    per_branch: u32,
    default_capability_class: u32,
    capability_classes: BTreeMap<OperationId, u32>,
}

impl SchedulerLimits {
    /// Constructs bounded global, run, branch, and default capability-class limits.
    pub fn new(
        global: u32,
        per_run: u32,
        per_branch: u32,
        default_capability_class: u32,
    ) -> Result<Self, RuntimeError> {
        if [global, per_run, per_branch, default_capability_class]
            .into_iter()
            .any(|value| value == 0)
        {
            return Err(RuntimeError::Scheduling(
                "scheduler concurrency limits must be non-zero".to_owned(),
            ));
        }
        if global > MAX_PAGE_SIZE {
            return Err(RuntimeError::Scheduling(format!(
                "global scheduler concurrency cannot exceed the durable active-lease page bound {MAX_PAGE_SIZE}"
            )));
        }
        if per_run > global || per_branch > per_run {
            return Err(RuntimeError::Scheduling(
                "branch concurrency cannot exceed run concurrency, and run cannot exceed global"
                    .to_owned(),
            ));
        }
        Ok(Self {
            global,
            per_run,
            per_branch,
            default_capability_class,
            capability_classes: BTreeMap::new(),
        })
    }

    /// Adds an exact operation-class limit.
    pub fn with_capability_class(
        mut self,
        operation: OperationId,
        maximum: u32,
    ) -> Result<Self, RuntimeError> {
        if maximum == 0 || maximum > self.global {
            return Err(RuntimeError::Scheduling(
                "a capability-class limit must be between one and the global limit".to_owned(),
            ));
        }
        self.capability_classes.insert(operation, maximum);
        Ok(self)
    }

    /// Returns whether the exact usage snapshot may admit one dispatch.
    #[must_use]
    pub fn allows(&self, request: &AdmissionRequest, usage: &AdmissionUsage) -> bool {
        let run_count = usage.runs.get(&request.run).copied().unwrap_or(0);
        let branch_count = request
            .branch
            .as_ref()
            .and_then(|branch| usage.branches.get(&(request.run.clone(), branch.clone())))
            .copied()
            .unwrap_or(0);
        let capability_count = usage
            .capability_classes
            .get(&request.operation)
            .copied()
            .unwrap_or(0);
        let capability_limit = self
            .capability_classes
            .get(&request.operation)
            .copied()
            .unwrap_or(self.default_capability_class);
        usage.global < self.global
            && run_count < self.per_run
            && branch_count < self.per_branch
            && capability_count < capability_limit
    }

    /// Global concurrent-dispatch limit.
    #[must_use]
    pub const fn global(&self) -> u32 {
        self.global
    }

    /// Per-run concurrent-dispatch limit.
    #[must_use]
    pub const fn per_run(&self) -> u32 {
        self.per_run
    }

    /// Per-branch concurrent-dispatch limit.
    #[must_use]
    pub const fn per_branch(&self) -> u32 {
        self.per_branch
    }

    /// Default exact-operation concurrency limit.
    #[must_use]
    pub const fn default_capability_class(&self) -> u32 {
        self.default_capability_class
    }

    /// Exact operation-specific concurrency overrides.
    #[must_use]
    pub const fn capability_classes(&self) -> &BTreeMap<OperationId, u32> {
        &self.capability_classes
    }
}

/// Exact admission subject for one proposed dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionRequest {
    /// Owning run.
    pub run: RunId,
    /// Structured branch, when branch-local.
    pub branch: Option<BranchId>,
    /// Exact operation/capability class.
    pub operation: OperationId,
}

/// Counts derived from active durable leases, never caller-maintained truth.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AdmissionUsage {
    /// Total active durable leases.
    pub global: u32,
    /// Active leases by run.
    pub runs: BTreeMap<RunId, u32>,
    /// Active leases by run and branch.
    pub branches: BTreeMap<(RunId, BranchId), u32>,
    /// Active leases by exact operation class.
    pub capability_classes: BTreeMap<OperationId, u32>,
}

/// Deterministically interleaves runnable entries by run so one run cannot monopolize a page.
#[must_use]
pub fn select_fair_runnable(
    entries: impl IntoIterator<Item = RunnableIndexEntry>,
    maximum: usize,
) -> Vec<RunnableIndexEntry> {
    let mut by_run: BTreeMap<RunId, Vec<RunnableIndexEntry>> = BTreeMap::new();
    for entry in entries {
        by_run.entry(entry.run.clone()).or_default().push(entry);
    }
    for run_entries in by_run.values_mut() {
        run_entries.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.eligible_at.cmp(&right.eligible_at))
                .then_with(|| left.execution.cmp(&right.execution))
        });
    }
    let mut queues: VecDeque<_> = by_run
        .into_iter()
        .map(|(run, entries)| (run, VecDeque::from(entries)))
        .collect();
    let mut selected = Vec::with_capacity(maximum.min(queues.len()));
    while selected.len() < maximum && !queues.is_empty() {
        let Some((run, mut entries)) = queues.pop_front() else {
            break;
        };
        if let Some(entry) = entries.pop_front() {
            selected.push(entry);
        }
        if !entries.is_empty() {
            queues.push_back((run, entries));
        }
    }
    selected
}

/// Retry bounds and conservative safety classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    maximum_attempts: u32,
    retryable_errors: Vec<ErrorClass>,
    initial_backoff_ms: u64,
    maximum_backoff_ms: u64,
    maximum_recorded_jitter_ms: u64,
}

impl RetryPolicy {
    /// Constructs a bounded retry policy; maximum attempts includes the first attempt.
    pub fn new(
        maximum_attempts: u32,
        retryable_errors: Vec<ErrorClass>,
        initial_backoff_ms: u64,
        maximum_backoff_ms: u64,
        maximum_recorded_jitter_ms: u64,
    ) -> Result<Self, RuntimeError> {
        if maximum_attempts == 0
            || maximum_attempts > 1_000
            || initial_backoff_ms == 0
            || initial_backoff_ms > maximum_backoff_ms
            || retryable_errors.len() > 32
        {
            return Err(RuntimeError::Scheduling(
                "invalid retry attempt, error-class, or backoff bounds".to_owned(),
            ));
        }
        let mut unique = Vec::new();
        for error in retryable_errors {
            if !unique.contains(&error) {
                unique.push(error);
            }
        }
        Ok(Self {
            maximum_attempts,
            retryable_errors: unique,
            initial_backoff_ms,
            maximum_backoff_ms,
            maximum_recorded_jitter_ms,
        })
    }

    /// Classifies whether another automatic attempt is safe and allowed.
    #[must_use]
    pub fn permits_automatic_retry(
        &self,
        completed_attempt_number: u32,
        error: ErrorClass,
        adapter_retryable: bool,
        side_effect: SideEffectClass,
        idempotency: IdempotencyBehavior,
        stable_key: Option<&IdempotencyKey>,
    ) -> bool {
        if !adapter_retryable
            || completed_attempt_number == 0
            || completed_attempt_number >= self.maximum_attempts
            || !self.retryable_errors.contains(&error)
        {
            return false;
        }
        match side_effect {
            SideEffectClass::None | SideEffectClass::ReadOnly => true,
            SideEffectClass::IdempotentWrite => {
                idempotency != IdempotencyBehavior::Unsupported && stable_key.is_some()
            }
            SideEffectClass::NonIdempotentWrite | SideEffectClass::Unknown => false,
        }
    }

    /// Calculates capped exponential backoff plus an already-recorded jitter fact.
    pub fn backoff_ms(
        &self,
        next_attempt_number: u32,
        recorded_jitter_ms: u64,
    ) -> Result<u64, RuntimeError> {
        if next_attempt_number < 2 || recorded_jitter_ms > self.maximum_recorded_jitter_ms {
            return Err(RuntimeError::Scheduling(
                "retry number or recorded jitter exceeds policy".to_owned(),
            ));
        }
        let exponent = next_attempt_number.saturating_sub(2).min(63);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let base = self
            .initial_backoff_ms
            .saturating_mul(multiplier)
            .min(self.maximum_backoff_ms);
        Ok(base
            .saturating_add(recorded_jitter_ms)
            .min(self.maximum_backoff_ms))
    }

    /// Reconciles policy backoff with an optional provider minimum without creating
    /// an unbounded durable timer.
    ///
    /// A provider delay above the configured hard maximum disables this automatic
    /// retry path instead of silently firing earlier than the provider requested.
    pub fn retry_delay_ms(
        &self,
        next_attempt_number: u32,
        recorded_jitter_ms: u64,
        provider_retry_after_ms: Option<u64>,
    ) -> Result<u64, RuntimeError> {
        let policy_delay = self.backoff_ms(next_attempt_number, recorded_jitter_ms)?;
        let provider_delay = provider_retry_after_ms.unwrap_or(0);
        if provider_delay > self.maximum_backoff_ms {
            return Err(RuntimeError::Scheduling(
                "provider retry-after exceeds the bounded retry policy".to_owned(),
            ));
        }
        Ok(policy_delay.max(provider_delay))
    }

    /// Maximum attempts, including the first attempt.
    #[must_use]
    pub const fn maximum_attempts(&self) -> u32 {
        self.maximum_attempts
    }

    /// Retryable closed error classes.
    #[must_use]
    pub fn retryable_errors(&self) -> &[ErrorClass] {
        &self.retryable_errors
    }

    /// Configured initial deterministic backoff.
    #[must_use]
    pub const fn initial_backoff_ms(&self) -> u64 {
        self.initial_backoff_ms
    }

    /// Hard maximum durable retry delay.
    #[must_use]
    pub const fn maximum_backoff_ms(&self) -> u64 {
        self.maximum_backoff_ms
    }

    /// Maximum accepted already-recorded jitter fact.
    #[must_use]
    pub const fn maximum_recorded_jitter_ms(&self) -> u64 {
        self.maximum_recorded_jitter_ms
    }
}

/// Immutable set of exact durable values used for one branch/repeat condition decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvaluationContext {
    values: BTreeMap<String, BoundedJson>,
}

impl EvaluationContext {
    /// Inserts one exact binding value under its canonical binding identity.
    pub fn insert(
        &mut self,
        source: &BindingSource,
        value: BoundedJson,
    ) -> Result<(), RuntimeError> {
        if matches!(source, BindingSource::Literal { .. }) {
            return Err(RuntimeError::Scheduling(
                "literal operands are embedded and cannot be inserted into context".to_owned(),
            ));
        }
        if self.values.insert(binding_key(source)?, value).is_some() {
            return Err(RuntimeError::Scheduling(
                "evaluation context cannot replace an existing durable binding".to_owned(),
            ));
        }
        Ok(())
    }

    /// Resolves one binding and applies its safe node-output path selector.
    pub fn resolve(&self, source: &BindingSource) -> Result<Option<Value>, RuntimeError> {
        if let BindingSource::Literal { value } = source {
            return Ok(Some(value.value().clone()));
        }
        let value = self
            .values
            .get(&binding_key(source)?)
            .map(|value| value.value().clone());
        match (source, value) {
            (BindingSource::NodeOutput { path, .. }, Some(value)) => {
                Ok(select_path(&value, path).cloned())
            }
            (_, value) => Ok(value),
        }
    }
}

/// Evaluates the safe Pass 1 condition AST against one exact durable context.
pub fn evaluate_condition(
    condition: &Condition,
    context: &EvaluationContext,
) -> Result<bool, RuntimeError> {
    match condition {
        Condition::Constant { value } => Ok(*value),
        Condition::All { conditions } => {
            for condition in conditions {
                if !evaluate_condition(condition, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Condition::Any { conditions } => {
            for condition in conditions {
                if evaluate_condition(condition, context)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Condition::Not { condition } => Ok(!evaluate_condition(condition, context)?),
        Condition::Exists { source } => Ok(context.resolve(source)?.is_some()),
        Condition::Compare {
            left,
            comparison,
            right,
        } => {
            let left = resolve_operand(left, context)?;
            let right = resolve_operand(right, context)?;
            compare_values(&left, *comparison, &right)
        }
    }
}

fn resolve_operand(
    operand: &ConditionOperand,
    context: &EvaluationContext,
) -> Result<Value, RuntimeError> {
    match operand {
        ConditionOperand::Literal { value } => Ok(value.value().clone()),
        ConditionOperand::Binding { source } => context.resolve(source)?.ok_or_else(|| {
            RuntimeError::Scheduling(
                "condition binding did not resolve to a durable value".to_owned(),
            )
        }),
    }
}

fn compare_values(
    left: &Value,
    comparison: Comparison,
    right: &Value,
) -> Result<bool, RuntimeError> {
    match comparison {
        Comparison::Equal => Ok(left == right),
        Comparison::NotEqual => Ok(left != right),
        Comparison::LessThan
        | Comparison::LessThanOrEqual
        | Comparison::GreaterThan
        | Comparison::GreaterThanOrEqual => {
            let left = left.as_number().ok_or_else(|| {
                RuntimeError::Scheduling("ordered condition comparison requires numbers".to_owned())
            })?;
            let right = right.as_number().ok_or_else(|| {
                RuntimeError::Scheduling("ordered condition comparison requires numbers".to_owned())
            })?;
            let ordering = compare_json_numbers(left, right)?;
            Ok(match comparison {
                Comparison::LessThan => ordering == Ordering::Less,
                Comparison::LessThanOrEqual => ordering != Ordering::Greater,
                Comparison::GreaterThan => ordering == Ordering::Greater,
                Comparison::GreaterThanOrEqual => ordering != Ordering::Less,
                Comparison::Equal | Comparison::NotEqual => false,
            })
        }
    }
}

#[derive(Debug)]
struct ExactDecimal {
    negative: bool,
    digits: Vec<u8>,
    scale: i64,
}

fn compare_json_numbers(left: &Number, right: &Number) -> Result<Ordering, RuntimeError> {
    let left = exact_decimal(left)?;
    let right = exact_decimal(right)?;
    if left.negative != right.negative {
        return Ok(if left.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        });
    }
    let magnitude = compare_decimal_magnitude(&left, &right)?;
    Ok(if left.negative {
        magnitude.reverse()
    } else {
        magnitude
    })
}

fn exact_decimal(number: &Number) -> Result<ExactDecimal, RuntimeError> {
    let rendered = number.to_string();
    let (negative, unsigned) = rendered
        .strip_prefix('-')
        .map_or((false, rendered.as_str()), |value| (true, value));
    let (coefficient, exponent) =
        if let Some((coefficient, exponent)) = unsigned.split_once(['e', 'E']) {
            let exponent = exponent.parse::<i64>().map_err(|_| {
                RuntimeError::Scheduling("JSON number exponent is out of range".to_owned())
            })?;
            (coefficient, exponent)
        } else {
            (unsigned, 0_i64)
        };
    let (integer, fraction) = coefficient
        .split_once('.')
        .map_or((coefficient, ""), |parts| parts);
    let mut digits: Vec<u8> = integer
        .bytes()
        .chain(fraction.bytes())
        .skip_while(|digit| *digit == b'0')
        .collect();
    if digits.is_empty() {
        return Ok(ExactDecimal {
            negative: false,
            digits: vec![b'0'],
            scale: 0,
        });
    }
    if digits.iter().any(|digit| !digit.is_ascii_digit()) {
        return Err(RuntimeError::Scheduling(
            "JSON number contains a non-decimal coefficient".to_owned(),
        ));
    }
    let fraction_digits = i64::try_from(fraction.len()).map_err(|_| {
        RuntimeError::Scheduling("JSON number fraction length is out of range".to_owned())
    })?;
    let mut scale = exponent.checked_sub(fraction_digits).ok_or_else(|| {
        RuntimeError::Scheduling("JSON number decimal scale is out of range".to_owned())
    })?;
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
        scale = scale.checked_add(1).ok_or_else(|| {
            RuntimeError::Scheduling("JSON number decimal scale is out of range".to_owned())
        })?;
    }
    Ok(ExactDecimal {
        negative,
        digits,
        scale,
    })
}

fn compare_decimal_magnitude(
    left: &ExactDecimal,
    right: &ExactDecimal,
) -> Result<Ordering, RuntimeError> {
    let left_digits = i64::try_from(left.digits.len()).map_err(|_| {
        RuntimeError::Scheduling("JSON number digit length is out of range".to_owned())
    })?;
    let right_digits = i64::try_from(right.digits.len()).map_err(|_| {
        RuntimeError::Scheduling("JSON number digit length is out of range".to_owned())
    })?;
    let left_magnitude = left_digits.checked_add(left.scale).ok_or_else(|| {
        RuntimeError::Scheduling("JSON number magnitude is out of range".to_owned())
    })?;
    let right_magnitude = right_digits.checked_add(right.scale).ok_or_else(|| {
        RuntimeError::Scheduling("JSON number magnitude is out of range".to_owned())
    })?;
    match left_magnitude.cmp(&right_magnitude) {
        Ordering::Equal => {
            let width = left.digits.len().max(right.digits.len());
            for index in 0..width {
                let left = left.digits.get(index).copied().unwrap_or(b'0');
                let right = right.digits.get(index).copied().unwrap_or(b'0');
                match left.cmp(&right) {
                    Ordering::Equal => {}
                    ordering => return Ok(ordering),
                }
            }
            Ok(Ordering::Equal)
        }
        ordering => Ok(ordering),
    }
}

fn select_path<'a>(value: &'a Value, path: &PathSelector) -> Option<&'a Value> {
    let mut selected = value;
    for segment in path.segments() {
        selected = match segment {
            PathSegment::Field(field) => selected.as_object()?.get(field.as_str())?,
            PathSegment::Index(index) => selected.as_array()?.get(usize::from(*index))?,
        };
    }
    Some(selected)
}

fn binding_key(source: &BindingSource) -> Result<String, RuntimeError> {
    let mut key_source = source.clone();
    if let BindingSource::NodeOutput { path, .. } = &mut key_source {
        *path = PathSelector::new(Vec::new())
            .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
    }
    let bytes = canonical_json_bytes(&key_source)
        .map_err(|error| RuntimeError::Scheduling(error.to_string()))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use milkdrift_blueprint::{BindingSource, Comparison, Condition, ConditionOperand, FieldId};
    use milkdrift_capability::{
        BoundedJson, ErrorClass, IdempotencyBehavior, IdempotencyKey, OperationId, SideEffectClass,
    };
    use milkdrift_persistence::{
        NodeExecutionId, RunSequence, RunnableIndexEntry, TimestampMillis,
    };
    use milkdrift_workspace::{BranchId, RunId};
    use serde_json::json;

    use super::{
        AdmissionRequest, AdmissionUsage, EvaluationContext, RetryPolicy, SchedulerLimits,
        evaluate_condition, select_fair_runnable,
    };

    #[test]
    fn safe_literal_condition_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let condition = Condition::Compare {
            left: ConditionOperand::Literal {
                value: BoundedJson::new(json!(2))?,
            },
            comparison: Comparison::LessThan,
            right: ConditionOperand::Literal {
                value: BoundedJson::new(json!(3))?,
            },
        };
        assert!(evaluate_condition(
            &condition,
            &EvaluationContext::default()
        )?);
        Ok(())
    }

    #[test]
    fn ordered_number_comparison_preserves_large_integer_precision()
    -> Result<(), Box<dyn std::error::Error>> {
        let condition = Condition::Compare {
            left: ConditionOperand::Literal {
                value: BoundedJson::new(json!(9_007_199_254_740_993_u64))?,
            },
            comparison: Comparison::GreaterThan,
            right: ConditionOperand::Literal {
                value: BoundedJson::new(json!(9_007_199_254_740_992_u64))?,
            },
        };
        assert!(evaluate_condition(
            &condition,
            &EvaluationContext::default()
        )?);
        Ok(())
    }

    #[test]
    fn evaluation_context_rejects_duplicate_binding_truth() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = BindingSource::WorkflowInput {
            field: FieldId::new("input")?,
        };
        let mut context = EvaluationContext::default();
        context.insert(&source, BoundedJson::new(json!({"version": 1}))?)?;
        assert!(
            context
                .insert(&source, BoundedJson::new(json!({"version": 2}))?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn admission_enforces_every_concurrency_dimension() -> Result<(), Box<dyn std::error::Error>> {
        let operation = OperationId::new("tool.execute")?;
        let run = RunId::new("run-a")?;
        let branch = BranchId::new("branch-a")?;
        let request = AdmissionRequest {
            run: run.clone(),
            branch: Some(branch.clone()),
            operation: operation.clone(),
        };
        let limits =
            SchedulerLimits::new(4, 2, 1, 2)?.with_capability_class(operation.clone(), 1)?;
        assert!(limits.allows(&request, &AdmissionUsage::default()));

        let mut usage = AdmissionUsage::default();
        usage.branches.insert((run.clone(), branch), 1);
        assert!(!limits.allows(&request, &usage));
        usage.branches.clear();
        usage.runs.insert(run.clone(), 2);
        assert!(!limits.allows(&request, &usage));
        usage.runs.clear();
        usage.capability_classes.insert(operation, 1);
        assert!(!limits.allows(&request, &usage));
        usage.capability_classes.clear();
        usage.global = 4;
        assert!(!limits.allows(&request, &usage));
        Ok(())
    }

    #[test]
    fn global_admission_limit_fits_one_durable_active_lease_page() {
        assert!(SchedulerLimits::new(milkdrift_persistence::MAX_PAGE_SIZE, 1, 1, 1).is_ok());
        assert!(SchedulerLimits::new(milkdrift_persistence::MAX_PAGE_SIZE + 1, 1, 1, 1).is_err());
    }

    #[test]
    fn runnable_selection_round_robins_runs_and_preserves_local_priority()
    -> Result<(), Box<dyn std::error::Error>> {
        let entry =
            |run: &str, execution: &str, priority| -> Result<_, Box<dyn std::error::Error>> {
                Ok(RunnableIndexEntry {
                    run: RunId::new(run)?,
                    execution: NodeExecutionId::new(execution)?,
                    eligible_at: TimestampMillis::new(1),
                    priority,
                    through_sequence: RunSequence::FIRST,
                })
            };
        let selected = select_fair_runnable(
            [
                entry("run-a", "execution-a-low", 1)?,
                entry("run-a", "execution-a-high", 9)?,
                entry("run-b", "execution-b", 2)?,
            ],
            3,
        );
        let observed: Vec<_> = selected
            .iter()
            .map(|entry| (entry.run.as_str(), entry.execution.as_str()))
            .collect();
        assert_eq!(
            observed,
            vec![
                ("run-a", "execution-a-high"),
                ("run-b", "execution-b"),
                ("run-a", "execution-a-low"),
            ]
        );
        Ok(())
    }

    #[test]
    fn retry_policy_never_repeats_uncertain_non_idempotent_effects()
    -> Result<(), Box<dyn std::error::Error>> {
        let policy = RetryPolicy::new(3, vec![ErrorClass::Transport], 100, 1_000, 25)?;
        let key = IdempotencyKey::new("stable-attempt-key")?;
        assert!(policy.permits_automatic_retry(
            1,
            ErrorClass::Transport,
            true,
            SideEffectClass::ReadOnly,
            IdempotencyBehavior::Unsupported,
            None,
        ));
        assert!(policy.permits_automatic_retry(
            1,
            ErrorClass::Transport,
            true,
            SideEffectClass::IdempotentWrite,
            IdempotencyBehavior::CapabilityScoped,
            Some(&key),
        ));
        assert!(!policy.permits_automatic_retry(
            1,
            ErrorClass::Transport,
            true,
            SideEffectClass::NonIdempotentWrite,
            IdempotencyBehavior::CapabilityScoped,
            Some(&key),
        ));
        assert!(!policy.permits_automatic_retry(
            1,
            ErrorClass::Transport,
            true,
            SideEffectClass::Unknown,
            IdempotencyBehavior::Unsupported,
            None,
        ));
        assert_eq!(policy.backoff_ms(2, 25)?, 125);
        assert_eq!(policy.backoff_ms(10, 25)?, 1_000);
        assert_eq!(policy.retry_delay_ms(2, 25, Some(400))?, 400);
        assert!(policy.retry_delay_ms(2, 25, Some(u64::MAX)).is_err());
        Ok(())
    }
}
