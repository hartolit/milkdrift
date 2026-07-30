"""Build the committed tiny F32 Llama GGUF fixture using only Python stdlib."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Any

GGUF_VERSION = 3
GGUF_ALIGNMENT = 32
GGML_TYPE_F32 = 0

TYPE_UINT32 = 4
TYPE_INT32 = 5
TYPE_FLOAT32 = 6
TYPE_BOOL = 7
TYPE_STRING = 8
TYPE_ARRAY = 9

HERE = Path(__file__).resolve().parent
CANDLE_FIXTURE = HERE.parent / "candle-llama"
DEFAULT_OUTPUT = HERE / "tiny-llama-f32.gguf"
CONFIG_PATH = CANDLE_FIXTURE / "config.json"
SAFETENSORS_PATH = CANDLE_FIXTURE / "model.safetensors"
EXPECTED_SOURCE_SHA256 = {
    CONFIG_PATH.name: "6c27e4687ddb94eea5e180e7d2e679826c4ccb1b7224945aab9f013607704b7a",
    SAFETENSORS_PATH.name: "a4407aa5c225725d3ea9036e41734533af33b95a0c778309858feed003c2a64c",
}

TOKENS = [
    "<unk>",
    "<s>",
    "</s>",
    "▁",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "<0x0A>",
]
# llama_token_type: NORMAL=1, UNKNOWN=2, CONTROL=3, BYTE=6.
TOKEN_TYPES = [2, 3, 3, *([1] * 12), 6]
TOKEN_SCORES = [0.0] * len(TOKENS)


@dataclass(frozen=True)
class Tensor:
    name: str
    dimensions: tuple[int, ...]
    data: bytes


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_checked(path: Path) -> bytes:
    data = path.read_bytes()
    expected = EXPECTED_SOURCE_SHA256[path.name]
    actual = sha256(data)
    if actual != expected:
        raise ValueError(
            f"{path.name} SHA-256 changed: expected {expected}, found {actual}"
        )
    return data


def checked_config(raw: bytes) -> dict[str, Any]:
    config = json.loads(raw)
    expected = {
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
        "rope_scaling": None,
        "max_position_embeddings": 16,
        "tie_word_embeddings": False,
    }
    if config != expected:
        raise ValueError("tiny Llama config no longer matches the reviewed fixture schema")
    if len(TOKENS) != config["vocab_size"]:
        raise ValueError("tokenizer vocabulary does not match model vocab_size")
    return config


def parse_safetensors(raw: bytes) -> tuple[dict[str, Any], int]:
    if len(raw) < 8:
        raise ValueError("Safetensors file is shorter than its length prefix")
    header_length = struct.unpack_from("<Q", raw, 0)[0]
    data_start = 8 + header_length
    if data_start > len(raw):
        raise ValueError("Safetensors header extends past end of file")
    header = json.loads(raw[8:data_start])
    if not isinstance(header, dict):
        raise TypeError("Safetensors header is not an object")
    return header, data_start


def product(values: list[int]) -> int:
    return math.prod(values)


def tensor_bytes(
    raw: bytes,
    header: dict[str, Any],
    data_start: int,
    name: str,
    expected_shape: list[int],
) -> bytes:
    try:
        info = header[name]
    except KeyError as error:
        raise ValueError(f"missing Safetensors tensor {name}") from error
    if info.get("dtype") != "F32" or info.get("shape") != expected_shape:
        raise ValueError(
            f"unexpected {name} dtype/shape: {info.get('dtype')} {info.get('shape')}"
        )
    offsets = info.get("data_offsets")
    if not isinstance(offsets, list) or len(offsets) != 2:
        raise ValueError(f"invalid data offsets for {name}")
    begin, end = offsets
    expected_bytes = product(expected_shape) * 4
    if begin < 0 or end - begin != expected_bytes or data_start + end > len(raw):
        raise ValueError(f"invalid data extent for {name}")
    return raw[data_start + begin : data_start + end]


def permute_rope_rows(data: bytes, shape: list[int], head_count: int) -> bytes:
    """Convert Hugging Face split-half Q/K rows to llama.cpp interleaved rows."""
    rows, columns = shape
    if rows % head_count != 0:
        raise ValueError("Q/K rows are not divisible by their attention head count")
    rows_per_head = rows // head_count
    if rows_per_head % 2 != 0:
        raise ValueError("Q/K rows per head must be even for RoPE permutation")
    values = struct.unpack(f"<{rows * columns}f", data)
    output: list[float] = []
    half = rows_per_head // 2
    for head in range(head_count):
        head_start = head * rows_per_head
        for pair in range(half):
            for side in range(2):
                source_row = head_start + side * half + pair
                begin = source_row * columns
                output.extend(values[begin : begin + columns])
    return struct.pack(f"<{len(output)}f", *output)


def build_tensors(
    config: dict[str, Any], raw: bytes, header: dict[str, Any], data_start: int
) -> list[Tensor]:
    hidden = config["hidden_size"]
    intermediate = config["intermediate_size"]
    vocabulary = config["vocab_size"]
    heads = config["num_attention_heads"]
    kv_heads = config["num_key_value_heads"]
    head_width = hidden // heads
    kv_width = head_width * kv_heads

    specifications: list[tuple[str, str, list[int], int | None]] = [
        ("model.embed_tokens.weight", "token_embd.weight", [vocabulary, hidden], None),
        ("model.norm.weight", "output_norm.weight", [hidden], None),
        ("lm_head.weight", "output.weight", [vocabulary, hidden], None),
        ("model.layers.0.input_layernorm.weight", "blk.0.attn_norm.weight", [hidden], None),
        ("model.layers.0.self_attn.q_proj.weight", "blk.0.attn_q.weight", [hidden, hidden], heads),
        ("model.layers.0.self_attn.k_proj.weight", "blk.0.attn_k.weight", [kv_width, hidden], kv_heads),
        ("model.layers.0.self_attn.v_proj.weight", "blk.0.attn_v.weight", [kv_width, hidden], None),
        ("model.layers.0.self_attn.o_proj.weight", "blk.0.attn_output.weight", [hidden, hidden], None),
        ("model.layers.0.post_attention_layernorm.weight", "blk.0.ffn_norm.weight", [hidden], None),
        ("model.layers.0.mlp.gate_proj.weight", "blk.0.ffn_gate.weight", [intermediate, hidden], None),
        ("model.layers.0.mlp.down_proj.weight", "blk.0.ffn_down.weight", [hidden, intermediate], None),
        ("model.layers.0.mlp.up_proj.weight", "blk.0.ffn_up.weight", [intermediate, hidden], None),
    ]

    expected_names = {source for source, _, _, _ in specifications}
    actual_names = {name for name in header if name != "__metadata__"}
    if actual_names != expected_names:
        missing = sorted(expected_names - actual_names)
        extra = sorted(actual_names - expected_names)
        raise ValueError(f"Safetensors tensor set changed; missing={missing}, extra={extra}")

    tensors = []
    for source, destination, shape, permutation_heads in specifications:
        data = tensor_bytes(raw, header, data_start, source, shape)
        if permutation_heads is not None:
            data = permute_rope_rows(data, shape, permutation_heads)
        # GGML dimensions are fastest-moving first, the reverse of row-major shape.
        tensors.append(Tensor(destination, tuple(reversed(shape)), data))
    return tensors


def encoded_string(value: str) -> bytes:
    raw = value.encode("utf-8")
    return struct.pack("<Q", len(raw)) + raw


def metadata_scalar(key: str, value_type: int, payload: bytes) -> bytes:
    return encoded_string(key) + struct.pack("<I", value_type) + payload


def metadata_string(key: str, value: str) -> bytes:
    return metadata_scalar(key, TYPE_STRING, encoded_string(value))


def metadata_u32(key: str, value: int) -> bytes:
    return metadata_scalar(key, TYPE_UINT32, struct.pack("<I", value))


def metadata_f32(key: str, value: float) -> bytes:
    return metadata_scalar(key, TYPE_FLOAT32, struct.pack("<f", value))


def metadata_bool(key: str, value: bool) -> bytes:
    return metadata_scalar(key, TYPE_BOOL, struct.pack("<?", value))


def metadata_array(key: str, element_type: int, values: list[Any]) -> bytes:
    payload = bytearray(struct.pack("<IQ", element_type, len(values)))
    for value in values:
        if element_type == TYPE_STRING:
            payload.extend(encoded_string(value))
        elif element_type == TYPE_FLOAT32:
            payload.extend(struct.pack("<f", value))
        elif element_type == TYPE_INT32:
            payload.extend(struct.pack("<i", value))
        else:
            raise ValueError(f"unsupported fixture metadata array type {element_type}")
    return metadata_scalar(key, TYPE_ARRAY, bytes(payload))


def build_metadata(config: dict[str, Any]) -> list[bytes]:
    head_width = config["hidden_size"] // config["num_attention_heads"]
    return [
        metadata_string("general.architecture", "llama"),
        metadata_string("general.name", "llm-app tiny Llama F32 fixture"),
        metadata_string(
            "general.description",
            "Deterministic E0 native-backend integration fixture generated from the committed Candle Safetensors",
        ),
        metadata_string("general.author", "llm-app project"),
        metadata_string("general.license", "project-test-fixture"),
        metadata_u32("general.file_type", 0),
        metadata_u32("general.quantization_version", 2),
        metadata_u32("general.alignment", GGUF_ALIGNMENT),
        metadata_u32("llama.context_length", config["max_position_embeddings"]),
        metadata_u32("llama.embedding_length", config["hidden_size"]),
        metadata_u32("llama.block_count", config["num_hidden_layers"]),
        metadata_u32("llama.feed_forward_length", config["intermediate_size"]),
        metadata_u32("llama.attention.head_count", config["num_attention_heads"]),
        metadata_u32("llama.attention.head_count_kv", config["num_key_value_heads"]),
        metadata_f32("llama.attention.layer_norm_rms_epsilon", config["rms_norm_eps"]),
        metadata_u32("llama.rope.dimension_count", head_width),
        metadata_f32("llama.rope.freq_base", config["rope_theta"]),
        metadata_string("tokenizer.ggml.model", "llama"),
        metadata_string("tokenizer.ggml.pre", "default"),
        metadata_array("tokenizer.ggml.tokens", TYPE_STRING, TOKENS),
        metadata_array("tokenizer.ggml.scores", TYPE_FLOAT32, TOKEN_SCORES),
        metadata_array("tokenizer.ggml.token_type", TYPE_INT32, TOKEN_TYPES),
        metadata_u32("tokenizer.ggml.token_type_count", 7),
        metadata_u32("tokenizer.ggml.bos_token_id", config["bos_token_id"]),
        metadata_u32("tokenizer.ggml.eos_token_id", config["eos_token_id"]),
        metadata_u32("tokenizer.ggml.unknown_token_id", 0),
        metadata_u32("tokenizer.ggml.padding_token_id", 0),
        metadata_bool("tokenizer.ggml.add_bos_token", True),
        metadata_bool("tokenizer.ggml.add_eos_token", False),
        metadata_bool("tokenizer.ggml.add_space_prefix", True),
        metadata_bool("tokenizer.ggml.remove_extra_whitespaces", False),
    ]


def aligned(value: int) -> int:
    return (value + GGUF_ALIGNMENT - 1) // GGUF_ALIGNMENT * GGUF_ALIGNMENT


def build_gguf() -> bytes:
    config = checked_config(read_checked(CONFIG_PATH))
    safetensors = read_checked(SAFETENSORS_PATH)
    tensor_header, data_start = parse_safetensors(safetensors)
    tensors = build_tensors(config, safetensors, tensor_header, data_start)
    metadata = build_metadata(config)

    offsets: list[int] = []
    tensor_data = bytearray()
    for tensor in tensors:
        next_offset = aligned(len(tensor_data))
        tensor_data.extend(bytes(next_offset - len(tensor_data)))
        offsets.append(next_offset)
        tensor_data.extend(tensor.data)

    output = bytearray(b"GGUF")
    output.extend(struct.pack("<IQQ", GGUF_VERSION, len(tensors), len(metadata)))
    for entry in metadata:
        output.extend(entry)
    for tensor, offset in zip(tensors, offsets, strict=True):
        output.extend(encoded_string(tensor.name))
        output.extend(struct.pack("<I", len(tensor.dimensions)))
        for dimension in tensor.dimensions:
            output.extend(struct.pack("<Q", dimension))
        output.extend(struct.pack("<IQ", GGML_TYPE_F32, offset))

    data_offset = aligned(len(output))
    output.extend(bytes(data_offset - len(output)))
    output.extend(tensor_data)
    return bytes(output)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify that --output is exactly reproducible instead of rewriting it",
    )
    args = parser.parse_args()

    generated = build_gguf()
    digest = sha256(generated)
    if args.check:
        committed = args.output.read_bytes()
        if committed != generated:
            raise SystemExit(f"{args.output} is stale (generated SHA-256 {digest})")
        print(f"verified {args.output} ({len(generated)} bytes, SHA-256 {digest})")
        return

    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_bytes(generated)
    temporary.replace(args.output)
    print(f"wrote {args.output} ({len(generated)} bytes, SHA-256 {digest})")


if __name__ == "__main__":
    main()
