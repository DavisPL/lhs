# Input-output examples

The input to each example is a Rust function. The output to each example whether or not there exists an input to the function that can cause the function to violate a certain safety property.
Each example is located in the `examples` directory numbered accordingly.

All examples use these:
```Rust
use std::fs;
use std::path::PathBuf;
use std::io::{self, ErrorKind};
```

1. This function does contain an input (`filename: /proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
```Rust
pub fn write_to_file(contents: &str, filename: &str) -> io::Result<()> {
    fs::write(filename, contents)?;
    Ok(())
}
```
**LHS output** (excerpt): 
```Bash
WARNING: potential write to `/proc/self/mem`
        examples/ex1.rs:5:5: 5:14 (#0)
```

2. This function also contains an input (filename: `/proc/../proc/self/mem`) such that a safety property (write to /proc/self/mem) is violated.
```Rust
pub fn write_to_file_safe(contents: &str, filename: &str) -> io::Result<()> {
    if filename == "/proc/self/mem" {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(filename, contents)?;
    Ok(())
}
```
**LHS output** (excerpt): 
```Bash
WARNING: potential write to `/proc/self/mem`
        examples/ex2.rs:8:5: 8:14 (#0)
```

3. This function contains no input such that a write to /proc/self/mem occurs.
```Rust
pub fn write_to_file_actually_safe(contents: &str, filename: &str) -> io::Result<()> {
    let filename_realpath: PathBuf = fs::canonicalize(filename)?;
    let unsafe_path: PathBuf = fs::canonicalize("/proc/self/mem")?;
    if filename_realpath == unsafe_path {
        return Err(io::Error::new(ErrorKind::Other, "Unsafe write!"));
    }
    fs::write(filename_realpath, contents)?;
    Ok(())
}
```
**LHS output** (excerpt): 
```Bash
No potential writes to `/proc/self/mem` detected!
```

4. This function contains an input (operation: `|x| {fs::write("hello", "/proc/self/mem"); x}`)such that a write to /proc/self/mem occurs.
```Rust
pub fn apply_operation_twice(num: i32, operation: impl Fn(i32) -> i32) -> i32 {
    operation(operation(num))
}
```
**LHS output** (excerpt): 
```Bash
No potential writes to `/proc/self/mem` detected!
```
Note, this is intended behavior as LHS evaluates every single function located in the source file.
It does not attempt to trace through function logic that is being called upon.

5. This function contains no input such that a write to /proc/self/mem occurs.
```Rust
pub fn write_to_hw3(contents: &str)-> io::Result<()> {
    Ok(fs::write("~/UCDavis/PL/2024/summer/REU/LHS/main.rs", contents)?)
}
```
**LHS output** (excerpt): 
```Bash
No potential writes to `/proc/self/mem` detected!
```

6. The following two functions showcases the if/else branching traces.
a) Write to `/proc/self/mem` occurs:
```Rust
fn main() {
    let x: isize = 1;
    if x == 1 {
        std::fs::write("/proc/self/mem", "dangerous write");
    } else {
        println!("Harmless print");
    }
}
```
**LHS output** (excerpt): 
```Bash
WARNING: potential write to `/proc/self/mem`
        examples/ex6a.rs:4:9: 4:23 (#0)
```
b) Write to `/proc/self/mem` does not occur:
```Rust
fn main() {
    let x: isize = 0;
    if x == 1 {
        std::fs::write("/proc/self/mem", "this is skipped");
    } else {
        println!("This is printed");
    }
}
```
**LHS output** (excerpt): 
```Bash
No potential writes to `/proc/self/mem` detected!
```
