//! Function signatures recorded into a custom section of WASM modules.

use core::str;

use crate::{
    alloc::{String, format},
    error::{ReadError, ReadErrorKind},
};

/// Type information needed to transform a function argument or result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ValueType {
    /// A value that is not an external reference; its WASM type is unchanged.
    Other = 0,
    /// A non-null external reference, `(ref extern)`.
    NonNullExternref = 1,
    /// A nullable external reference, `(ref null extern)`.
    NullableExternref = 2,
}

/// Const builder for packed function types.
#[doc(hidden)]
#[derive(Debug)]
pub struct TypeSliceBuilder<const BYTES: usize> {
    bytes: [u8; BYTES],
    len: usize,
}

#[doc(hidden)]
impl<const BYTES: usize> TypeSliceBuilder<BYTES> {
    #[must_use]
    pub const fn with_type(mut self, idx: usize, ty: ValueType) -> Self {
        assert!(idx < self.len);
        let shift = (idx % 4) * 2;
        self.bytes[idx / 4] = (self.bytes[idx / 4] & !(3 << shift)) | ((ty as u8) << shift);
        self
    }

    pub const fn build(&self) -> TypeSlice<'_> {
        TypeSlice {
            bytes: &self.bytes,
            len: self.len,
        }
    }
}

/// Function argument and result types packed into two bits per value.
///
/// Each byte stores up to four types, starting with its least significant bits:
/// `00` is an ordinary value, `01` is non-null, and `10` is nullable. `11` is invalid,
/// and unused bits in the last byte must be zero. The serialized slice starts with
/// a little-endian `u32` type count, followed by the packed bytes.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct TypeSlice<'a> {
    bytes: &'a [u8],
    len: usize,
}

impl TypeSlice<'static> {
    #[doc(hidden)]
    pub const fn builder<const BYTES: usize>(len: usize) -> TypeSliceBuilder<BYTES> {
        assert!(BYTES == len.div_ceil(4));
        TypeSliceBuilder {
            bytes: [0; BYTES],
            len,
        }
    }
}

impl<'a> TypeSlice<'a> {
    /// Returns the number of types in this slice.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether this slice contains no types.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns a type by its zero-based index.
    pub fn get(&self, idx: usize) -> Option<ValueType> {
        if idx >= self.len {
            return None;
        }
        Some(self.type_at(idx))
    }

    fn type_at(&self, idx: usize) -> ValueType {
        match (self.bytes[idx / 4] >> ((idx % 4) * 2)) & 3 {
            0 => ValueType::Other,
            1 => ValueType::NonNullExternref,
            2 => ValueType::NullableExternref,
            _ => unreachable!(),
        }
    }

    /// Iterates over the types in this slice.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = ValueType> + '_ {
        (0..self.len).map(|idx| self.type_at(idx))
    }

    fn read_from_section(buffer: &mut &'a [u8], context: &str) -> Result<Self, ReadError> {
        let len = read_u32(buffer, || format!("length for {context}"))? as usize;
        let byte_len = len.div_ceil(4);
        if buffer.len() < byte_len {
            return Err(ReadErrorKind::UnexpectedEof.with_context(context));
        }
        let bytes = &buffer[..byte_len];
        for (byte_idx, &byte) in bytes.iter().enumerate() {
            for slot in 0..4 {
                let tag = (byte >> (slot * 2)) & 3;
                if tag == 3 || (byte_idx * 4 + slot >= len && tag != 0) {
                    return Err(ReadErrorKind::InvalidTypeEncoding.with_context(context));
                }
            }
        }
        *buffer = &buffer[byte_len..];
        Ok(Self { bytes, len })
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
    let len = read_u32(buffer, || format!("length for {context}"))? as usize;
    if buffer.len() < len {
        Err(ReadErrorKind::UnexpectedEof.with_context(context))
    } else {
        let string = str::from_utf8(&buffer[..len])
            .map_err(|err| ReadErrorKind::Utf8(err).with_context(context))?;
        *buffer = &buffer[len..];
        Ok(string)
    }
}

/// Kind of a function with [`Resource`](crate::Resource) args or return type.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
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

/// Information about a function with [`Resource`](crate::Resource) args or return type.
///
/// This information is written to a custom section of a WASM module and is then used
/// during module [post-processing].
///
/// [post-processing]: crate::processor
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub struct Function<'a> {
    /// Kind of this function.
    pub kind: FunctionKind<'a>,
    /// Name of this function.
    pub name: &'a str,
    /// Packed argument and result types, including external reference nullability.
    pub types: TypeSlice<'a>,
}

impl<'a> Function<'a> {
    /// Name of a custom section in WASM modules where `Function` declarations are stored.
    /// `Function`s can be read from this section using [`Self::read_from_section()`].
    // **NB.** Keep synced with the `declare_function!()` macro below.
    pub const CUSTOM_SECTION_NAME: &'static str = "__externrefs";

    /// Computes length of a custom section for this function signature.
    #[doc(hidden)]
    pub const fn custom_section_len(&self) -> usize {
        self.kind.len_in_custom_section() + 4 + self.name.len() + 4 + self.types.bytes.len()
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

        write_u32!(buffer, self.types.len as u32, pos);
        pos += 4;
        let mut i = 0;
        while i < self.types.bytes.len() {
            buffer[pos] = self.types.bytes[i];
            i += 1;
            pos += 1;
        }

        buffer
    }

