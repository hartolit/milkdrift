//! GGUF tokenizer identity and portable-contract tests.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::num::{NonZeroI32, NonZeroU32};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};

use domain_contracts::TokenId;
use gguf_backend::{
    GgufBackendRuntime, GgufExecutionConfiguration, GgufSource, GgufTokenizer,
    GgufTokenizerLoadError, Sha256Digest, sha256_digest, sha256_file,
};
use tokenization::{
    ByteBuffer, DecodeOptions, EncodeOptions, SpecialTokenPolicy, StreamingTokenizer, TokenBuffer,
    TokenizationError, Tokenizer,
};

static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const MULTI_BLOCK_SHA256: &str = "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1";
const FIXTURE_RELATIVE_PATH: &str =
    "../../runtime/inference-runtime/tests/fixtures/gguf-llama/tiny-llama-f32.gguf";

type TestResult = Result<(), Box<dyn Error + Send + Sync>>;

#[test]
fn computes_and_parses_standard_sha256_vector() -> TestResult {
    assert_eq!(sha256_digest(b"").to_string(), EMPTY_SHA256);
    let digest = sha256_digest(b"abc");
    assert_eq!(digest.to_string(), ABC_SHA256);
    assert_eq!(
        sha256_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").to_string(),
        MULTI_BLOCK_SHA256
    );
    assert_eq!(Sha256Digest::from_hex(ABC_SHA256)?, digest);
    assert_eq!(ABC_SHA256.parse::<Sha256Digest>()?, digest);
    assert!(Sha256Digest::from_hex("not-a-digest").is_err());
    Ok(())
}

#[test]
fn hashes_file_content_with_the_same_identity() -> TestResult {
    let path = temporary_path("digest");
    fs::write(&path, b"abc")?;
    let digest = sha256_file(&path);
    fs::remove_file(&path)?;
    assert_eq!(digest?, sha256_digest(b"abc"));
    Ok(())
}

#[test]
fn source_digest_is_optional_and_can_be_required_without_breaking_new() -> TestResult {
    let execution = execution_configuration()?;
    let digest = sha256_digest(b"model identity");

    let legacy = GgufSource::new("model.gguf", execution);
    assert_eq!(legacy.expected_digest(), None);

    let upgraded = legacy.with_expected_digest(digest);
    assert_eq!(upgraded.expected_digest(), Some(digest));

    let verified = GgufSource::new_verified("model.gguf", execution, digest);
    assert_eq!(verified.expected_digest(), Some(digest));
    Ok(())
}

#[test]
fn exercises_native_tokenizer_when_fixture_is_present() -> TestResult {
    let path = fixture_path();
    assert!(path.is_file(), "shared GGUF fixture is missing");

    let expected = sha256_file(&path)?;
    let runtime = GgufBackendRuntime::initialize()?;
    let tokenizer = GgufTokenizer::from_file_verified(runtime.clone(), &path, expected)?;
    let cloned = tokenizer.clone();

    assert_eq!(tokenizer.content_digest(), expected);
    assert_eq!(cloned.content_digest(), expected);
    assert!(tokenizer.vocabulary_size() > 0);
    assert_eq!(cloned.vocabulary_size(), tokenizer.vocabulary_size());
    assert_boundary_evidence(&tokenizer)?;
    assert_special_policy_if_available(&tokenizer)?;

    let owned = tokenizer.owned_decoder(DecodeOptions {
        skip_special_tokens: true,
    });
    assert_eq!(owned.pending_bytes(), 0);
    tokenization(owned.finish())?;
    assert_eq!(owned.tokenizer().content_digest(), expected);

    let borrowed = tokenizer.decoder(DecodeOptions::default());
    assert_eq!(borrowed.pending_bytes(), 0);
    tokenization(borrowed.finish())?;

    let unknown = TokenId::new(tokenizer.vocabulary_size());
    let mut empty_storage = [];
    let mut output = ByteBuffer::new(&mut empty_storage);
    assert_eq!(
        tokenizer.decode_token(unknown, DecodeOptions::default(), &mut output),
        Err(TokenizationError::UnknownToken(unknown))
    );

    let mut wrong_bytes = expected.into_bytes();
    if let Some(first) = wrong_bytes.first_mut() {
        *first ^= 0xff;
    }
    let wrong = Sha256Digest::from_bytes(wrong_bytes);
    assert!(matches!(
        GgufTokenizer::from_file_verified(runtime, &path, wrong),
        Err(GgufTokenizerLoadError::DigestMismatch {
            expected: mismatch,
            actual
        }) if mismatch == wrong && actual == expected
    ));
    Ok(())
}

