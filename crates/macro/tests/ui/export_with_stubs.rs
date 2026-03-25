use externref_macro::externref;

#[externref(stubs)]
pub extern "C" fn bogus() {
    // Do nothing
}

fn main() {}
