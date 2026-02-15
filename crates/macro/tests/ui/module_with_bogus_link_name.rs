use externref_macro::externref;

#[externref]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    #[link_name("huh")]
    pub fn unused(ptr: *const u8, len: usize);
}

#[externref]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    #[link_name = 3]
    pub fn other(ptr: *const u8, len: usize);
}

fn main() {}
