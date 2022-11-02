use externref_macro::externref;

#[externref]
pub fn test() {
    // Does nothing.
}

#[externref]
extern {
    pub fn unused(ptr: *const u8, len: usize);
}

fn main() {}
