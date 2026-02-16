use externref_macro::externref;

#[externref]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    #[resource = false]
    fn test_import(some_ptr: *mut u8);
}

#[externref]
#[resource]
pub extern "C" fn test_export(some_ptr: *const u8) {
    // Do nothing
}

fn main() {}