fn assert_boundary_evidence(tokenizer: &GgufTokenizer) -> TestResult {
    let mut storage = [TokenId::default(); 8];
    let mut output = TokenBuffer::new(&mut storage);
    match tokenizer.bos_token_id() {
        Some(bos) => {
            let report = tokenization(tokenizer.encode(
                "",
                EncodeOptions {
                    special_tokens: SpecialTokenPolicy::Allow,
                    add_beginning_of_sequence: true,
                    add_end_of_sequence: false,
                },
                &mut output,
            ))?;
            assert!(report.tokens_written >= 1);
            assert_eq!(output.as_slice().first(), Some(&bos));
        }
        None => assert_eq!(
            tokenizer.encode(
                "",
                EncodeOptions {
                    special_tokens: SpecialTokenPolicy::Allow,
                    add_beginning_of_sequence: true,
                    add_end_of_sequence: false,
                },
                &mut output,
            ),
            Err(TokenizationError::Implementation {
                code: GgufTokenizer::ERROR_MISSING_BOS
            })
        ),
    }

    output.clear();
    match tokenizer.eos_token_id() {
        Some(eos) => {
            let report = tokenization(tokenizer.encode(
                "",
                EncodeOptions {
                    special_tokens: SpecialTokenPolicy::Allow,
                    add_beginning_of_sequence: false,
                    add_end_of_sequence: true,
                },
                &mut output,
            ))?;
            assert!(report.tokens_written >= 1);
            assert_eq!(output.as_slice().last(), Some(&eos));
        }
        None => assert_eq!(
            tokenizer.encode(
                "",
                EncodeOptions {
                    special_tokens: SpecialTokenPolicy::Allow,
                    add_beginning_of_sequence: false,
                    add_end_of_sequence: true,
                },
                &mut output,
            ),
            Err(TokenizationError::Implementation {
                code: GgufTokenizer::ERROR_MISSING_EOS
            })
        ),
    }
    Ok(())
}

fn assert_special_policy_if_available(tokenizer: &GgufTokenizer) -> TestResult {
    let Some((token, spelling)) = first_renderable_special(tokenizer) else {
        return Ok(());
    };
    if spelling.contains('\0') {
        return Ok(());
    }

    assert!(tokenizer.token_id(&spelling).is_some());

    let mut allowed_storage = [TokenId::default(); 8];
    let mut allowed = TokenBuffer::new(&mut allowed_storage);
    let report = tokenization(tokenizer.encode(
        &spelling,
        EncodeOptions {
            special_tokens: SpecialTokenPolicy::Allow,
            add_beginning_of_sequence: false,
            add_end_of_sequence: false,
        },
        &mut allowed,
    ))?;
    assert!(report.tokens_written >= 1);

    let mut rejected_storage = [TokenId::default(); 8];
    let mut rejected = TokenBuffer::new(&mut rejected_storage);
    assert_eq!(
        tokenizer.encode(
            &spelling,
            EncodeOptions {
                special_tokens: SpecialTokenPolicy::OrdinaryText,
                add_beginning_of_sequence: false,
                add_end_of_sequence: false,
            },
            &mut rejected,
        ),
        Err(TokenizationError::Implementation {
            code: GgufTokenizer::ERROR_ORDINARY_TEXT_SPECIAL
        })
    );
    assert_eq!(
        tokenizer.encode(
            &spelling,
            EncodeOptions {
                special_tokens: SpecialTokenPolicy::Reject,
                add_beginning_of_sequence: false,
                add_end_of_sequence: false,
            },
            &mut rejected,
        ),
        Err(TokenizationError::Implementation {
            code: GgufTokenizer::ERROR_REJECTED_SPECIAL
        })
    );

    let mut empty_storage = [];
    let mut skipped = ByteBuffer::new(&mut empty_storage);
    let report = tokenization(tokenizer.decode_token(
        token,
        DecodeOptions {
            skip_special_tokens: true,
        },
        &mut skipped,
    ))?;
    assert!(report.skipped_special_token);
    assert_eq!(report.bytes_written, 0);
    Ok(())
}

fn first_renderable_special(tokenizer: &GgufTokenizer) -> Option<(TokenId, String)> {
    for identifier in 0..tokenizer.vocabulary_size() {
        let token = TokenId::new(identifier);
        if !tokenizer.is_special_token(token) {
            continue;
        }

        let mut storage = vec![0_u8; 16 * 1024];
        let mut output = ByteBuffer::new(storage.as_mut_slice());
        let Ok(report) = tokenizer.decode_token(token, DecodeOptions::default(), &mut output)
        else {
            continue;
        };
        let Some(bytes) = storage.get(..report.bytes_written) else {
            continue;
        };
        let Ok(spelling) = str::from_utf8(bytes) else {
            continue;
        };
        if !spelling.is_empty() {
            return Some((token, spelling.to_owned()));
        }
    }
    None
}

fn tokenization<T>(result: Result<T, TokenizationError>) -> Result<T, TestTokenizationError> {
    result.map_err(TestTokenizationError)
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_RELATIVE_PATH)
}

fn temporary_path(label: &str) -> PathBuf {
    let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "llm-app-gguf-tokenizer-{label}-{}-{sequence}",
        std::process::id()
    ))
}

fn execution_configuration() -> Result<GgufExecutionConfiguration, TestInvariantError> {
    GgufExecutionConfiguration::new(
        non_zero_u32(8)?,
        non_zero_u32(8)?,
        non_zero_u32(4)?,
        non_zero_u32(1)?,
        non_zero_i32(1)?,
        non_zero_i32(1)?,
    )
    .map_err(|_| TestInvariantError)
}

fn non_zero_u32(value: u32) -> Result<NonZeroU32, TestInvariantError> {
    NonZeroU32::new(value).ok_or(TestInvariantError)
}

fn non_zero_i32(value: i32) -> Result<NonZeroI32, TestInvariantError> {
    NonZeroI32::new(value).ok_or(TestInvariantError)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TestTokenizationError(TokenizationError);

impl Display for TestTokenizationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "tokenization failed: {:?}", self.0)
    }
}

impl Error for TestTokenizationError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TestInvariantError;

impl Display for TestInvariantError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("test requested an invalid non-zero integer or execution bound")
    }
}

impl Error for TestInvariantError {}
