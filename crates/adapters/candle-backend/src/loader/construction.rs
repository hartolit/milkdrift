//! Candle Llama construction over already materialized execution tensors.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Shape, Tensor};
use candle_nn::VarBuilder;
use candle_nn::var_builder::SimpleBackend;
use candle_transformers::models::llama::{Config, Llama};
use domain_contracts::{BackendId, LoadError};

use crate::failure::{CODE_MODEL_LOAD, CODE_MODEL_LOAD_PANIC};

use super::{invalid_model_failure, map_candle_load_error};

/// Constructs a Llama by borrowing the final load map.
///
/// Candle's map backend returns shallow handles after checking shape, device,
/// and dtype. Borrowing avoids a second allocated map and leaves the prepared
/// transaction as the only explicit owner outside the constructed model.
pub(super) fn construct_llama(
    backend: BackendId,
    tensors: &HashMap<String, Tensor>,
    execution_dtype: DType,
    device: &Device,
    config: &Config,
) -> Result<Llama, LoadError> {
    let variable_builder = VarBuilder::from_backend(
        Box::new(BorrowedTensorBackend { tensors }),
        execution_dtype,
        device.clone(),
    );
    catch_unwind(AssertUnwindSafe(|| Llama::load(variable_builder, config)))
        .map_err(|_| invalid_model_failure(backend, CODE_MODEL_LOAD_PANIC))?
        .map_err(|error| map_candle_load_error(backend, device, &error, CODE_MODEL_LOAD))
}

struct BorrowedTensorBackend<'a> {
    tensors: &'a HashMap<String, Tensor>,
}

impl SimpleBackend for BorrowedTensorBackend<'_> {
    fn get(
        &self,
        shape: Shape,
        name: &str,
        _hints: candle_nn::Init,
        dtype: DType,
        device: &Device,
    ) -> candle_core::Result<Tensor> {
        let tensor = self.get_unchecked(name, dtype, device)?;
        if tensor.shape() != &shape {
            candle_core::bail!(
                "shape mismatch for {name}: expected {shape:?}, got {:?}",
                tensor.shape()
            )
        }
        Ok(tensor)
    }

    fn get_unchecked(
        &self,
        name: &str,
        dtype: DType,
        device: &Device,
    ) -> candle_core::Result<Tensor> {
        let tensor =
            self.tensors
                .get(name)
                .ok_or_else(|| candle_core::Error::CannotFindTensor {
                    path: name.to_owned(),
                })?;
        tensor.to_device(device)?.to_dtype(dtype)
    }

    fn contains_tensor(&self, name: &str) -> bool {
        self.tensors.contains_key(name)
    }
}
