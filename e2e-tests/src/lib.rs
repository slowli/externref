//! E2E test for `externref`.

use externref::{externref, Resource};

pub struct Sender(());

pub struct Bytes(());

#[externref]
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "test")]
extern "C" {
    #[link_name = "send_message"]
    fn send_message(
        sender: &Resource<Sender>,
        message_ptr: *const u8,
        message_len: usize,
    ) -> Resource<Bytes>;

    #[link_name = "inspect_refs"]
    fn inspect_refs_on_host();
}

#[cfg(not(target_arch = "wasm32"))]
unsafe fn send_message(_: &Resource<Sender>, _: *const u8, _: usize) -> Resource<Bytes> {
    panic!("only callable from WASM")
}

/// Calls to the host to check the `externrefs` table.
fn inspect_refs() {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        inspect_refs_on_host();
    }
}

#[externref]
#[export_name = "test_export"]
pub extern "C" fn test_export(sender: Resource<Sender>) {
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
