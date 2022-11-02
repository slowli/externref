use externref_macro::externref;

#[externref]
#[link = 5]
extern "C" {
    pub fn unused(ptr: *const u8, len: usize);
}

#[externref]
#[link(wasm_module = "what")]
extern "C" {
    pub fn unused(ptr: *const u8, len: usize);
}

#[externref]
#[link(wasm_import_module = 5)]
extern "C" {
    pub fn unused(ptr: *const u8, len: usize);
}

fn main() {}
