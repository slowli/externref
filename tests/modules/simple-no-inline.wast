(module
  ;; Corresponds to the following logic:
  ;;
  ;; ```
  ;; extern "C" {
  ;;     fn alloc(arena: &Resource<Arena>, cap: usize)
  ;;         -> Option<Resource<Bytes>>;
  ;; }
  ;;
  ;; pub extern "C" fn test(arena: &Resource<Arena>) {
  ;;     let _bytes = unsafe { alloc(arena, 42) }.unwrap();
  ;; }
  ;; ```
  ;;
  ;; Unlike with the module in `simple.wast`, we don't inline some assignments
  ;; from functions returning `externref`s in order to test that the corresponding
  ;; locals are transformed.

  ;; surrogate imports
  (import "externref" "insert" (func $insert_ref (param i32) (result i32)))
  (import "externref" "get" (func $get_ref (param i32) (result i32)))
  (import "externref" "drop" (func $drop_ref (param i32)))
  (import "externref" "guard" (func $ref_guard))
  ;; real imported fn
  (import "arena" "alloc" (func $alloc (param i32 i32) (result i32)))

  ;; exported fn
  (func (export "test") (param $arena i32)
    (local $bytes i32)
    (local.set $bytes
      (call $alloc
        (call $get_ref
          (local.tee $arena
            (call $insert_ref (local.get $arena))
          )
        )
        (i32.const 42)
      )
    )
    (if (i32.eq
      (local.tee $bytes
        (call $insert_ref (local.get $bytes))
      )
      (i32.const -1))
      (then (unreachable))
      (else (call $drop_ref (local.get $bytes)))
   )
   (call $drop_ref (local.get $arena))
  )

  ;; internal fn; the `ref` local should be transformed as well
  (func (param $index i32)
    (local $ref i32)
    (call $ref_guard)
    (local.set $ref
      (call $get_ref (local.get $index))
    )
    (call $drop_ref (local.get $ref))
  )
)
