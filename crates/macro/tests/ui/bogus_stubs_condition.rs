use externref_macro::externref;

#[externref(stubs("what"))]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    pub fn unused(ptr: *const u8, len: usize);
}

fn main() {}
