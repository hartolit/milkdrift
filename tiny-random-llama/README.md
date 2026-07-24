[Commands]:
export LLM_APP_CANDLE_MODEL_DIR="$MODEL_DIR"
export LLM_APP_CANDLE_MODEL_REVISION="$MODEL_REVISION"
export LLM_APP_CANDLE_PROMPT_TOKENS="1,2,3"

cargo run --locked \
  -p inference-runtime \
  --example candle_llama_smoke


[OUTPUT]:
[hartolit@hart-desk llm-app]$ printf '%s  %s\n' \
  '49c20f32c6c597480fcaec5df2f86c645eabea765cbea1e67886dbae45e5c992' \
  "$MODEL_DIR/model.safetensors" \
  | sha256sum --check --strict -
/home/hartolit/Projects/dev/llm-app/.phase4/tiny-random-llama/model.safetensors: OK
[hartolit@hart-desk llm-app]$ export LLM_APP_CANDLE_MODEL_DIR="$MODEL_DIR"
export LLM_APP_CANDLE_MODEL_REVISION="$MODEL_REVISION"
export LLM_APP_CANDLE_PROMPT_TOKENS="1,2,3"

cargo run --locked \
  -p inference-runtime \
  --example candle_llama_smoke
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.20s
     Running `target/debug/examples/candle_llama_smoke`
model repository: neubla/tiny-random-LlamaForCausalLM
model revision: 39ca1f8a1fc940377c5cb49a21aff73bb99b52f5
expected architecture: LlamaForCausalLM/Llama
prompt token count: 3
generated token ids: [TokenId(18568), TokenId(1727), TokenId(8705), TokenId(3598), TokenId(27426), TokenId(4496), TokenId(998), TokenId(16911)]
model load duration: 0.005661 s
time to first generated token: 0.060969 s
decode tokens per second: 21.954
cancellation latency: 0.045297 s
model unload duration: 0.000380 s
process RSS before load: 4636 KiB
process RSS after load: 11116 KiB
process RSS during generation: 14088 KiB
process RSS after unload: 10412 KiB
[hartolit@hart-desk llm-app]$
