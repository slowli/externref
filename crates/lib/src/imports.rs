//! WASM imports transformed by the processor.

use crate::ExternRef;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "externref")]
unsafe extern "C" {
    #[link_name = "get"]
    pub(crate) fn get_externref(id: usize) -> ExternRef;

    #[link_name = "insert"]
    pub(crate) fn insert_externref(id: ExternRef) -> usize;

    #[link_name = "drop"]
    pub(crate) fn drop_externref(id: usize);

    #[link_name = "guard"]
    pub(crate) fn externref_guard();
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) unsafe fn get_externref(id: usize) -> ExternRef {
    ExternRef(id)
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::needless_pass_by_value)]
pub(crate) unsafe fn insert_externref(id: ExternRef) -> usize {
    id.0
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) unsafe fn drop_externref(_id: usize) {
    // Do nothing
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) unsafe fn externref_guard() {
    // Do nothing
}
