use externref_macro::externref;

#[externref]
#[export_name("what")]
pub extern "C" fn bogus() {
    // Do nothing
}

#[externref]
#[export_name = 10]
pub extern "C" fn bogus_too() {
    // Do nothing
}

fn main() {}
