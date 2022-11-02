use externref_macro::externref;

#[externref]
pub extern "win64" fn test() {
    // Does nothing.
}

#[externref]
extern "win64" {
    pub fn unused(ptr: *const u8, len: usize);
}

fn main() {}
