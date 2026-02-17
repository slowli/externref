use externref_macro::externref;

type Resource<T> = Vec<T>;

#[externref]
#[link(wasm_import_module = "test")]
unsafe extern "C" {
    fn test_import(#[resource = 5] some_ptr: &Resource<usize>);
}

#[externref]
#[resource(!)]
pub extern "C" fn test_export() -> Resource<i8> {
    // Do nothing
}

fn main() {}
