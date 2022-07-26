//! E2E test for `externref`.

//use std::cell::RefCell;

use externref::{signature::*, Resource};

pub struct Sender(());

pub struct Bytes(());

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "test")]
extern "C" {
    #[link_name = "send_message"]
    fn __externref_send_message(__arg0: usize, message_ptr: *const u8, message_len: usize)
        -> usize;

    #[link_name = "inspect_refs"]
    fn inspect_refs_on_host();
}

#[cfg(target_arch = "wasm32")]
unsafe fn send_message(
    sender: &Resource<Sender>,
    message_ptr: *const u8,
    message_len: usize,
) -> Resource<Bytes> {
    Resource::new(__externref_send_message(
        sender.as_raw(),
        message_ptr,
        message_len,
    ))
}

#[cfg(not(target_arch = "wasm32"))]
unsafe fn send_message(_: &Resource<Sender>, _: *const u8, _: usize) -> Resource<Bytes> {
    panic!("only callable from WASM")
}

externref::declare_function!(Function {
    kind: FunctionKind::Import("test"),
    name: "send_message",
    externrefs: BitSlice::builder::<1>(4)
        .with_set_bit(0)
        .with_set_bit(3)
        .build(),
});

/// Calls to the host to check the `externrefs` table.
fn inspect_refs() {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        inspect_refs_on_host();
    }
}

#[no_mangle]
#[export_name = "test_export"]
pub extern "C" fn test_export(sender: /*Resource<Sender>*/ usize) {
    let sender = unsafe { Resource::<Sender>::new(sender) };
    let messages = ["test", "42", "some other string"]
        .into_iter()
        .map(|message| {
            inspect_refs();
            unsafe { send_message(&sender, message.as_ptr(), message.len()) }
        });
    let mut messages: Vec<_> = messages.collect();
    inspect_refs();
    messages.swap_remove(0);
    inspect_refs();
    drop(messages);
    inspect_refs();
}

externref::declare_function!(Function {
    kind: FunctionKind::Export,
    name: "test_export",
    externrefs: BitSlice::builder::<1>(1).with_set_bit(0).build(),
});
