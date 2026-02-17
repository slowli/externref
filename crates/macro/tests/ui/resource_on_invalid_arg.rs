use externref_macro::externref;

#[externref]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    fn test_import(#[resource = false] some_ptr: *mut u8);
}

#[externref]
pub extern "C" fn test_export(#[resource] some_ptr: *const u8) {
    // Do nothing
}

fn main() {}
