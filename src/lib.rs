use core::marker::PhantomData;

#[doc(hidden)]
pub mod signature;

#[cfg(feature = "macro")]
pub use externref_macro::externref;

/// Host resource.
#[derive(Debug)]
pub struct Resource<T> {
    id: usize,
    _ty: PhantomData<fn(T)>,
}

impl<T> Resource<T> {
    /// # Safety
    ///
    /// FIXME
    #[inline(always)]
    pub unsafe fn new(id: usize) -> Self {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "insert"]
            fn insert_externref(id: usize) -> usize;
        }

        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn insert_externref(id: usize) -> usize {
            id
        }

        Self {
            id: insert_externref(id),
            _ty: PhantomData,
        }
    }

    /// # Safety
    ///
    /// FIXME
    #[inline(always)]
    pub unsafe fn as_raw(&self) -> usize {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "get"]
            fn get_externref(id: usize) -> usize;
        }

        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn get_externref(id: usize) -> usize {
            id
        }

        get_externref(self.id)
    }

    /// # Safety
    ///
    /// FIXME
    #[inline(always)]
    pub unsafe fn into_raw(self) -> usize {
        self.as_raw()
    }
}

impl<T> Drop for Resource<T> {
    #[inline(always)]
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "externref")]
        extern "C" {
            #[link_name = "drop"]
            fn drop_externref(id: usize);
        }

        #[cfg(not(target_arch = "wasm32"))]
        unsafe fn drop_externref(_id: usize) {
            // Do nothing
        }

        unsafe { drop_externref(self.id) };
    }
}
