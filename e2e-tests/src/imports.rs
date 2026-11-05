use externref::{Resource, ResourceCopy};

use crate::{Bytes, Sender};

#[externref::externref(stubs)]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    pub(crate) fn send_message(
        #[resource] sender: &Resource<Sender>,
        message_ptr: *const u8,
        message_len: usize,
    ) -> Resource<Bytes>;

    pub(crate) fn send_message_non_null(
        sender: &mut Resource<Sender>,
        message_ptr: *const u8,
        message_len: usize,
    ) -> Resource<Bytes>;

    pub(crate) fn message_len(bytes: Option<&Resource<Bytes>>) -> usize;

    pub(crate) fn round_trip(
        value: Resource<Sender>,
        reference: &Resource<Sender>,
        mut_reference: &mut Resource<Sender>,
        optional_value: Option<Resource<Sender>>,
        optional_reference: Option<&Resource<Sender>>,
        optional_mut_reference: Option<&mut Resource<Sender>>,
    ) -> Option<Resource<Sender>>;

    /// Inspects the pointer to the `Resource` rather than using the resource itself.
    /// This is unusual but valid resource usage.
    pub(crate) fn inspect_message_ref(#[resource = false] bytes: &Resource<Bytes>);

    #[link_name = "inspect_refs"]
    pub(crate) fn inspect_refs_on_host();
}

// ANCHOR: imports_copy
type MessageCopy = ResourceCopy<Bytes>;

#[externref::externref(stubs(target_arch = "wasm32"))]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    #[resource]
    pub(crate) fn send_message_copy(
        sender: &Resource<Sender>,
        message_ptr: *const u8,
        message_len: usize,
    ) -> MessageCopy;

    /// This is valid because `Resource<..>` is guaranteed to have `usize` representation,
    /// so the host essentially receives the index into the `externrefs` table.
    pub(crate) fn inspect_message(#[resource = false] bytes: MessageCopy);
}
// ANCHOR_END: imports_copy

#[externref::externref(stubs)]
#[link(wasm_import_module = "wasm:js-string")]
unsafe extern "C" {
    pub(crate) fn cast(value: Option<&Resource<()>>) -> Resource<()>;
}
