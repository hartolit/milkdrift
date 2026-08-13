//! Allocation enforcement for graph validation, provenance, and state transitions.

#![forbid(unsafe_code)]

use std::alloc::System;
use std::process::ExitCode;

use domain_contracts::ArtifactId;
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use task_graph::{
    ArtifactFlow, GraphValidationScratch, TaskArtifactInput, TaskArtifactOutput, TaskDependency,
    TaskGraph, TaskId, TaskNode, TaskRuntimeState, TaskStateTable, validate_artifact_flow,
    validate_graph,
};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() -> ExitCode {
    let nodes = [
        TaskNode {
            id: TaskId::new(1),
            operation: 10_u16,
            maximum_attempts: core::num::NonZeroU16::MIN,
        },
        TaskNode {
            id: TaskId::new(2),
            operation: 20_u16,
            maximum_attempts: core::num::NonZeroU16::MIN,
        },
    ];
    let dependencies = [TaskDependency {
        prerequisite: TaskId::new(1),
        dependent: TaskId::new(2),
    }];
    let external = [ArtifactId::new(1)];
    let inputs = [
        TaskArtifactInput {
            consumer: TaskId::new(1),
            artifact: ArtifactId::new(1),
        },
        TaskArtifactInput {
            consumer: TaskId::new(2),
            artifact: ArtifactId::new(2),
        },
    ];
    let outputs = [TaskArtifactOutput {
        producer: TaskId::new(1),
        artifact: ArtifactId::new(2),
    }];
    let graph = TaskGraph::new(&nodes, &dependencies);
    let flow = ArtifactFlow::new(&external, &inputs, &outputs);
    let mut incoming = [0_u32; 2];
    let mut queue = [0_usize; 2];
    let mut states = [TaskRuntimeState::default(); 2];
    let mut ready = [TaskId::new(0); 2];

    let region = Region::new(GLOBAL);
    let result = validate_graph(
        &graph,
        GraphValidationScratch {
            incoming_counts: &mut incoming,
            queue: &mut queue,
        },
    )
    .and_then(|()| validate_artifact_flow(&graph, &flow))
    .and_then(|()| {
        let mut table = TaskStateTable::new(&graph, &mut states)?;
        let count = table.ready_tasks(&graph, &mut ready)?;
        let Some(task) = ready.first().copied().filter(|_| count == 1) else {
            return Err(task_graph::TaskGraphError::UnknownTask(TaskId::new(0)));
        };
        let attempt = table.start(&graph, task)?;
        table.succeed_attempt(&graph, attempt)
    });
    let change = region.change();

    if result.is_ok() && change.allocations == 0 && change.reallocations == 0 {
        ExitCode::SUCCESS
    } else {
        eprintln!("task-graph allocation contract failed: result={result:?}, change={change:?}");
        ExitCode::FAILURE
    }
}
