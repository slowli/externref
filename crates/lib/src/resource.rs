//! `Resource` type.

use core::{
    alloc::Layout,
    hash::{Hash, Hasher},
    marker::PhantomData,
    mem, ptr,
};

use crate::{DropGuard, ExternRef, Forget, Register, imports};

/// Host resource exposed to WASM.
///
/// Internally, a resource is just an index into the `externref`s table; thus, it is completely
/// valid to store `Resource`s on heap (in a `Vec`, thread-local storage, etc.). The type param
/// can be used for type safety.
///
/// # Equality
///
/// `Resource` implements [`PartialEq`], [`Eq`] and [`Hash`] traits from the standard library,
/// in which it has pointer semantics (i.e., two `Resource`s are equal if they point to the same data,
/// which, since `Resource` [cannot be cloned](#cloning), means they are the same object). If you want to compare
/// the pointed-to content, it can be accomplished by wrapping a `Resource` into a higher-level abstraction
/// and implementing `PartialEq` / `Eq` / `Hash` / other traits e.g. by reading data from the host
/// or delegating comparison to the host.
///
/// # Cloning
///
/// By default, resources may have [configurable logic executed on drop](processor::Processor::set_drop_fn())
/// (e.g., to have RAII-style resource management on the host side). Dropping the resource also cleans up the resource slot
/// in the `externref` table.
/// Thus, `Resource` intentionally doesn't implement [`Clone`] or [`Copy`]. To clone such a resource,
/// you may use [`Rc`](std::rc::Rc), [`Arc`](std::sync::Arc) or another smart pointer.
///
/// As an alternative, you may use [`ResourceCopy`]. This is a version of `Resource` that does not
/// execute *any* logic on drop (not even cleaning up the `externref` table entry!). As a consequence,
/// `ResourceCopy` may be copied across the app.
///
/// # Examples
///
/// ## Cloning
///
/// In this scenario, the `Resource` is cloneable by wrapping it in an `Arc`. This retains RAII
/// resource management capabilities.
///
/// ```no_run
/// use externref::{externref, Resource};
/// use std::sync::Arc;
///
/// #[externref]
/// #[link(wasm_import_module = "data")]
/// unsafe extern "C" {
///     fn alloc_data(capacity: usize) -> Resource<SmartData>;
///
///     fn data_len(handle: &Resource<SmartData>) -> usize;
/// }
///
/// #[derive(Debug, Clone)]
/// pub struct SmartData {
///     // `Resource<Self>` is completely valid (doesn't lead to type size errors),
///     // and in fact is encouraged.
///     handle: Arc<Resource<Self>>,
/// }
///
/// impl SmartData {
///     fn new(capacity: usize) -> Self {
///         Self {
///             handle: Arc::new(unsafe { alloc_data(capacity) }),
///         }
///     }
///
///     fn len(&self) -> usize {
///         unsafe { data_len(&self.handle) }
///     }
/// }
/// ```
///
/// ## Implementing comparisons
///
/// This implements `Eq`, `Ord` and `Hash` traits for the *pointee* based on host imports.
///
/// ```no_run
/// use externref::{externref, Resource};
/// use core::{cmp, hash::{Hash, Hasher}};
///
/// #[externref]
/// #[link(wasm_import_module = "data")]
/// unsafe extern "C" {
///     /// Compares pointed-to data and returns -1 / 0 / 1.
///     fn compare(
///         lhs: &Resource<ComparableData>,
///         rhs: &Resource<ComparableData>,
///     ) -> isize;
///
///     /// Hashes the pointed-to data.
///     fn hash(data: &Resource<ComparableData>) -> u64;
/// }
///
/// #[derive(Debug)]
/// pub struct ComparableData {
///     handle: Resource<Self>,
/// }
///
/// impl PartialEq for ComparableData {
///     fn eq(&self, other: &Self) -> bool {
///         unsafe { compare(&self.handle, &other.handle) == 0 }
///     }
/// }
///
/// impl Eq for ComparableData {}
///
/// impl PartialOrd for ComparableData {
///     fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
///         Some(self.cmp(other))
///     }
/// }
///
/// impl Ord for ComparableData {
///     fn cmp(&self, other: &Self) -> cmp::Ordering {
///         let ordering = unsafe { compare(&self.handle, &other.handle) };
///         ordering.cmp(&0)
///     }
/// }
///
/// impl Hash for ComparableData {
///     fn hash<H: Hasher>(&self, hasher: &mut H) {
///         unsafe { hash(&self.handle) }.hash(hasher)
///     }
/// }
/// ```
#[derive(Debug)]
#[repr(C)]
pub struct Resource<T, D = Register> {
    drop_guard: D,
    _ty: PhantomData<fn(T)>,
}

