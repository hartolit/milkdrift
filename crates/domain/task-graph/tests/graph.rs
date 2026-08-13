//! Generic graph validation and runtime-state integration tests.

use core::num::NonZeroU16;

use task_graph::{
    GraphValidationScratch, TaskAttempt, TaskDependency, TaskGraph, TaskGraphError, TaskId,
    TaskNode, TaskRuntimeState, TaskStateTable, TaskStatus, validate_graph,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Operation(u16);

fn node(id: u64, operation: u16, attempts: u16) -> Result<TaskNode<Operation>, &'static str> {
    Ok(TaskNode {
        id: TaskId::new(id),
        operation: Operation(operation),
        maximum_attempts: NonZeroU16::new(attempts).ok_or("attempt count must be non-zero")?,
    })
}

fn validate<const N: usize>(graph: &TaskGraph<'_, Operation>) -> Result<(), TaskGraphError> {
    let mut incoming = [0_u32; N];
    let mut queue = [0_usize; N];
    validate_graph(
        graph,
        GraphValidationScratch {
            incoming_counts: &mut incoming,
            queue: &mut queue,
        },
    )
}

#[test]
fn generic_nodes_and_acyclic_dependencies_validate() -> Result<(), &'static str> {
    let nodes = [node(20, 9, 2)?, node(10, 7, 1)?, node(30, 11, 3)?];
    let dependencies = [
        TaskDependency {
            prerequisite: TaskId::new(20),
            dependent: TaskId::new(30),
        },
        TaskDependency {
            prerequisite: TaskId::new(10),
            dependent: TaskId::new(30),
        },
    ];
    let graph = TaskGraph::new(&nodes, &dependencies);

    validate::<3>(&graph).map_err(|_| "valid graph rejected")?;
    assert_eq!(
        graph.node(TaskId::new(10)).map(|value| value.operation),
        Some(Operation(7))
    );
    Ok(())
}

#[test]
fn duplicate_ids_edges_missing_nodes_cycles_and_scratch_exhaustion_are_rejected()
-> Result<(), &'static str> {
    let duplicate_nodes = [node(1, 1, 1)?, node(1, 2, 1)?];
    assert_eq!(
        validate::<2>(&TaskGraph::new(&duplicate_nodes, &[])),
        Err(TaskGraphError::DuplicateTask(TaskId::new(1)))
    );

    let nodes = [node(1, 1, 1)?, node(2, 2, 1)?];
    let edge = TaskDependency {
        prerequisite: TaskId::new(1),
        dependent: TaskId::new(2),
    };
    assert_eq!(
        validate::<2>(&TaskGraph::new(&nodes, &[edge, edge])),
        Err(TaskGraphError::DuplicateDependency(edge))
    );
    let missing = TaskDependency {
        prerequisite: TaskId::new(9),
        dependent: TaskId::new(2),
    };
    assert_eq!(
        validate::<2>(&TaskGraph::new(&nodes, &[missing])),
        Err(TaskGraphError::UnknownTask(TaskId::new(9)))
    );
    let cycle = [
        edge,
        TaskDependency {
            prerequisite: TaskId::new(2),
            dependent: TaskId::new(1),
        },
    ];
    assert_eq!(
        validate::<2>(&TaskGraph::new(&nodes, &cycle)),
        Err(TaskGraphError::CycleDetected)
    );

    let graph = TaskGraph::new(&nodes, &[]);
    let mut incoming = [0_u32; 1];
    let mut queue = [0_usize; 2];
    assert!(matches!(
        validate_graph(
            &graph,
            GraphValidationScratch {
                incoming_counts: &mut incoming,
                queue: &mut queue,
            }
        ),
        Err(TaskGraphError::CapacityExhausted(_))
    ));
    Ok(())
}

#[test]
fn ready_order_follows_node_definition_order() -> Result<(), &'static str> {
    let nodes = [node(30, 3, 1)?, node(10, 1, 1)?, node(20, 2, 1)?];
    let graph = TaskGraph::new(&nodes, &[]);
    let mut states = [TaskRuntimeState::default(); 3];
    let table = TaskStateTable::new(&graph, &mut states).map_err(|_| "state table rejected")?;
    let mut ready = [TaskId::new(0); 3];

    assert_eq!(
        table
            .ready_tasks(&graph, &mut ready)
            .map_err(|_| "ready failed")?,
        3
    );
    assert_eq!(ready, [TaskId::new(30), TaskId::new(10), TaskId::new(20)]);
    Ok(())
}

