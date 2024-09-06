pub fn apply_operation_twice(num: i32, operation: impl Fn(i32) -> i32) -> i32 {
    operation(operation(num))
}
