use super::*;

pub(crate) struct TinyLlamaFixture {
    directory: PathBuf,
    pub(crate) config_path: PathBuf,
    pub(crate) weight_path: PathBuf,
    pub(crate) second_weight_path: PathBuf,
}

impl TinyLlamaFixture {
    pub(crate) fn create(profile: RequiredProfile, extras: &[ExtraTensor]) -> TestResult<Self> {
        let fixture = Self::empty()?;
        let tensors = create_weight_tensors(profile)?;
        candle_core::safetensors::save(&tensors, &fixture.weight_path)
            .map_err(|error| format!("save weights: {error}"))?;
        append_raw_extras(&fixture.weight_path, extras)?;
        Ok(fixture)
    }

    pub(crate) fn create_sharded(
        profile: RequiredProfile,
        duplicate_extra: bool,
    ) -> TestResult<Self> {
        let fixture = Self::empty()?;
        let tensors = create_weight_tensors(profile)?;
        let mut first = HashMap::new();
        let mut second = HashMap::new();
        for (name, tensor) in tensors {
            let destination = if name.contains("model.layers") {
                &mut second
            } else {
                &mut first
            };
            if destination.insert(name, tensor).is_some() {
                return Err("duplicate sharded tensor".to_owned());
            }
        }
        if duplicate_extra {
            let name = "unused.cross_shard_duplicate".to_owned();
            let first_duplicate =
                Tensor::zeros(1, DType::F32, &Device::Cpu).map_err(|error| error.to_string())?;
            let second_duplicate =
                Tensor::zeros(1, DType::F32, &Device::Cpu).map_err(|error| error.to_string())?;
            first.insert(name.clone(), first_duplicate);
            second.insert(name, second_duplicate);
        }
        candle_core::safetensors::save(&first, &fixture.second_weight_path)
            .map_err(|error| format!("save first shard: {error}"))?;
        candle_core::safetensors::save(&second, &fixture.weight_path)
            .map_err(|error| format!("save second shard: {error}"))?;
        Ok(fixture)
    }

    pub(crate) fn empty() -> TestResult<Self> {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "milkdrift-candle-loader-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let config_path = directory.join("config.json");
        let weight_path = directory.join("z-model.safetensors");
        let second_weight_path = directory.join("a-model.safetensors");
        write_tiny_config(&config_path, ConfigDeclaration::Absent)?;
        Ok(Self {
            directory,
            config_path,
            weight_path,
            second_weight_path,
        })
    }

    pub(crate) fn source(&self, declaration: ConfigDeclaration) -> TestResult<CandleLlamaSource> {
        self.source_with_paths(declaration, vec![self.weight_path.clone()])
    }

    pub(crate) fn source_with_paths(
        &self,
        declaration: ConfigDeclaration,
        paths: Vec<PathBuf>,
    ) -> TestResult<CandleLlamaSource> {
        write_tiny_config(&self.config_path, declaration)?;
        CandleLlamaSource::from_local_files(self.config_path.clone(), paths)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn source_with_shards(
        &self,
        declaration: ConfigDeclaration,
        shards: Vec<CandleWeightShard>,
    ) -> TestResult<CandleLlamaSource> {
        write_tiny_config(&self.config_path, declaration)?;
        CandleLlamaSource::new(self.config_path.clone(), shards).map_err(|error| error.to_string())
    }
}

impl Drop for TinyLlamaFixture {
    fn drop(&mut self) {
        let _ignored = fs::remove_dir_all(&self.directory);
    }
}

pub(crate) fn write_tiny_config(path: &Path, declaration: ConfigDeclaration) -> TestResult {
    let mut config = json!({
        "model_type": "llama",
        "hidden_size": 8,
        "intermediate_size": 16,
        "vocab_size": 16,
        "num_hidden_layers": 1,
        "num_attention_heads": 2,
        "num_key_value_heads": 2,
        "rms_norm_eps": 0.00001,
        "rope_theta": 10000.0,
        "bos_token_id": 1,
        "eos_token_id": 2,
        "rope_scaling": null,
        "max_position_embeddings": 16,
        "tie_word_embeddings": false
    });
    let object = config
        .as_object_mut()
        .ok_or_else(|| "tiny config must be an object".to_owned())?;
    match declaration {
        ConfigDeclaration::Absent => {}
        ConfigDeclaration::F32 => {
            object.insert("dtype".to_owned(), JsonValue::String("float32".to_owned()));
        }
        ConfigDeclaration::F16 => {
            object.insert("dtype".to_owned(), JsonValue::String("float16".to_owned()));
        }
        ConfigDeclaration::Bf16 => {
            object.insert("dtype".to_owned(), JsonValue::String("bfloat16".to_owned()));
        }
        ConfigDeclaration::Unsupported => {
            object.insert("dtype".to_owned(), JsonValue::String("int4".to_owned()));
        }
        ConfigDeclaration::Conflict => {
            object.insert("dtype".to_owned(), JsonValue::String("float16".to_owned()));
            object.insert(
                "torch_dtype".to_owned(),
                JsonValue::String("bfloat16".to_owned()),
            );
        }
    }
    let mut bytes = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| error.to_string())
}

