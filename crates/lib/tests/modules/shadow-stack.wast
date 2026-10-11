(module
    (import "externref" "insert" (func $insert (param i32) (result i32)))
    (import "externref" "drop" (func $drop (param i32)))
    (import "externref" "guard" (func $guard))
    (global $__stack_pointer (mut i32) (i32.const 32768))
    (memory 1)
    (func $test (export "test") (param $resource i32)
        (local $frame i32) (local $index i32)
        (global.set $__stack_pointer
            (local.tee $frame
                (i32.sub (global.get $__stack_pointer) (i32.const 32))))
        (i32.store offset=16 (local.get $frame) (local.get $resource))
        (i32.store offset=12 (local.get $frame)
            (i32.load offset=16 (local.get $frame)))
        (call $guard)
        (local.set $index (call $insert
            (i32.load offset=12 (local.get $frame))))
        (call $drop (local.get $index))
        (global.set $__stack_pointer (i32.add (local.get $frame) (i32.const 32)))
    )
)