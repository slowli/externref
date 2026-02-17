//! Surrogate `ExternRef` value.

use core::fmt;

use crate::imports;

/// `externref` surrogate.
///
/// The post-processing logic replaces variables of this type with real `externref`s.
#[doc(hidden)] // should only be used by macro-generated code
#[repr(transparent)]
pub struct ExternRef(pub(crate) usize);

impl fmt::Debug for ExternRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("ExternRef").finish_non_exhaustive()
    }
}

impl ExternRef {
    /// Guard for imported function wrappers. The processor checks that each transformed function
    /// has this guard as the first instruction.
    ///
    /// # Safety
    ///
    /// This guard should only be inserted by the `externref` macro.
    #[inline(always)]
    pub unsafe fn guard() {
        unsafe {
            imports::externref_guard();
        }
    }
}
