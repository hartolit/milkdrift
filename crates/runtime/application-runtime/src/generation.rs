//! Frontend-neutral direct-completion admission, decoding, and bounded text output.

mod admission;
mod bridge;
mod output;
mod settings;

pub use admission::encode_text_with_policy;
pub use bridge::GenerationBridge;
pub use output::{
    ApplicationOutputBatch, ApplicationOutputRecord, ApplicationOutputRecordKind,
    ApplicationOutputState, ApplicationTextRange, GenerationTerminalKind,
};
pub use settings::{GenerationSeed, GenerationSettings};