    /// Reads function information from a WASM custom section. After reading, the `buffer`
    /// is advanced to trim the bytes consumed by the parser.
    ///
    /// This crate does not provide tools to read custom sections from a WASM module;
    /// use a library like [`walrus`] or [`wasmparser`] for this purpose.
    ///
    /// # Errors
    ///
    /// Returns an error if the custom section is malformed.
    ///
    /// [`walrus`]: https://docs.rs/walrus/
    /// [`wasmparser`]: https://docs.rs/wasmparser/
    pub fn read_from_section(buffer: &mut &'a [u8]) -> Result<Self, ReadError> {
        let kind = FunctionKind::read_from_section(buffer)?;
        Ok(Self {
            kind,
            name: read_str(buffer, "function name")?,
            types: TypeSlice::read_from_section(buffer, "function types")?,
        })
    }
}

#[macro_export]
#[doc(hidden)]
macro_rules! declare_function {
    ($signature:expr) => {
        const _: () = {
            const FUNCTION: $crate::Function = $signature;

            #[cfg_attr(target_arch = "wasm32", unsafe(link_section = "__externrefs"))]
            static DATA_SECTION: [u8; FUNCTION.custom_section_len()] = FUNCTION.custom_section();
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_types_encode_nullability() {
        const TYPES: TypeSlice<'static> = TypeSlice::builder::<2>(5)
            .with_type(0, ValueType::NullableExternref)
            .with_type(1, ValueType::NonNullExternref)
            .with_type(4, ValueType::NonNullExternref)
            .build();

        assert_eq!(TYPES.bytes, [0b0000_0110, 0b0000_0001]);
        assert_eq!(TYPES.len(), 5);
        assert_eq!(TYPES.get(0), Some(ValueType::NullableExternref));
        assert_eq!(TYPES.get(1), Some(ValueType::NonNullExternref));
        assert_eq!(TYPES.get(2), Some(ValueType::Other));
        assert_eq!(TYPES.get(4), Some(ValueType::NonNullExternref));
        assert_eq!(TYPES.get(5), None);
        assert_eq!(TYPES.get(usize::MAX), None);
    }

    #[test]
    fn packed_type_builder_replaces_previous_type() {
        const TYPES: TypeSlice<'static> = TypeSlice::builder::<1>(1)
            .with_type(0, ValueType::NonNullExternref)
            .with_type(0, ValueType::NullableExternref)
            .build();
        assert_eq!(TYPES.bytes, [2]);
    }

    #[test]
    fn invalid_packed_types_are_rejected() {
        for tag in [3, 4, 128] {
            let bytes = [1, 0, 0, 0, tag];
            let error = TypeSlice::read_from_section(&mut bytes.as_slice(), "types").unwrap_err();
            assert!(format!("{error}").contains("invalid packed type encoding"));
        }
        let truncated = [5, 0, 0, 0, 0];
        assert!(TypeSlice::read_from_section(&mut truncated.as_slice(), "types").is_err());
        let invalid_second_byte = [5, 0, 0, 0, 0, 3];
        assert!(
            TypeSlice::read_from_section(&mut invalid_second_byte.as_slice(), "types").is_err()
        );
    }

    #[test]
    fn concatenated_signatures_preserve_type_boundaries() {
        const IMPORT: Function = Function {
            kind: FunctionKind::Import("test"),
            name: "mixed",
            types: TypeSlice::builder::<2>(5)
                .with_type(0, ValueType::NullableExternref)
                .with_type(3, ValueType::NonNullExternref)
                .with_type(4, ValueType::NullableExternref)
                .build(),
        };
        const EXPORT: Function = Function {
            kind: FunctionKind::Export,
            name: "empty",
            types: TypeSlice::builder::<0>(0).build(),
        };
        const IMPORT_BYTES: [u8; IMPORT.custom_section_len()] = IMPORT.custom_section();
        const EXPORT_BYTES: [u8; EXPORT.custom_section_len()] = EXPORT.custom_section();
        let section = [IMPORT_BYTES.as_slice(), EXPORT_BYTES.as_slice()].concat();
        let mut reader = section.as_slice();
        assert_eq!(Function::read_from_section(&mut reader).unwrap(), IMPORT);
        let export = Function::read_from_section(&mut reader).unwrap();
        assert_eq!(export, EXPORT);
        assert!(export.types.is_empty());
        assert_eq!(export.types.get(0), None);
        assert!(reader.is_empty());
    }

    #[test]
    fn function_serialization() {
        const FUNCTION: Function = Function {
            kind: FunctionKind::Import("module"),
            name: "test",
            types: TypeSlice::builder::<1>(3)
                .with_type(1, ValueType::NonNullExternref)
                .with_type(2, ValueType::NullableExternref)
                .build(),
        };

        const SECTION: [u8; FUNCTION.custom_section_len()] = FUNCTION.custom_section();

        assert_eq!(SECTION[..4], [6, 0, 0, 0]); // little-endian module name length
        assert_eq!(SECTION[4..10], *b"module");
        assert_eq!(SECTION[10..14], [4, 0, 0, 0]); // little-endian fn name length
        assert_eq!(SECTION[14..18], *b"test");
        assert_eq!(SECTION[18..22], [3, 0, 0, 0]);
        assert_eq!(SECTION[22], 0b0010_0100);

        let mut section_reader = &SECTION as &[u8];
        let restored_function = Function::read_from_section(&mut section_reader).unwrap();
        assert_eq!(restored_function, FUNCTION);
    }

    #[test]
    fn export_fn_serialization() {
        const FUNCTION: Function = Function {
            kind: FunctionKind::Export,
            name: "test",
            types: TypeSlice::builder::<1>(3)
                .with_type(1, ValueType::NonNullExternref)
                .build(),
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
