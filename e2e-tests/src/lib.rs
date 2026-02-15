//! E2E test for `externref`.

#![cfg_attr(target_arch = "wasm32", no_std)]

extern crate alloc;

use alloc::vec::Vec;

use externref::{Resource, externref};
use hashbrown::HashSet;

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn handle_panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

pub struct Sender(());

pub struct Bytes(());

// Emulate reexporting the crate.
mod reexports {
    pub use externref as anyref;
}

mod imports {
    use externref::Resource;

    use crate::{Bytes, Sender};

    #[cfg(target_arch = "wasm32")]
    #[externref::externref]
    #[link(wasm_import_module = "test")]
    unsafe extern "C" {
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

    // Check `PartialEq` for messages.
    for (i, lhs) in messages.iter().enumerate() {
        for (j, rhs) in messages.iter().enumerate() {
            assert_eq!(lhs == rhs, i == j);
        }
    }

    inspect_refs();
    messages.swap_remove(0);
    inspect_refs();

    let messages: HashSet<_> = messages.into_iter().collect();
    inspect_refs();
    assert_eq!(messages.len(), 2);

    drop(messages);
    inspect_refs();
}

#[unsafe(export_name = concat!("test_export_", stringify!(with_casts)))]
// ^ tests manually specified name with a complex expression
#[externref]
pub extern "C" fn test_export_with_casts(sender: Resource<()>) {
    let sender = unsafe { sender.downcast_unchecked() };
    let messages = ["test", "42", "some other string"]
        .into_iter()
        .map(|message| {
            inspect_refs();
            unsafe { imports::send_message(&sender, message.as_ptr(), message.len()) }.upcast()
        });
    let mut messages: Vec<_> = messages.collect();
    inspect_refs();

    messages.swap_remove(0);
    inspect_refs();

    let messages: HashSet<_> = messages.into_iter().collect();
    inspect_refs();
    assert_eq!(messages.len(), 2);

    drop(messages);
    inspect_refs();
}

#[externref(crate = "crate::reexports::anyref")]
#[unsafe(no_mangle)]
pub extern "C" fn test_nulls(sender: Option<&Resource<Sender>>) {
    let message = "test";
    if let Some(sender) = sender {
        let bytes = unsafe { imports::send_message(sender, message.as_ptr(), message.len()) };
        assert_eq!(unsafe { imports::message_len(Some(&bytes)) }, 4);
    }
    assert_eq!(unsafe { imports::message_len(None) }, 0);
}

// Add another param to the function so that it's not deduped with `test_nulls`
// (incl. by `wasm-opt` over which we don't have any control).
#[externref(crate = crate::reexports::anyref)]
pub extern "C" fn test_nulls2(sender: Option<&Resource<Sender>>, _unused: u32) {
    test_nulls(sender);
}
