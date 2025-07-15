# Quick guide: adding a new tracked function

> **File to edit:** `lhs/src/parser.rs`

---

### 1  Locate the registration point

Open `add_builtin_handlers(&mut self)` – every line in that method is a
`self.register_handler("<path>", <handler_fn>)` call.

---

### 2  Write a handler

```rust
fn handle_<name><'tcx,'mir,'ctx>(p: &mut MIRParser<'tcx,'mir,'ctx>, c: Call<'tcx>) {
    // c.args   → Vec<Operand<'tcx>>    (function arguments)
    // c.span   → Option<Span>          (call site)
    // p.curr   → SymExec<'ctx>         (symbolic state)
    // p.dangerous_spans.push(span)     (report a finding)

}
```

You can check the definition of `Call` in `lhs/src/parser.rs` for more details on the fields.
---

### 3  Register the handler

Add **one** line in `add_builtin_handlers` (or wherever you construct the
parser):

```rust
const FN_NEW: &str = "crate::module::function";
self.register_handler(FN_NEW, handle_<name>);
```

> What this essentially does is , what handler to call for which function.

---

### 4  Example – track `tokio::fs::write`

```rust
// create a new handler function in `lhs/src/parser.rs`
fn handle_tokio_write<'tcx,'mir,'ctx>(p: &mut MIRParser<'tcx,'mir,'ctx>, c: Call<'tcx>) {
    // whatever you wanna do when you see a call to `tokio::fs::write`
}

// in add_builtin_handlers()
self.register_handler("tokio::fs::write", handle_tokio_write);
```

