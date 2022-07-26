//! Function signatures.

use std::{
    error, fmt,
    str::{self, Utf8Error},
};

#[derive(Debug)]
pub struct BitSliceBuilder<const BYTES: usize> {
    bytes: [u8; BYTES],
    bit_len: usize,
}

impl<const BYTES: usize> BitSliceBuilder<BYTES> {
    pub const fn with_set_bit(mut self, bit_idx: usize) -> Self {
        assert!(bit_idx < self.bit_len);
        self.bytes[bit_idx / 8] |= 1 << (bit_idx % 8);
        self
    }

    pub const fn build(&self) -> BitSlice<'_> {
        BitSlice {
            bytes: &self.bytes,
            bit_len: self.bit_len,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub struct BitSlice<'a> {
    bytes: &'a [u8],
    bit_len: usize,
}

impl BitSlice<'static> {
    pub const fn builder<const BYTES: usize>(bit_len: usize) -> BitSliceBuilder<BYTES> {
        assert!(BYTES > 0);
        assert!(bit_len > (BYTES - 1) * 8 && bit_len <= BYTES * 8);
        BitSliceBuilder {
            bytes: [0_u8; BYTES],
            bit_len,
        }
    }
}

impl<'a> BitSlice<'a> {
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub fn is_set(&self, idx: usize) -> bool {
        if idx > self.bit_len {
            return false;
        }
        let mask = 1 << (idx % 8);
        self.bytes[idx / 8] & mask > 0
    }

    pub fn set_indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.bit_len).filter(|&idx| self.is_set(idx))
    }

    fn read_from_section(buffer: &mut &'a [u8], context: &str) -> Result<Self, ReadError> {
        let bit_len = read_u32(buffer, || format!("length for {}", context))? as usize;
        let byte_len = (bit_len + 7) / 8;
        if buffer.len() < byte_len {
            Err(ReadError {
                kind: ReadErrorKind::UnexpectedEof,
                context: context.to_owned(),
            })
        } else {
            let bytes = &buffer[..byte_len];
            *buffer = &buffer[byte_len..];
            Ok(Self { bytes, bit_len })
        }
    }
}

macro_rules! write_u32 {
    ($buffer:ident, $value:expr, $pos:expr) => {{
        let value: u32 = $value;
        let pos: usize = $pos;
        $buffer[pos] = (value & 0xff) as u8;
        $buffer[pos + 1] = ((value >> 8) & 0xff) as u8;
        $buffer[pos + 2] = ((value >> 16) & 0xff) as u8;
        $buffer[pos + 3] = ((value >> 24) & 0xff) as u8;
    }};
}

#[derive(Debug)]
enum ReadErrorKind {
    UnexpectedEof,
    Utf8(Utf8Error),
}

impl fmt::Display for ReadErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => formatter.write_str("reached end of input"),
            Self::Utf8(err) => write!(formatter, "{}", err),
        }
    }
}

#[derive(Debug)]
pub struct ReadError {
    kind: ReadErrorKind,
    context: String,
}

impl fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "failed reading {}: {}", self.context, self.kind)
    }
}

impl error::Error for ReadError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            ReadErrorKind::Utf8(err) => Some(err),
            _ => None,
        }
    }
}

fn read_u32(buffer: &mut &[u8], context: impl FnOnce() -> String) -> Result<u32, ReadError> {
    if buffer.len() < 4 {
        Err(ReadError {
            kind: ReadErrorKind::UnexpectedEof,
            context: context(),
        })
    } else {
        let value = u32::from_le_bytes(buffer[..4].try_into().unwrap());
        *buffer = &buffer[4..];
        Ok(value)
    }
}

