//! E2E test for `externref`.

use externref::{externref, Resource};

pub struct Sender(());

pub struct Bytes(());

mod imports {
    use externref::Resource;

    use crate::{Bytes, Sender};

    #[cfg(target_arch = "wasm32")]
    #[externref::externref]
    #[link(wasm_import_module = "test")]
    extern "C" {
        pub(crate) fn send_message(
            sender: &Resource<Sender>,
            message_ptr: *const u8,
            message_len: usize,
        ) -> Resource<Bytes>;

        pub(crate) fn message_len(bytes: Option<&Resource<Bytes>>) -> usize;

        #[link_name = "inspect_refs"]
        pub(crate) fn inspect_refs_on_host();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) unsafe fn send_message(
        _: &Resource<Sender>,
        _: *const u8,
        _: usize,
    ) -> Resource<Bytes> {
        panic!("only callable from WASM")
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) unsafe fn message_len(_: Option<&Resource<Bytes>>) -> usize {
        panic!("only callable from WASM")
    }
}

/// Calls to the host to check the `externrefs` table.
fn inspect_refs() {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        imports::inspect_refs_on_host();
    }
}

#[externref]
pub extern "C" fn test_export(sender: Resource<Sender>) {
    let messages = ["test", "42", "some other string"]
        .into_iter()
        .map(|message| {
            inspect_refs();
            unsafe { imports::send_message(&sender, message.as_ptr(), message.len()) }
        });
    let mut messages: Vec<_> = messages.collect();
    inspect_refs();
    messages.swap_remove(0);
    inspect_refs();
    drop(messages);
    inspect_refs();
}

#[externref]
pub extern "C" fn test_nulls(sender: Option<&Resource<Sender>>) {
    let message = "test";
    if let Some(sender) = sender {
        let bytes = unsafe { imports::send_message(sender, message.as_ptr(), message.len()) };
        assert_eq!(unsafe { imports::message_len(Some(&bytes)) }, 4);
    }
    assert_eq!(unsafe { imports::message_len(None) }, 0);
}
