//! Drop guards.

use crate::{imports, sealed};

/// Trait for various drop behaviors for [`Resource`](crate::Resource)s. This constrains the second type param
/// in `Resource`.
///
/// The contents of this trait is an implementation detail. It cannot be implemented for external types.
///
/// Currently, 2 implementations are available:
///
/// - [`Register`] is the default implementation that implements RAII-style cleanup on drop, including
///   calling a customizable hook if one was supplied to the [`Processor`](crate::processor::Processor::set_drop_fn()).
/// - [`Forget`] is a no-op implementation corresponding to [`ResourceCopy`](crate::ResourceCopy).
///
/// See `Resource` and `ResourceCopy` docs for more context and examples of usage.
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