/// [`Resource`] variation that can be copied.
///
/// # Cleanup
///
/// `ResourceCopy` **does not** clean up the `externref` table entry on drop. It can only be cleaned up
/// by the host side, or by implementing custom `Drop` logic for a higher-level `Resource` wrapper.
/// In the extreme case, when the WASM module is short-lived, garbage collection of dead `externref`s may
/// be summarily ignored.
///
/// For custom `Drop` logic, it may be useful to pass `ResourceCopy<_>` by value as a non-resource
/// (see the [`externref`](macro@externref) macro docs) as follows.
///
/// ```no_run
/// use externref::{externref, ResourceCopy};
///
/// #[externref]
/// // ^ In this particular case, this attribute may be skipped.
/// #[link(wasm_import_module = "data")]
/// unsafe extern "C" {
///     fn custom_drop(#[resource = false] data: ResourceCopy<CustomDrop>);
///     // The host will receive `data: usize` - the 0-based index
///     // of `externref` table entry the resource points to.
/// }
///
/// struct CustomDrop(ResourceCopy<Self>);
///
/// impl Drop for CustomDrop {
///     fn drop(&mut self) {
///         unsafe { custom_drop(self.0); }
///     }
/// }
/// ```
///
/// This relies on the fact that `ResourceCopy<_>` is guaranteed to have the identical layout as `usize`.
pub type ResourceCopy<T> = Resource<T, Forget>;

impl<T> Clone for ResourceCopy<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ResourceCopy<T> {}

#[doc(hidden)] // should only be used by macro-generated code
impl<T, D: DropGuard> Resource<T, D> {
    /// Creates a new resource converting it from.
    ///
    /// # Safety
    ///
    /// This method must be called with an `externref` obtained from the host (as a return
    /// type for an imported function or an argument for an exported function); it is not
    /// a "real" `usize`. The proper use is ensured by the [`externref`] macro.
    #[inline(always)]
    pub unsafe fn new(id: ExternRef) -> Option<Self> {
        let id = unsafe { imports::insert_externref(id) };
        if id == usize::MAX {
            None
        } else {
            Some(Self {
                drop_guard: D::from_id(id),
                _ty: PhantomData,
            })
        }
    }

    #[inline(always)]
    pub unsafe fn new_non_null(id: ExternRef) -> Self {
        let id = unsafe { imports::insert_externref(id) };
        assert!(
            id != usize::MAX,
            "Passed null `externref` as non-nullable arg"
        );
        Self {
            drop_guard: D::from_id(id),
            _ty: PhantomData,
        }
    }
}

impl<T> Resource<T> {
    /// Leaks this resource. Similarly to [`Box::leak()`] or [`mem::forget()`], this isn't unsafe,
    /// but may lead to resource starvation.
    pub fn leak(self) -> ResourceCopy<T> {
        let this = ResourceCopy {
            drop_guard: Forget::from_id(self.drop_guard.as_id()),
            _ty: PhantomData,
        };
        mem::forget(self.drop_guard);
        this
    }
}

#[doc(hidden)] // should only be used by macro-generated code
impl<T, D: DropGuard> Resource<T, D> {
    /// Obtains an `externref` from this resource.
    ///
    /// # Safety
    ///
    /// The returned value of this method must be passed as an `externref` to the host
    /// (as a return type of an exported function or an argument of the imported function);
    /// it is not a "real" `usize`. The proper use is ensured by the [`externref`] macro.
    #[inline(always)]
    pub unsafe fn raw(this: Option<&Self>) -> ExternRef {
        unsafe {
            imports::get_externref(match this {
                None => usize::MAX,
                Some(resource) => resource.drop_guard.as_id(),
            })
        }
    }

    /// Obtains an `externref` from this resource and drops the resource.
    #[inline(always)]
    #[allow(clippy::needless_pass_by_value)]
    pub unsafe fn take_raw(this: Option<Self>) -> ExternRef {
        unsafe {
            imports::get_externref(match &this {
                None => usize::MAX,
                Some(resource) => resource.drop_guard.as_id(),
            })
        }
    }
}

impl<T, D: DropGuard> Resource<T, D> {
    /// Upcasts this resource to a generic resource.
    pub fn upcast(self) -> Resource<(), D> {
        Resource {
            drop_guard: self.drop_guard,
            _ty: PhantomData,
        }
    }

    /// Upcasts a reference to this resource to a generic resource reference.
    #[allow(clippy::missing_panics_doc)] // sanity check; should never be triggered.
    pub fn upcast_ref(&self) -> &Resource<(), D> {
        assert_eq!(Layout::new::<Self>(), Layout::new::<Resource<(), D>>());

        let ptr = ptr::from_ref(self).cast::<Resource<(), D>>();
        unsafe {
            // SAFETY: All resource types have identical alignment (thanks to `repr(C)`),
            // hence, casting among them is safe.
            &*ptr
        }
    }
}

impl<D: DropGuard> Resource<(), D> {
    /// Downcasts this generic resource to a specific type.
    ///
    /// # Safety
    ///
    /// No checks are performed that the resource actually encapsulates what is meant
    /// by `Resource<T>`. It is up to the caller to check this beforehand (e.g., by calling
    /// a WASM import taking `&Resource<()>` and returning an app-specific resource kind).
    pub unsafe fn downcast_unchecked<T>(self) -> Resource<T, D> {
        Resource {
            drop_guard: self.drop_guard,
            _ty: PhantomData,
        }
    }
}

/// Compares resources by their pointers, similar to [`ptr::eq()`].
impl<T, D: DropGuard> PartialEq for Resource<T, D> {
    fn eq(&self, other: &Self) -> bool {
        self.drop_guard.as_id() == other.drop_guard.as_id()
    }
}

impl<T, D: DropGuard> Eq for Resource<T, D> {}

/// Hashes the resource based on its pointer, consistently with the [`PartialEq`] / [`Eq`] implementation.
impl<T, D: DropGuard> Hash for Resource<T, D> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.drop_guard.as_id().hash(state);
    }
}