pub(crate) fn create_weight_tensors(
    profile: RequiredProfile,
) -> TestResult<HashMap<String, Tensor>> {
    let device = Device::Cpu;
    let mut tensors = HashMap::new();
    insert_token_matrix(&mut tensors, "model.embed_tokens.weight", profile, &device)?;
    insert_token_matrix(&mut tensors, "lm_head.weight", profile, &device)?;
    let norm_length = if profile == RequiredProfile::InvalidNormShape {
        7
    } else {
        8
    };
    insert_vector(
        &mut tensors,
        "model.norm.weight",
        norm_length,
        profile,
        &device,
    )?;
    for projection in ["q_proj", "k_proj", "v_proj", "o_proj"] {
        insert_matrix(
            &mut tensors,
            &format!("model.layers.0.self_attn.{projection}.weight"),
            8,
            8,
            profile,
            &device,
        )?;
    }
    for normalization in ["input_layernorm", "post_attention_layernorm"] {
        insert_vector(
            &mut tensors,
            &format!("model.layers.0.{normalization}.weight"),
            8,
            profile,
            &device,
        )?;
    }
    for projection in ["gate_proj", "up_proj"] {
        insert_matrix(
            &mut tensors,
            &format!("model.layers.0.mlp.{projection}.weight"),
            16,
            8,
            profile,
            &device,
        )?;
    }
    insert_matrix(
        &mut tensors,
        "model.layers.0.mlp.down_proj.weight",
        8,
        16,
        profile,
        &device,
    )?;
    Ok(tensors)
}

pub(crate) fn insert_token_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let values = (0_u32..16)
        .flat_map(|token| {
            (0_u32..8).map(move |dimension| {
                if token & (1_u32 << (dimension % 4)) == 0 {
                    -1.0_f32
                } else {
                    1.0_f32
                }
            })
        })
        .collect::<Vec<_>>();
    let tensor = Tensor::from_vec(values, (16, 8), device)
        .and_then(|tensor| tensor.to_dtype(profile.dtype_for(name)))
        .map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