fn read_str<'a>(buffer: &mut &'a [u8], context: &str) -> Result<&'a str, ReadError> {
    let len = read_u32(buffer, || format!("length for {}", context))? as usize;
    if buffer.len() < len {
        Err(ReadError {
            kind: ReadErrorKind::UnexpectedEof,
            context: context.to_owned(),
        })
    } else {
        let string = str::from_utf8(&buffer[..len]).map_err(|err| ReadError {
            kind: ReadErrorKind::Utf8(err),
            context: context.to_owned(),
        })?;
        *buffer = &buffer[len..];
        Ok(string)
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum FunctionKind<'a> {
    Export,
    Import(&'a str),
}

impl<'a> FunctionKind<'a> {
    pub fn module(&self) -> Option<&'a str> {
        match self {
            Self::Export => None,
            Self::Import(module) => Some(*module),
        }
    }

    const fn len_in_custom_section(&self) -> usize {
        match self {
            Self::Export => 4,
            Self::Import(module_name) => 4 + module_name.len(),
        }
    }

    const fn write_to_custom_section<const N: usize>(
        &self,
        mut buffer: [u8; N],
    ) -> ([u8; N], usize) {
        match self {
            Self::Export => {
                write_u32!(buffer, u32::MAX, 0);
                (buffer, 4)
            }

            Self::Import(module_name) => {
                write_u32!(buffer, module_name.len() as u32, 0);
                let mut pos = 4;
                while pos - 4 < module_name.len() {
                    buffer[pos] = module_name.as_bytes()[pos - 4];
                    pos += 1;
                }
                (buffer, pos)
            }
        }
    }

    fn read_from_section(buffer: &mut &'a [u8]) -> Result<Self, ReadError> {
        if buffer.len() >= 4 && buffer[..4] == [0xff; 4] {
            *buffer = &buffer[4..];
            Ok(Self::Export)
        } else {
            let module_name = read_str(buffer, "module name")?;
            Ok(Self::Import(module_name))
        }
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Function<'a> {
    pub kind: FunctionKind<'a>,
    pub name: &'a str,
    pub externrefs: BitSlice<'a>,
}

impl<'a> Function<'a> {
    /// Computes length of a custom section for this function signature.
    pub const fn custom_section_len(&self) -> usize {
        self.kind.len_in_custom_section() + 4 + self.name.len() + 4 + self.externrefs.bytes.len()
    }

    pub const fn custom_section<const N: usize>(&self) -> [u8; N] {
        debug_assert!(N == self.custom_section_len());
        let (mut buffer, mut pos) = self.kind.write_to_custom_section([0_u8; N]);

        write_u32!(buffer, self.name.len() as u32, pos);
        pos += 4;
        let mut i = 0;
        while i < self.name.len() {
            buffer[pos] = self.name.as_bytes()[i];
            pos += 1;
            i += 1;
        }

        write_u32!(buffer, self.externrefs.bit_len as u32, pos);
        pos += 4;
        let mut i = 0;
        while i < self.externrefs.bytes.len() {
            buffer[pos] = self.externrefs.bytes[i];
            i += 1;
            pos += 1;
        }

        buffer
    }

    pub fn read_from_section(buffer: &mut &'a [u8]) -> Result<Self, ReadError> {
        let kind = FunctionKind::read_from_section(buffer)?;
        Ok(Self {
            kind,
            name: read_str(buffer, "function name")?,
            externrefs: BitSlice::read_from_section(buffer, "externref bit slice")?,
        })
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! declare_function {
    ($signature:expr) => {
        const _: () = {
            const FUNCTION: $crate::signature::Function = $signature;

            #[cfg_attr(target_arch = "wasm32", link_section = "__externrefs")]
            static DATA_SECTION: [u8; FUNCTION.custom_section_len()] = FUNCTION.custom_section();
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_serialization() {
        const FUNCTION: Function = Function {
            kind: FunctionKind::Import("module"),
            name: "test",
            externrefs: BitSlice::builder::<1>(3).with_set_bit(1).build(),
        };

        const SECTION: [u8; FUNCTION.custom_section_len()] = FUNCTION.custom_section();

        assert_eq!(SECTION[..4], [6, 0, 0, 0]); // little-endian module name length
        assert_eq!(SECTION[4..10], *b"module");
        assert_eq!(SECTION[10..14], [4, 0, 0, 0]); // little-endian fn name length
        assert_eq!(SECTION[14..18], *b"test");
        assert_eq!(SECTION[18..22], [3, 0, 0, 0]); // little-endian bit slice length
        assert_eq!(SECTION[22], 2); // bit slice

        let mut section_reader = &SECTION as &[u8];
        let restored_function = Function::read_from_section(&mut section_reader).unwrap();
        assert_eq!(restored_function, FUNCTION);
    }

    #[test]
    fn export_fn_serialization() {
        const FUNCTION: Function = Function {
            kind: FunctionKind::Export,
            name: "test",
            externrefs: BitSlice::builder::<1>(3).with_set_bit(1).build(),
        };

        const SECTION: [u8; FUNCTION.custom_section_len()] = FUNCTION.custom_section();

        assert_eq!(SECTION[..4], [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(SECTION[4..8], [4, 0, 0, 0]); // little-endian fn name length
        assert_eq!(SECTION[8..12], *b"test");

        let mut section_reader = &SECTION as &[u8];
        let restored_function = Function::read_from_section(&mut section_reader).unwrap();
        assert_eq!(restored_function, FUNCTION);
    }
}
