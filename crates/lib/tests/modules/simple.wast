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

  ;; surrogate imports
  (import "externref" "insert" (func $insert_ref (param i32) (result i32)))
  (import "externref" "get" (func $get_ref (param i32) (result i32)))
  (import "externref" "drop" (func $drop_ref (param i32)))
  ;; real imported fn
  (import "arena" "alloc" (func $alloc (param i32 i32) (result i32)))

  ;; exported fn
  (func (export "test") (param $arena i32)
    (local $bytes i32)
    (if (i32.eq
      (local.tee $bytes
        (call $insert_ref
          (call $alloc
            (call $get_ref
              ;; Reassigning the param local is completely valid,
              ;; and the Rust compliler frequently does this.
              (local.tee $arena
                (call $insert_ref (local.get $arena))
              )
            )
            (i32.const 42)
          )
        )
      )
      (i32.const -1))
      (then (unreachable))
      (else (call $drop_ref (local.get $bytes)))
   )
   (call $drop_ref (local.get $arena))
  )
)