pub(crate) fn insert_matrix(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    rows: usize,
    columns: usize,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let tensor = Tensor::zeros((rows, columns), profile.dtype_for(name), device)
        .map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

pub(crate) fn insert_vector(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    length: usize,
    profile: RequiredProfile,
    device: &Device,
) -> TestResult {
    let tensor =
        Tensor::ones(length, profile.dtype_for(name), device).map_err(|error| error.to_string())?;
    insert_tensor(tensors, name, tensor)
}

pub(crate) fn insert_tensor(
    tensors: &mut HashMap<String, Tensor>,
    name: &str,
    tensor: Tensor,
) -> TestResult {
    if tensors.insert(name.to_owned(), tensor).is_some() {
        return Err(format!("duplicate fixture tensor: {name}"));
    }
    Ok(())
}

pub(crate) fn append_raw_extras(path: &Path, extras: &[ExtraTensor]) -> TestResult {
    if extras.is_empty() {
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let header_length = read_header_length(bytes.as_slice())?;
    let data_start = 8_usize
        .checked_add(header_length)
        .ok_or_else(|| "Safetensors data start overflow".to_owned())?;
    let header_bytes = bytes
        .get(8..data_start)
        .ok_or_else(|| "saved Safetensors header is truncated".to_owned())?;
    let mut header: JsonMap<String, JsonValue> =
        serde_json::from_slice(header_bytes).map_err(|error| error.to_string())?;
    let mut payload = bytes
        .get(data_start..)
        .ok_or_else(|| "saved Safetensors payload is missing".to_owned())?
        .to_vec();

    for extra in extras {
        if header.contains_key(extra.name) {
            return Err(format!("duplicate raw extra tensor: {}", extra.name));
        }
        let start = u64::try_from(payload.len()).map_err(|error| error.to_string())?;
        let byte_length = extra
            .elements
            .checked_mul(extra.bytes_per_element)
            .ok_or_else(|| format!("raw extra byte length overflow: {}", extra.name))?;
        let new_length = payload
            .len()
            .checked_add(byte_length)
            .ok_or_else(|| format!("raw payload length overflow: {}", extra.name))?;
        payload
            .try_reserve_exact(byte_length)
            .map_err(|error| error.to_string())?;
        payload.resize(new_length, 0);
        let end = u64::try_from(new_length).map_err(|error| error.to_string())?;
        header.insert(
            extra.name.to_owned(),
            json!({
                "dtype": extra.dtype,
                "shape": [extra.elements],
                "data_offsets": [start, end]
            }),
        );
    }

    let mut encoded_header = serde_json::to_vec(&header).map_err(|error| error.to_string())?;
    let padding = (8 - encoded_header.len() % 8) % 8;
    encoded_header.resize(encoded_header.len() + padding, b' ');
    let encoded_length = u64::try_from(encoded_header.len()).map_err(|error| error.to_string())?;
    let total_length = 8_usize
        .checked_add(encoded_header.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or_else(|| "rebuilt Safetensors length overflow".to_owned())?;
    let mut rebuilt = Vec::new();
    rebuilt
        .try_reserve_exact(total_length)
        .map_err(|error| error.to_string())?;
    rebuilt.extend_from_slice(&encoded_length.to_le_bytes());
    rebuilt.extend_from_slice(&encoded_header);
    rebuilt.extend_from_slice(&payload);
    fs::write(path, rebuilt).map_err(|error| error.to_string())
}

pub(crate) fn read_header_length(bytes: &[u8]) -> TestResult<usize> {
    let prefix: [u8; 8] = bytes
        .get(..8)
        .ok_or_else(|| "Safetensors prefix is truncated".to_owned())?
        .try_into()
        .map_err(|_| "Safetensors prefix has the wrong length".to_owned())?;
    usize::try_from(u64::from_le_bytes(prefix)).map_err(|error| error.to_string())
}

pub(crate) fn file_identity(path: &Path) -> TestResult<(u64, [u8; 32])> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let byte_length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
    Ok((byte_length, Sha256::digest(bytes).into()))
}

pub(crate) fn scalar_set(types: &[ScalarType]) -> ScalarTypeSet {
    let mut set = ScalarTypeSet::EMPTY;
    for scalar_type in types {
        set.insert(*scalar_type);
    }
    set
}

pub(crate) fn write_raw_safetensors(path: &Path, header: &str, payload: &[u8]) -> TestResult {
    let header_length = u64::try_from(header.len()).map_err(|error| error.to_string())?;
    let total_length = 8_usize
        .checked_add(header.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or_else(|| "raw fixture length overflow".to_owned())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(total_length)
        .map_err(|error| error.to_string())?;
    bytes.extend_from_slice(&header_length.to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    bytes.extend_from_slice(payload);
    fs::write(path, bytes).map_err(|error| error.to_string())
}
