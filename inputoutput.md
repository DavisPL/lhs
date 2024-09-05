# Input-output examples

The input to each example is a Rust function. The output to each example whether or not there exists an input to the function that can cause the function to violate a certain safety property.

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

4. This function contains an input (operation: `|x| {fs::write("hello", "/proc/self/mem"); x}`)such that a write to /proc/self/mem occurs.
```Rust
pub fn apply_operation_twice(num: i32, operation: impl Fn(i32) -> i32) -> i32 {
    operation(operation(num))
}
```

5. This function contains no input such that a write to /proc/self/mem occurs.
```Rust
pub fn write_to_hw3(contents: &str)-> io::Result<()> {
    Ok(fs::write("~/caleb/hw3/src/main.rs", contents)?)
}
```