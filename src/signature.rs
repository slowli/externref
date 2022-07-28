//! Function signatures recorded into a custom section of WASM modules.

use std::str;

use crate::error::{ReadError, ReadErrorKind};

/// Builder for [`BitSlice`]s that can be used in const contexts.
#[doc(hidden)] // not public yet
#[derive(Debug)]
pub struct BitSliceBuilder<const BYTES: usize> {
    bytes: [u8; BYTES],
    bit_len: usize,
}

#[doc(hidden)] // not public yet
impl<const BYTES: usize> BitSliceBuilder<BYTES> {
    #[must_use]
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

/// Slice of bits.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub struct BitSlice<'a> {
    bytes: &'a [u8],
    bit_len: usize,
}

impl BitSlice<'static> {
    #[doc(hidden)]
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
    /// Returns the number of bits in this slice.
    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    fn is_set(&self, idx: usize) -> bool {
        if idx > self.bit_len {
            return false;
        }
        let mask = 1 << (idx % 8);
        self.bytes[idx / 8] & mask > 0
    }

    /// Iterates over the indexes of set bits in this slice.
    pub fn set_indices(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.bit_len).filter(|&idx| self.is_set(idx))
    }

    fn read_from_section(buffer: &mut &'a [u8], context: &str) -> Result<Self, ReadError> {
        let bit_len = read_u32(buffer, || format!("length for {}", context))? as usize;
        let byte_len = (bit_len + 7) / 8;
        if buffer.len() < byte_len {
            Err(ReadErrorKind::UnexpectedEof.with_context(context))
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

fn read_u32(buffer: &mut &[u8], context: impl FnOnce() -> String) -> Result<u32, ReadError> {
    if buffer.len() < 4 {
        Err(ReadErrorKind::UnexpectedEof.with_context(context()))
    } else {
        let value = u32::from_le_bytes(buffer[..4].try_into().unwrap());
        *buffer = &buffer[4..];
        Ok(value)
    }
}

fn read_str<'a>(buffer: &mut &'a [u8], context: &str) -> Result<&'a str, ReadError> {
    let len = read_u32(buffer, || format!("length for {}", context))? as usize;
    if buffer.len() < len {
        Err(ReadErrorKind::UnexpectedEof.with_context(context))
    } else {
        let string = str::from_utf8(&buffer[..len])
            .map_err(|err| ReadErrorKind::Utf8(err).with_context(context))?;
        *buffer = &buffer[len..];
        Ok(string)
    }
}

/// Kind of a function with [`Resource`] args or return type.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum FunctionKind<'a> {
    /// Function exported from a WASM module.
    Export,
    /// Function imported to a WASM module from the module with the enclosed name.
    Import(&'a str),
}

impl<'a> FunctionKind<'a> {
    const fn len_in_custom_section(&self) -> usize {
        match self {
            Self::Export => 4,
            Self::Import(module_name) => 4 + module_name.len(),
        }
    }

    #[allow(clippy::cast_possible_truncation)] // `TryFrom` cannot be used in const fns
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

/// Information about a function with [`Resource`] args or return type.
///
/// This information is written to a custom section of a WASM module and is then used
/// during module [post-processing].
///
/// [post-processing]: https://docs.rs/externref-processor
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Function<'a> {
    /// Kind of this function.
    pub kind: FunctionKind<'a>,
    /// Name of this function.
    pub name: &'a str,
    /// Bit slice marking [`Resource`](crate::Resource) args / return type.
    pub externrefs: BitSlice<'a>,
}

impl<'a> Function<'a> {
    /// Computes length of a custom section for this function signature.
    #[doc(hidden)]
    pub const fn custom_section_len(&self) -> usize {
        self.kind.len_in_custom_section() + 4 + self.name.len() + 4 + self.externrefs.bytes.len()
    }

    #[doc(hidden)]
    #[allow(clippy::cast_possible_truncation)] // `TryFrom` cannot be used in const fns
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

    /// Reads function information from a WASM custom section.
    ///
    /// # Errors
    ///
    /// Returns an error if the custom section is malformed.
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
            const FUNCTION: $crate::Function = $signature;

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
