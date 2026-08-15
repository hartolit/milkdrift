use super::*;

pub(crate) fn candle_fixture_source() -> TestResult<CandleLlamaSource> {
    let directory = candle_fixture_directory();
    CandleLlamaSource::from_local_files(
        directory.join("config.json"),
        vec![directory.join("model.safetensors")],
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn mixed_fixture_source(fixture: &ConvertedFixture) -> TestResult<CandleLlamaSource> {
    CandleLlamaSource::from_local_files(
        fixture.config_path.clone(),
        vec![fixture.weight_path.clone()],
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn candle_fixture_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/candle-llama")
}

pub(crate) struct ConvertedFixture {
    directory: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) weight_path: PathBuf,
}

impl ConvertedFixture {
    pub(crate) fn create(primary_dtype: DType, mixed_f32: bool) -> TestResult<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-e0-phase12-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("model.safetensors");
        write_converted_configuration(&config_path, primary_dtype)?;
        convert_weights(
            &candle_fixture_directory().join("model.safetensors"),
            &weight_path,
            primary_dtype,
            mixed_f32,
        )?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
        })
    }
}

impl Drop for ConvertedFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

pub(crate) fn write_converted_configuration(
    destination: &Path,
    primary_dtype: DType,
) -> TestResult {
    let declaration = match primary_dtype {
        DType::F16 => "float16",
        DType::BF16 => "bfloat16",
        DType::F32 => "float32",
        _ => {
            return Err(format!(
                "unsupported converted fixture dtype: {primary_dtype:?}"
            ));
        }
    };
    let source = candle_fixture_directory().join("config.json");
    let configuration = fs::read_to_string(&source)
        .map_err(|error| format!("read converted fixture configuration source: {error}"))?;
    let expected = "\"dtype\": \"float32\"";
    if configuration.matches(expected).count() != 1 {
        return Err("committed fixture configuration has unexpected dtype declaration".to_owned());
    }
    let replacement = format!("\"dtype\": \"{declaration}\"");
    let converted = configuration.replacen(expected, replacement.as_str(), 1);
    fs::write(destination, converted)
        .map_err(|error| format!("write converted fixture configuration: {error}"))
}

pub(crate) fn convert_weights(
    source: &Path,
    destination: &Path,
    primary_dtype: DType,
    mixed_f32: bool,
) -> TestResult {
    let tensors = candle_core::safetensors::load(source, &Device::Cpu)
        .map_err(|error| format!("load F32 conversion source: {error}"))?;
    let converted = tensors
        .into_iter()
        .map(|(name, tensor)| {
            let dtype = if mixed_f32 && name == "model.norm.weight" {
                DType::F32
            } else {
                primary_dtype
            };
            let tensor = tensor
                .to_dtype(dtype)
                .map_err(|error| format!("convert {name} to {dtype:?}: {error}"))?;
            Ok((name, tensor))
        })
        .collect::<TestResult<HashMap<String, Tensor>>>()?;
    candle_core::safetensors::save(&converted, destination)
        .map_err(|error| format!("save converted fixture: {error}"))
}
