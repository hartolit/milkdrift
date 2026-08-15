use super::*;

pub(crate) const CUDA_EXECUTION_DEVICE: ExecutionDevice =
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);

#[cfg(feature = "cuda-hardware-tests")]
pub(crate) fn candle_mixed_cuda_fixture_covers_e0_generation_accounting_and_lifecycle() -> TestResult
{
    mixed_f16_f32_fixture_covers_generation_accounting_and_lifecycle(CUDA_EXECUTION_DEVICE)
}
