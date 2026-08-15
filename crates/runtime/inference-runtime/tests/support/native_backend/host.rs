use super::{
    CANDLE_BACKEND, CandleLlamaLoader, CandleLlamaSource, CandleRuntime, CommandTicket, DeviceKind,
    EVENT_TIMEOUT, ExecutionDevice, HOMOGENEOUS_F32_DESCRIPTOR, HOMOGENEOUS_F32_FINAL_FOOTPRINT,
    HOMOGENEOUS_F32_LOADING_PEAK_FOOTPRINT, HostedRuntimeConfiguration, LoadConfiguration,
    LoadPlan, LoadReceipt, MODEL, MemoryBudget, ModelGeneration, ModelHandle, NonZeroU32,
    NonZeroU64, RuntimeCommand, RuntimeEvent, RuntimeLimits, RuntimeThread, ScalarType,
    ScalarTypeSet, TestResult, nonzero_usize, start_hosted_runtime,
};

pub(crate) const LOAD_TICKET: CommandTicket = CommandTicket::new(1);
pub(crate) fn hosted_runtime(
    execution_device: ExecutionDevice,
    token_capacity: usize,
    record_capacity: usize,
) -> TestResult<(CandleRuntime, RuntimeThread)> {
    let configuration =
        HostedRuntimeConfiguration::new(nonzero_usize(8)?, nonzero_usize(8)?, NonZeroU64::MIN)
            .with_token_output_capacity(
                nonzero_usize(token_capacity)?,
                nonzero_usize(record_capacity)?,
            );
    start_hosted_runtime(
        CandleLlamaLoader::new(CANDLE_BACKEND),
        RuntimeLimits::new(
            NonZeroU32::MIN,
            NonZeroU32::MIN,
            runtime_memory_budget(execution_device)?,
        ),
        configuration,
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn runtime_memory_budget(execution_device: ExecutionDevice) -> TestResult<MemoryBudget> {
    let device_bytes = match execution_device.kind {
        DeviceKind::Cpu => 0,
        DeviceKind::Cuda => u64::MAX,
        _ => return Err("native fixture selected an unsupported execution device".to_owned()),
    };
    Ok(MemoryBudget {
        host_bytes: u64::MAX,
        device_bytes,
    })
}

pub(crate) fn load_model(
    hosted: &CandleRuntime,
    source: CandleLlamaSource,
    execution_device: ExecutionDevice,
) -> TestResult<LoadReceipt> {
    hosted
        .try_submit(RuntimeCommand::LoadModel {
            ticket: LOAD_TICKET,
            model_id: MODEL,
            source,
            execution_device,
        })
        .map_err(|error| format!("load command rejected: {error:?}"))?;
    match hosted
        .receive_timeout(EVENT_TIMEOUT)
        .map_err(|error| format!("load event failed: {error:?}"))?
    {
        RuntimeEvent::ModelLoaded {
            ticket,
            result: Ok(receipt),
        } if ticket == LOAD_TICKET => Ok(receipt),
        RuntimeEvent::ModelLoaded {
            result: Err(error), ..
        } => Err(format!("model load failed: {error:?}")),
        event => Err(format!(
            "unexpected load event for ticket {:?}",
            event.ticket()
        )),
    }
}

pub(crate) fn assert_homogeneous_f32_plan(plan: &LoadPlan, execution_device: ExecutionDevice) {
    assert_eq!(
        plan.descriptor.metadata.observed_tensor_scalar_types,
        ScalarTypeSet::from_scalar(ScalarType::F32)
    );
    assert_eq!(
        *plan,
        LoadPlan {
            accepted_configuration: LoadConfiguration {
                handle: ModelHandle::new(MODEL, ModelGeneration::new(1)),
                execution_device,
                memory_budget: MemoryBudget {
                    host_bytes: u64::MAX,
                    device_bytes: u64::MAX,
                },
            },
            descriptor: HOMOGENEOUS_F32_DESCRIPTOR,
            execution_scalar_type: ScalarType::F32,
            final_footprint: HOMOGENEOUS_F32_FINAL_FOOTPRINT,
            loading_peak_footprint: HOMOGENEOUS_F32_LOADING_PEAK_FOOTPRINT,
        }
    );
}
