//! Drop guards.

use crate::{imports, sealed};

/// Trait for various drop behaviors for [`Resource`]s.
pub trait DropGuard: sealed::Sealed {
    #[doc(hidden)] // implementation detail
    fn from_id(id: usize) -> Self;
    #[doc(hidden)] // implementation detail
    fn as_id(&self) -> usize;
}

/// No-op [`DropGuard`] implementation.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Forget(usize);

impl sealed::Sealed for Forget {}

impl DropGuard for Forget {
    fn from_id(id: usize) -> Self {
        Self(id)
    }

    fn as_id(&self) -> usize {
        self.0
    }
}

/// [`DropGuard`] implementation that triggers `externref` table cleanup (and potentially a user-defined
/// drop hook) on drop.
#[derive(Debug)]
#[repr(C)]
pub struct Register(usize);

impl sealed::Sealed for Register {}

impl DropGuard for Register {
    fn from_id(id: usize) -> Self {
        Self(id)
    }

    fn as_id(&self) -> usize {
        self.0
    }
}

impl Drop for Register {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe { imports::drop_externref(self.0) };
    }
}
