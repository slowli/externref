use externref_macro::externref;

#[externref]
pub extern "C" fn printf(format: *const c_char, _: ...) {
    // Do nothing
}

fn main() {}
