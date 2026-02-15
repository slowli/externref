use externref_macro::externref;

#[externref]
#[unsafe(export_name("what"))]
pub extern "C" fn bogus() {
    // Do nothing
}

#[externref]
#[unsafe(export_name = 10)]
pub extern "C" fn bogus_too() {
    // Do nothing
}

fn main() {}
