use externref_macro::externref;

#[externref]
extern "C" {
    pub fn unused(ptr: *const u8, len: usize);
}

fn main() {}