#[test]
fn attempt_identity_retry_and_exhaustion_are_checked() -> Result<(), &'static str> {
    let nodes = [node(1, 1, 2)?];
    let graph = TaskGraph::new(&nodes, &[]);
    let mut states = [TaskRuntimeState::default(); 1];
    let mut table = TaskStateTable::new(&graph, &mut states).map_err(|_| "state table rejected")?;

    let first = table
        .start(&graph, TaskId::new(1))
        .map_err(|_| "first start failed")?;
    table
        .fail_attempt(&graph, first)
        .map_err(|_| "first failure failed")?;
    assert_eq!(
        table
            .state(&graph, TaskId::new(1))
            .map(|state| state.status),
        Some(TaskStatus::Failed)
    );
    let second = table
        .start(&graph, TaskId::new(1))
        .map_err(|_| "retry failed")?;
    assert_eq!(
        table.succeed_attempt(&graph, first),
        Err(TaskGraphError::InvalidAttempt {
            attempt: first,
            active: Some(second),
            state: TaskStatus::Running,
        })
    );
    table
        .fail_attempt(&graph, second)
        .map_err(|_| "exhaustion failed")?;
    assert_eq!(
        table
            .state(&graph, TaskId::new(1))
            .map(|state| state.status),
        Some(TaskStatus::Exhausted)
    );
    assert_eq!(
        table.start(&graph, TaskId::new(1)),
        Err(TaskGraphError::InvalidTransition {
            task: TaskId::new(1),
            state: TaskStatus::Exhausted,
        })
    );
    Ok(())
}

#[test]
fn completion_requires_the_active_attempt_identity() -> Result<(), &'static str> {
    let nodes = [node(1, 1, 2)?];
    let graph = TaskGraph::new(&nodes, &[]);
    let mut states = [TaskRuntimeState::default(); 1];
    let mut table = TaskStateTable::new(&graph, &mut states).map_err(|_| "state table rejected")?;
    let active = table
        .start(&graph, TaskId::new(1))
        .map_err(|_| "start failed")?;
    let wrong = TaskAttempt::new(
        TaskId::new(1),
        NonZeroU16::new(2).ok_or("attempt count must be non-zero")?,
    );

    assert!(matches!(
        table.fail_attempt(&graph, wrong),
        Err(TaskGraphError::InvalidAttempt { .. })
    ));
    table
        .succeed_attempt(&graph, active)
        .map_err(|_| "active attempt rejected")?;
    assert_eq!(
        table
            .state(&graph, TaskId::new(1))
            .map(|state| state.status),
        Some(TaskStatus::Succeeded)
    );
    Ok(())
}

#[test]
fn cancellation_blocks_all_descendants() -> Result<(), &'static str> {
    let nodes = [node(1, 1, 1)?, node(2, 2, 1)?, node(3, 3, 1)?];
    let dependencies = [
        TaskDependency {
            prerequisite: TaskId::new(1),
            dependent: TaskId::new(2),
        },
        TaskDependency {
            prerequisite: TaskId::new(2),
            dependent: TaskId::new(3),
        },
    ];
    let graph = TaskGraph::new(&nodes, &dependencies);
    let mut states = [TaskRuntimeState::default(); 3];
    let mut table = TaskStateTable::new(&graph, &mut states).map_err(|_| "state table rejected")?;

    table
        .cancel(&graph, TaskId::new(1))
        .map_err(|_| "cancel failed")?;
    assert_eq!(
        table
            .propagate_blocked(&graph)
            .map_err(|_| "propagation failed")?,
        2
    );
    assert_eq!(
        table
            .state(&graph, TaskId::new(1))
            .map(|state| state.status),
        Some(TaskStatus::Cancelled)
    );
    assert_eq!(
        table
            .state(&graph, TaskId::new(2))
            .map(|state| state.status),
        Some(TaskStatus::Blocked)
    );
    assert_eq!(
        table
            .state(&graph, TaskId::new(3))
            .map(|state| state.status),
        Some(TaskStatus::Blocked)
    );
    Ok(())
}
