use externref_macro::externref;

#[externref]
pub extern "C" fn printf(format: *const c_char, ...) {
    // Do nothing
}

fn main() {}
