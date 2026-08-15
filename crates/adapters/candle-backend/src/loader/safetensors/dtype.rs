use candle_core::DType;
use domain_contracts::ScalarType;
use safetensors::tensor::Dtype as SafeDtype;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceTensorDType {
    Bool,
    F4,
    F6E2M3,
    F6E3M2,
    U8,
    I8,
    F8E5M2,
    F8E4M3,
    F8E8M0,
    F8E4M3Fnuz,
    F8E5M2Fnuz,
    I16,
    U16,
    F16,
    Bf16,
    I32,
    U32,
    F32,
    C64,
    F64,
    I64,
    U64,
}

impl SourceTensorDType {
    pub(crate) const fn from_safetensors(dtype: SafeDtype) -> Option<Self> {
        match dtype {
            SafeDtype::BOOL => Some(Self::Bool),
            SafeDtype::F4 => Some(Self::F4),
            SafeDtype::F6_E2M3 => Some(Self::F6E2M3),
            SafeDtype::F6_E3M2 => Some(Self::F6E3M2),
            SafeDtype::U8 => Some(Self::U8),
            SafeDtype::I8 => Some(Self::I8),
            SafeDtype::F8_E5M2 => Some(Self::F8E5M2),
            SafeDtype::F8_E4M3 => Some(Self::F8E4M3),
            SafeDtype::F8_E8M0 => Some(Self::F8E8M0),
            SafeDtype::F8_E4M3FNUZ => Some(Self::F8E4M3Fnuz),
            SafeDtype::F8_E5M2FNUZ => Some(Self::F8E5M2Fnuz),
            SafeDtype::I16 => Some(Self::I16),
            SafeDtype::U16 => Some(Self::U16),
            SafeDtype::F16 => Some(Self::F16),
            SafeDtype::BF16 => Some(Self::Bf16),
            SafeDtype::I32 => Some(Self::I32),
            SafeDtype::U32 => Some(Self::U32),
            SafeDtype::F32 => Some(Self::F32),
            SafeDtype::C64 => Some(Self::C64),
            SafeDtype::F64 => Some(Self::F64),
            SafeDtype::I64 => Some(Self::I64),
            SafeDtype::U64 => Some(Self::U64),
            _ => None,
        }
    }

    pub(crate) const fn scalar_type(self) -> ScalarType {
        match self {
            Self::F32 => ScalarType::F32,
            Self::F16 => ScalarType::F16,
            Self::Bf16 => ScalarType::Bf16,
            Self::I8 => ScalarType::I8,
            Self::U8 => ScalarType::U8,
            Self::Bool => ScalarType::Other(1),
            Self::F4 => ScalarType::Other(2),
            Self::F6E2M3 => ScalarType::Other(3),
            Self::F6E3M2 => ScalarType::Other(4),
            Self::F8E5M2 => ScalarType::Other(5),
            Self::F8E4M3 => ScalarType::Other(6),
            Self::F8E8M0 => ScalarType::Other(7),
            Self::F8E4M3Fnuz => ScalarType::Other(8),
            Self::F8E5M2Fnuz => ScalarType::Other(9),
            Self::I16 => ScalarType::Other(10),
            Self::U16 => ScalarType::Other(11),
            Self::I32 => ScalarType::Other(12),
            Self::U32 => ScalarType::Other(13),
            Self::C64 => ScalarType::Other(14),
            Self::F64 => ScalarType::Other(15),
            Self::I64 => ScalarType::Other(16),
            Self::U64 => ScalarType::Other(17),
        }
    }

    pub(crate) const fn executable_dtype(self) -> Option<DType> {
        match self {
            Self::F32 => Some(DType::F32),
            Self::F16 => Some(DType::F16),
            Self::Bf16 => Some(DType::BF16),
            _ => None,
        }
    }

    pub(crate) const fn alignment(self) -> Option<u64> {
        match self {
            Self::F32 => Some(4),
            Self::F16 | Self::Bf16 => Some(2),
            _ => None,
        }
    }

    pub(crate) const fn bits_per_element(self) -> u64 {
        match self {
            Self::F4 => 4,
            Self::F6E2M3 | Self::F6E3M2 => 6,
            Self::Bool
            | Self::U8
            | Self::I8
            | Self::F8E5M2
            | Self::F8E4M3
            | Self::F8E8M0
            | Self::F8E4M3Fnuz
            | Self::F8E5M2Fnuz => 8,
            Self::I16 | Self::U16 | Self::F16 | Self::Bf16 => 16,
            Self::I32 | Self::U32 | Self::F32 => 32,
            Self::C64 | Self::F64 | Self::I64 | Self::U64 => 64,
        }
    }
}
